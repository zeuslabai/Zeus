//! Learning Engine - Active learning from interactions and outcomes
//!
//! Zeus learns from every interaction to continuously improve:
//! - Extracts lessons from successes and failures
//! - Identifies patterns in user behavior
//! - Generalizes knowledge across domains
//! - Updates confidence in learned knowledge
//!
//! ## Mnemosyne Integration
//!
//! When a `Mnemosyne` instance is provided, the learning engine persists
//! lessons as `Semantic` memories and intents as `Episodic` memories.
//! On startup, existing lessons are loaded from Mnemosyne.
//! On recall, Mnemosyne is queried for relevant learned patterns.

use crate::intent::Intent;
use crate::reasoning::ThoughtChain;
use chrono::Timelike;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};
use zeus_core::{CompactionHint, Message, Result, Role, TextDirection};
use zeus_mnemosyne::{MemoryType, Mnemosyne};

/// Session ID used for all learning-engine memories in Mnemosyne.
const LEARNING_SESSION_ID: &str = "nous-learning";

/// Outcome of an action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    /// ID of the intent that led to this outcome
    pub intent_id: String,
    /// Whether the action succeeded
    pub success: bool,
    /// Feedback (user or system)
    pub feedback: String,
    /// When this outcome occurred
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// A lesson learned from an experience
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lesson {
    /// Unique identifier
    pub id: String,
    /// What was learned
    pub insight: String,
    /// Category of lesson
    pub category: LessonCategory,
    /// Conditions under which this lesson applies
    pub conditions: Vec<String>,
    /// Recommended action based on this lesson
    pub recommendation: Option<String>,
    /// Confidence in this lesson
    pub confidence: f32,
    /// Number of times this lesson has been reinforced
    pub reinforcements: u32,
    /// When first learned
    pub learned_at: chrono::DateTime<chrono::Utc>,
    /// When last reinforced
    pub last_reinforced: chrono::DateTime<chrono::Utc>,
}

impl Lesson {
    /// Create a new lesson
    pub fn new(insight: &str, category: LessonCategory) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: ulid::Ulid::new().to_string(),
            insight: insight.to_string(),
            category,
            conditions: Vec::new(),
            recommendation: None,
            confidence: 0.5,
            reinforcements: 1,
            learned_at: now,
            last_reinforced: now,
        }
    }

    /// Reinforce this lesson (increase confidence)
    pub fn reinforce(&mut self) {
        self.reinforcements += 1;
        self.last_reinforced = chrono::Utc::now();
        // Confidence increases with reinforcement, capped at 0.95
        self.confidence = (self.confidence + 0.1).min(0.95);
    }

    /// Weaken this lesson (decrease confidence)
    pub fn weaken(&mut self) {
        self.confidence = (self.confidence - 0.15).max(0.1);
    }

    /// Check if this lesson is stale (hasn't been reinforced recently)
    pub fn is_stale(&self, days: i64) -> bool {
        let age = chrono::Utc::now() - self.last_reinforced;
        age.num_days() > days
    }

    /// Marker prefix for Mnemosyne FTS5 searchability
    const MEMORY_PREFIX: &'static str = "nous_lesson ";

    /// Serialize this lesson to a JSON string for Mnemosyne storage.
    /// Includes a prefix so FTS5 can find it.
    fn to_memory_content(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_else(|_| self.insight.clone());
        format!("{}{}", Self::MEMORY_PREFIX, json)
    }

    /// Deserialize a lesson from Mnemosyne memory content.
    /// Strips the marker prefix before parsing.
    fn from_memory_content(content: &str) -> Option<Self> {
        let json = content.strip_prefix(Self::MEMORY_PREFIX).unwrap_or(content);
        serde_json::from_str(json).ok()
    }
}

/// Categories of lessons
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LessonCategory {
    /// User preference learned
    UserPreference,
    /// Successful strategy
    SuccessfulStrategy,
    /// Failed approach to avoid
    FailedApproach,
    /// Domain knowledge
    DomainKnowledge,
    /// Tool usage insight
    ToolUsage,
    /// Communication pattern
    Communication,
    /// Timing/scheduling insight
    Timing,
    /// Error recovery
    ErrorRecovery,
}

/// The Learning Engine
pub struct LearningEngine {
    /// Stored lessons (in-memory cache)
    lessons: tokio::sync::RwLock<Vec<Lesson>>,
    /// Intent history for pattern detection
    intent_history: tokio::sync::RwLock<Vec<IntentRecord>>,
    /// Outcome history for learning
    outcome_history: tokio::sync::RwLock<Vec<Outcome>>,
    /// Optional Mnemosyne backend for persistence
    mnemosyne: Option<Arc<Mnemosyne>>,
}

/// Record of an intent for history
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IntentRecord {
    id: String,
    intent_type: String,
    raw_input: String,
    timestamp: chrono::DateTime<chrono::Utc>,
    outcome: Option<bool>,
}

impl LearningEngine {
    /// Create a new learning engine (in-memory only, no persistence)
    pub async fn new() -> Result<Self> {
        Ok(Self {
            lessons: tokio::sync::RwLock::new(Vec::new()),
            intent_history: tokio::sync::RwLock::new(Vec::new()),
            outcome_history: tokio::sync::RwLock::new(Vec::new()),
            mnemosyne: None,
        })
    }

    /// Create a learning engine backed by Mnemosyne for persistence.
    ///
    /// On creation, loads existing lessons from the Semantic memory store.
    pub async fn with_mnemosyne(mnemosyne: Arc<Mnemosyne>) -> Result<Self> {
        let lessons = Self::load_lessons_from_store(&mnemosyne).await;
        debug!(count = lessons.len(), "Loaded lessons from Mnemosyne");

        Ok(Self {
            lessons: tokio::sync::RwLock::new(lessons),
            intent_history: tokio::sync::RwLock::new(Vec::new()),
            outcome_history: tokio::sync::RwLock::new(Vec::new()),
            mnemosyne: Some(mnemosyne),
        })
    }

    /// Load persisted lessons from Mnemosyne Semantic memory.
    async fn load_lessons_from_store(mnemosyne: &Mnemosyne) -> Vec<Lesson> {
        // Search for all lesson entries stored as Semantic memories
        // Uses the "nous_lesson" marker prefix for FTS5 matching
        match mnemosyne
            .search_by_type("nous_lesson", MemoryType::Semantic, 500)
            .await
        {
            Ok(results) => {
                // Deduplicate by lesson ID, keeping the most recently persisted
                // entry (highest SQLite rowid = r.id). Each persist call inserts
                // a new row, so the same lesson may appear multiple times after
                // updates. `last_reinforced` is not a reliable tiebreaker because
                // decay updates preserve the original timestamp.
                let mut by_id: HashMap<String, (i64, Lesson)> = HashMap::new();
                for r in &results {
                    if let Some(lesson) = Lesson::from_memory_content(&r.content) {
                        let entry = by_id
                            .entry(lesson.id.clone())
                            .or_insert_with(|| (r.id, lesson.clone()));
                        if r.id > entry.0 {
                            *entry = (r.id, lesson);
                        }
                    }
                }
                by_id.into_values().map(|(_, l)| l).collect()
            }
            Err(e) => {
                warn!(error = %e, "Failed to load lessons from Mnemosyne");
                Vec::new()
            }
        }
    }

    /// Persist a lesson to Mnemosyne as a Semantic memory.
    async fn persist_lesson(&self, lesson: &Lesson) {
        if let Some(ref mnemosyne) = self.mnemosyne {
            let msg = Message {
                role: Role::System,
                content: lesson.to_memory_content(),
                tool_calls: Vec::new(),
                tool_results: Vec::new(),
                timestamp: lesson.last_reinforced,
                attachments: Vec::new(),
                message_id: Some(lesson.id.clone()),
                parent_id: None,
                thread_id: None,
                direction: TextDirection::Ltr, channel_source: None,
                compaction_hint: CompactionHint::default(),
            };

            if let Err(e) = mnemosyne
                .store_typed(
                    LEARNING_SESSION_ID,
                    &msg,
                    MemoryType::Semantic,
                    lesson.confidence,
                )
                .await
            {
                warn!(error = %e, lesson_id = %lesson.id, "Failed to persist lesson to Mnemosyne");
            } else {
                debug!(lesson_id = %lesson.id, "Persisted lesson to Mnemosyne");
            }
        }
    }

    /// Persist an intent record to Mnemosyne as an Episodic memory.
    async fn persist_intent(&self, record: &IntentRecord) {
        if let Some(ref mnemosyne) = self.mnemosyne {
            let content =
                serde_json::to_string(record).unwrap_or_else(|_| record.raw_input.clone());
            let msg = Message {
                role: Role::User,
                content,
                tool_calls: Vec::new(),
                tool_results: Vec::new(),
                timestamp: record.timestamp,
                attachments: Vec::new(),
                message_id: Some(record.id.clone()),
                parent_id: None,
                thread_id: None,
                direction: TextDirection::Ltr, channel_source: None,
                compaction_hint: CompactionHint::default(),
            };

            if let Err(e) = mnemosyne
                .store_typed(LEARNING_SESSION_ID, &msg, MemoryType::Episodic, 0.3)
                .await
            {
                warn!(error = %e, "Failed to persist intent to Mnemosyne");
            }
        }
    }

    /// Record an intent for future learning
    pub async fn record_intent(&self, intent: &Intent) -> Result<()> {
        let record = IntentRecord {
            id: intent.id.clone(),
            intent_type: intent.action().to_string(),
            raw_input: intent.raw_input.clone(),
            timestamp: intent.timestamp,
            outcome: None,
        };

        // Persist to Mnemosyne (fire-and-forget, non-blocking for the caller)
        self.persist_intent(&record).await;

        let mut history = self.intent_history.write().await;
        history.push(record);

        // Keep last 1000 intents
        if history.len() > 1000 {
            history.remove(0);
        }

        Ok(())
    }

    /// Record reasoning for future learning
    pub async fn record_reasoning(&self, chain: &ThoughtChain) -> Result<()> {
        // Extract insights from reasoning
        if chain.success && !chain.steps.is_empty() {
            let insight = format!(
                "Successful reasoning approach for '{}': {} steps",
                chain.problem,
                chain.steps.len()
            );

            let mut lesson = Lesson::new(&insight, LessonCategory::SuccessfulStrategy);
            lesson
                .conditions
                .push(format!("Problem type: {}", chain.problem));

            self.persist_lesson(&lesson).await;

            let mut lessons = self.lessons.write().await;
            lessons.push(lesson);
        }

        Ok(())
    }

    /// Extract a lesson from an outcome
    pub async fn extract_lesson(&self, outcome: &Outcome) -> Result<Lesson> {
        // Record the outcome
        let mut outcomes = self.outcome_history.write().await;
        outcomes.push(outcome.clone());
        // Keep last 1000 outcomes (mirrors intent_history cap — prevents unbounded growth)
        if outcomes.len() > 1000 {
            outcomes.remove(0);
        }

        // Update intent history with outcome
        let mut intents = self.intent_history.write().await;
        if let Some(record) = intents.iter_mut().find(|r| r.id == outcome.intent_id) {
            record.outcome = Some(outcome.success);
        }

        // Extract lesson
        let category = if outcome.success {
            LessonCategory::SuccessfulStrategy
        } else {
            LessonCategory::FailedApproach
        };

        let insight = if outcome.success {
            format!("Approach succeeded: {}", outcome.feedback)
        } else {
            format!("Approach failed: {} - avoid in future", outcome.feedback)
        };

        let lesson = Lesson::new(&insight, category);

        // Check if we have a similar lesson
        let mut lessons = self.lessons.write().await;
        if let Some(existing) = lessons
            .iter_mut()
            .find(|l| self.lessons_similar(l, &lesson))
        {
            if outcome.success {
                existing.reinforce();
            } else {
                existing.weaken();
            }
            let updated = existing.clone();
            // Persist the reinforced/weakened lesson
            drop(lessons);
            self.persist_lesson(&updated).await;
            return Ok(updated);
        }

        lessons.push(lesson.clone());

        // Persist the new lesson
        drop(outcomes);
        drop(intents);
        drop(lessons);
        self.persist_lesson(&lesson).await;

        // Analyze patterns
        self.analyze_patterns().await?;

        Ok(lesson)
    }

    /// Check if two lessons are similar
    fn lessons_similar(&self, a: &Lesson, b: &Lesson) -> bool {
        a.category == b.category && {
            // Simple word overlap check
            let a_words: std::collections::HashSet<_> = a.insight.split_whitespace().collect();
            let b_words: std::collections::HashSet<_> = b.insight.split_whitespace().collect();
            let overlap = a_words.intersection(&b_words).count();
            overlap >= 3
        }
    }

    /// Analyze patterns in history
    async fn analyze_patterns(&self) -> Result<()> {
        let intents = self.intent_history.read().await;

        // Look for repeated intent patterns
        let mut pattern_counts: HashMap<String, u32> = HashMap::new();
        for record in intents.iter() {
            *pattern_counts
                .entry(record.intent_type.clone())
                .or_insert(0) += 1;
        }

        // Create lessons for common patterns
        let mut lessons = self.lessons.write().await;
        let mut new_lessons = Vec::new();
        for (intent_type, count) in pattern_counts.iter() {
            if *count >= 5 {
                let insight = format!("User frequently uses '{}' ({} times)", intent_type, count);
                if !lessons.iter().any(|l| l.insight.contains(&insight)) {
                    let lesson = Lesson::new(&insight, LessonCategory::UserPreference);
                    new_lessons.push(lesson.clone());
                    lessons.push(lesson);
                }
            }
        }

        // Look for time-based patterns
        let temporal_lessons = self.analyze_temporal_patterns(&intents, &mut lessons);

        drop(lessons);

        // Persist newly created pattern lessons
        for lesson in new_lessons.iter().chain(temporal_lessons.iter()) {
            self.persist_lesson(lesson).await;
        }

        Ok(())
    }

    /// Analyze temporal patterns. Returns newly created lessons.
    fn analyze_temporal_patterns(
        &self,
        intents: &[IntentRecord],
        lessons: &mut Vec<Lesson>,
    ) -> Vec<Lesson> {
        let mut new_lessons = Vec::new();

        // Group by hour of day
        let mut hour_counts: HashMap<u32, u32> = HashMap::new();
        for record in intents.iter() {
            let hour = record.timestamp.hour();
            *hour_counts.entry(hour).or_insert(0) += 1;
        }

        // Find peak hours
        let total: u32 = hour_counts.values().sum();
        if total > 20 {
            for (hour, count) in hour_counts.iter() {
                let percentage = (*count as f32 / total as f32) * 100.0;
                if percentage > 20.0 {
                    let time_name = match *hour {
                        6..=11 => "morning",
                        12..=13 => "lunchtime",
                        14..=17 => "afternoon",
                        18..=21 => "evening",
                        _ => "night",
                    };

                    let insight = format!(
                        "User is most active in the {} ({:.0}% of interactions)",
                        time_name, percentage
                    );

                    if !lessons.iter().any(|l| l.insight.contains(time_name)) {
                        let mut lesson = Lesson::new(&insight, LessonCategory::Timing);
                        lesson.confidence = 0.7;
                        new_lessons.push(lesson.clone());
                        lessons.push(lesson);
                    }
                }
            }
        }

        new_lessons
    }

    /// Get relevant lessons for a context.
    ///
    /// Combines in-memory lessons with Mnemosyne search results.
    pub async fn get_relevant_lessons(&self, context: &str) -> Vec<Lesson> {
        let lessons = self.lessons.read().await;

        let mut result: Vec<Lesson> = lessons
            .iter()
            .filter(|l| {
                // Check if lesson conditions match context
                l.conditions
                    .iter()
                    .any(|c| context.to_lowercase().contains(&c.to_lowercase()))
                    || context.to_lowercase().contains(
                        l.insight
                            .to_lowercase()
                            .split_whitespace()
                            .next()
                            .unwrap_or(""),
                    )
            })
            .filter(|l| l.confidence > 0.4) // Only confident lessons
            .cloned()
            .collect();

        // Also search Mnemosyne for persisted lessons not yet in memory
        if let Some(ref mnemosyne) = self.mnemosyne
            && let Ok(search_results) = mnemosyne
                .search_by_type(context, MemoryType::Semantic, 10)
                .await
        {
            let in_memory_ids: std::collections::HashSet<_> =
                result.iter().map(|l| l.id.clone()).collect();

            for sr in &search_results {
                if let Some(lesson) = Lesson::from_memory_content(&sr.content)
                    && lesson.confidence > 0.4
                    && !in_memory_ids.contains(&lesson.id)
                {
                    result.push(lesson);
                }
            }
        }

        result
    }

    /// Get all lessons
    pub async fn all_lessons(&self) -> Vec<Lesson> {
        self.lessons.read().await.clone()
    }

    /// Update an existing lesson in-place (by id) and persist to Mnemosyne.
    ///
    /// Replaces the in-memory entry matching `lesson.id`, then persists the
    /// updated lesson. If no matching entry is found in the cache the
    /// persistence call is still made (handles lessons sourced from
    /// Mnemosyne search results that were never cached locally).
    pub async fn update_lesson(&self, lesson: &Lesson) {
        let mut lessons = self.lessons.write().await;
        if let Some(slot) = lessons.iter_mut().find(|l| l.id == lesson.id) {
            *slot = lesson.clone();
        }
        drop(lessons);
        self.persist_lesson(lesson).await;
    }

    /// Prune stale lessons
    pub async fn prune_stale(&self, days: i64) {
        let mut lessons = self.lessons.write().await;
        lessons.retain(|l| !l.is_stale(days) || l.confidence > 0.8);
    }

    /// Get learning statistics
    pub async fn stats(&self) -> LearningStats {
        let lessons = self.lessons.read().await;
        let intents = self.intent_history.read().await;
        let outcomes = self.outcome_history.read().await;

        let successful = outcomes.iter().filter(|o| o.success).count();
        let total_outcomes = outcomes.len();

        LearningStats {
            total_lessons: lessons.len(),
            total_intents: intents.len(),
            total_outcomes,
            success_rate: if total_outcomes > 0 {
                successful as f32 / total_outcomes as f32
            } else {
                0.0
            },
            avg_lesson_confidence: if lessons.is_empty() {
                0.0
            } else {
                lessons.iter().map(|l| l.confidence).sum::<f32>() / lessons.len() as f32
            },
            lessons_by_category: lessons.iter().fold(HashMap::new(), |mut map, l| {
                *map.entry(format!("{:?}", l.category)).or_insert(0) += 1;
                map
            }),
        }
    }

    /// Check if Mnemosyne persistence is enabled
    pub fn has_persistence(&self) -> bool {
        self.mnemosyne.is_some()
    }
}

/// Learning statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningStats {
    pub total_lessons: usize,
    pub total_intents: usize,
    pub total_outcomes: usize,
    pub success_rate: f32,
    pub avg_lesson_confidence: f32,
    pub lessons_by_category: HashMap<String, u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lesson_creation() {
        let lesson = Lesson::new("Test insight", LessonCategory::UserPreference);
        assert_eq!(lesson.confidence, 0.5);
        assert_eq!(lesson.reinforcements, 1);
    }

    #[test]
    fn test_lesson_reinforcement() {
        let mut lesson = Lesson::new("Test", LessonCategory::SuccessfulStrategy);
        let initial_confidence = lesson.confidence;

        lesson.reinforce();
        assert!(lesson.confidence > initial_confidence);
        assert_eq!(lesson.reinforcements, 2);
    }

    #[test]
    fn test_lesson_weakening() {
        let mut lesson = Lesson::new("Test", LessonCategory::FailedApproach);
        lesson.confidence = 0.8;

        lesson.weaken();
        assert!(lesson.confidence < 0.8);
    }

    #[tokio::test]
    async fn test_learning_engine() {
        let engine = LearningEngine::new()
            .await
            .expect("LearningEngine::new should succeed");

        let outcome = Outcome {
            intent_id: "test-123".to_string(),
            success: true,
            feedback: "Worked perfectly".to_string(),
            timestamp: chrono::Utc::now(),
        };

        let lesson = engine
            .extract_lesson(&outcome)
            .await
            .expect("async operation should succeed");
        assert_eq!(lesson.category, LessonCategory::SuccessfulStrategy);
    }

    #[test]
    fn test_lesson_max_confidence() {
        let mut lesson = Lesson::new("Capped confidence", LessonCategory::SuccessfulStrategy);
        // Reinforce many times
        for _ in 0..20 {
            lesson.reinforce();
        }
        // Confidence should be capped at 0.95
        assert!((lesson.confidence - 0.95).abs() < f32::EPSILON);
        assert_eq!(lesson.reinforcements, 21); // 1 initial + 20
    }

    #[test]
    fn test_lesson_min_confidence() {
        let mut lesson = Lesson::new("Floored confidence", LessonCategory::FailedApproach);
        // Weaken many times
        for _ in 0..20 {
            lesson.weaken();
        }
        // Confidence should be floored at 0.1
        assert!((lesson.confidence - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn test_lesson_fields() {
        let mut lesson = Lesson::new("All fields test", LessonCategory::DomainKnowledge);
        lesson.conditions.push("when debugging".to_string());
        lesson.recommendation = Some("Use trace logging".to_string());

        assert!(!lesson.id.is_empty());
        assert_eq!(lesson.insight, "All fields test");
        assert_eq!(lesson.category, LessonCategory::DomainKnowledge);
        assert_eq!(lesson.conditions.len(), 1);
        assert_eq!(lesson.recommendation.as_deref(), Some("Use trace logging"));
        assert_eq!(lesson.confidence, 0.5);
        assert_eq!(lesson.reinforcements, 1);
    }

    #[test]
    fn test_lesson_reinforcement_count() {
        let mut lesson = Lesson::new("Count test", LessonCategory::ToolUsage);
        assert_eq!(lesson.reinforcements, 1);

        lesson.reinforce();
        assert_eq!(lesson.reinforcements, 2);

        lesson.reinforce();
        lesson.reinforce();
        assert_eq!(lesson.reinforcements, 4);
    }

    #[test]
    fn test_lesson_multiple_weakening() {
        let mut lesson = Lesson::new("Weaken 5 times", LessonCategory::FailedApproach);
        lesson.confidence = 0.9;

        for _ in 0..5 {
            lesson.weaken();
        }
        // 0.9 - 5*0.15 = 0.9 - 0.75 = 0.15, but clamped at each step
        // Step by step: 0.75, 0.60, 0.45, 0.30, 0.15
        assert!((lesson.confidence - 0.15).abs() < f32::EPSILON);
    }

    #[test]
    fn test_lesson_creation_from_different_categories() {
        let categories = vec![
            LessonCategory::UserPreference,
            LessonCategory::SuccessfulStrategy,
            LessonCategory::FailedApproach,
            LessonCategory::DomainKnowledge,
            LessonCategory::ToolUsage,
            LessonCategory::Communication,
            LessonCategory::Timing,
            LessonCategory::ErrorRecovery,
        ];

        for cat in categories {
            let lesson = Lesson::new("test", cat.clone());
            assert_eq!(lesson.category, cat);
            assert_eq!(lesson.confidence, 0.5);
        }
    }

    #[test]
    fn test_lesson_serialization() {
        let mut lesson = Lesson::new("Serde test", LessonCategory::Communication);
        lesson.conditions.push("in meetings".to_string());
        lesson.recommendation = Some("Be concise".to_string());
        lesson.reinforce();

        let json = serde_json::to_string(&lesson).expect("should serialize to JSON");
        let deserialized: Lesson = serde_json::from_str(&json).expect("should parse successfully");

        assert_eq!(deserialized.insight, "Serde test");
        assert_eq!(deserialized.category, LessonCategory::Communication);
        assert_eq!(deserialized.conditions.len(), 1);
        assert_eq!(deserialized.recommendation.as_deref(), Some("Be concise"));
        assert_eq!(deserialized.reinforcements, 2);
        assert!(deserialized.confidence > 0.5);
    }

    #[tokio::test]
    async fn test_learning_engine_empty_lessons() {
        let engine = LearningEngine::new()
            .await
            .expect("LearningEngine::new should succeed");

        let lessons = engine.all_lessons().await;
        assert!(lessons.is_empty());

        let relevant = engine.get_relevant_lessons("any context").await;
        assert!(relevant.is_empty());

        let stats = engine.stats().await;
        assert_eq!(stats.total_lessons, 0);
        assert_eq!(stats.total_intents, 0);
        assert_eq!(stats.total_outcomes, 0);
        assert_eq!(stats.success_rate, 0.0);
        assert_eq!(stats.avg_lesson_confidence, 0.0);
    }

    #[test]
    fn test_lesson_memory_content_roundtrip() {
        let mut lesson = Lesson::new("Roundtrip test", LessonCategory::ToolUsage);
        lesson.conditions.push("when using shell".to_string());
        lesson.recommendation = Some("Prefer edit_file over shell sed".to_string());
        lesson.reinforce();
        lesson.reinforce();

        let content = lesson.to_memory_content();
        assert!(content.starts_with("nous_lesson "));

        let restored = Lesson::from_memory_content(&content).expect("should parse");

        assert_eq!(restored.id, lesson.id);
        assert_eq!(restored.insight, lesson.insight);
        assert_eq!(restored.category, lesson.category);
        assert_eq!(restored.conditions, lesson.conditions);
        assert_eq!(restored.recommendation, lesson.recommendation);
        assert_eq!(restored.confidence, lesson.confidence);
        assert_eq!(restored.reinforcements, lesson.reinforcements);
    }

    #[test]
    fn test_lesson_from_memory_content_invalid() {
        assert!(Lesson::from_memory_content("not json").is_none());
        assert!(Lesson::from_memory_content("{}").is_none());
        assert!(Lesson::from_memory_content("").is_none());
        // Direct JSON without prefix should also work
        let lesson = Lesson::new("Direct JSON", LessonCategory::DomainKnowledge);
        let json = serde_json::to_string(&lesson).unwrap();
        let parsed = Lesson::from_memory_content(&json).expect("direct JSON should parse");
        assert_eq!(parsed.insight, "Direct JSON");
    }

    #[tokio::test]
    async fn test_update_lesson_persisted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = zeus_mnemosyne::MnemosyneConfig {
            db_path: dir.path().join("update_lesson.db"),
            ..Default::default()
        };
        let mnemosyne = Arc::new(Mnemosyne::new(config).await.expect("mnemosyne"));
        let engine = LearningEngine::with_mnemosyne(mnemosyne).await.expect("engine");

        let outcome = Outcome {
            intent_id: "update-1".to_string(),
            success: true,
            feedback: "Build pipeline succeeded with optimization".to_string(),
            timestamp: chrono::Utc::now(),
        };
        let lesson = engine.extract_lesson(&outcome).await.expect("extract");
        assert!((lesson.confidence - 0.5).abs() < f32::EPSILON);

        // Bump confidence via update_lesson
        let mut updated = lesson.clone();
        updated.confidence = 0.9;
        engine.update_lesson(&updated).await;

        // In-memory reflects the change
        let lessons = engine.all_lessons().await;
        assert_eq!(lessons.len(), 1);
        assert!((lessons[0].confidence - 0.9).abs() < f32::EPSILON);

        // Reload from Mnemosyne — updated confidence must survive restart
        let config2 = zeus_mnemosyne::MnemosyneConfig {
            db_path: dir.path().join("update_lesson.db"),
            ..Default::default()
        };
        let mnemosyne2 = Arc::new(Mnemosyne::new(config2).await.expect("mnemosyne2"));
        let engine2 = LearningEngine::with_mnemosyne(mnemosyne2).await.expect("engine2");

        let reloaded = engine2.all_lessons().await;
        assert_eq!(reloaded.len(), 1, "deduplication: exactly one entry per id");
        assert!(
            (reloaded[0].confidence - 0.9).abs() < f32::EPSILON,
            "updated confidence must survive reload; got {}",
            reloaded[0].confidence
        );
    }

    #[test]
    fn test_has_persistence_without_mnemosyne() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let engine = rt.block_on(LearningEngine::new()).unwrap();
        assert!(!engine.has_persistence());
    }

    #[tokio::test]
    async fn test_with_mnemosyne_persistence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = zeus_mnemosyne::MnemosyneConfig {
            db_path: dir.path().join("test_learning.db"),
            ..Default::default()
        };
        let mnemosyne = Arc::new(Mnemosyne::new(config).await.expect("mnemosyne init"));

        let engine = LearningEngine::with_mnemosyne(mnemosyne.clone())
            .await
            .expect("engine init");
        assert!(engine.has_persistence());

        // Extract a lesson — should persist to Mnemosyne
        let outcome = Outcome {
            intent_id: "persist-test".to_string(),
            success: true,
            feedback: "Shell command succeeded".to_string(),
            timestamp: chrono::Utc::now(),
        };
        let lesson = engine.extract_lesson(&outcome).await.expect("extract");
        assert_eq!(lesson.category, LessonCategory::SuccessfulStrategy);

        // Verify it was persisted by creating a new engine from same DB
        let config2 = zeus_mnemosyne::MnemosyneConfig {
            db_path: dir.path().join("test_learning.db"),
            ..Default::default()
        };
        let mnemosyne2 = Arc::new(Mnemosyne::new(config2).await.expect("mnemosyne2 init"));
        let engine2 = LearningEngine::with_mnemosyne(mnemosyne2)
            .await
            .expect("engine2 init");

        let loaded = engine2.all_lessons().await;
        assert!(
            !loaded.is_empty(),
            "lessons should be loaded from Mnemosyne"
        );
        assert_eq!(loaded[0].insight, lesson.insight);
    }

    #[tokio::test]
    async fn test_persist_and_recall_multiple_lessons() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = zeus_mnemosyne::MnemosyneConfig {
            db_path: dir.path().join("multi.db"),
            ..Default::default()
        };
        let mnemosyne = Arc::new(Mnemosyne::new(config).await.expect("init"));
        let engine = LearningEngine::with_mnemosyne(mnemosyne.clone())
            .await
            .expect("engine");

        // Create several lessons with distinct enough feedback to avoid similarity merging.
        // Note: failed lessons always share "Approach failed: ... - avoid in future"
        // which triggers the 3-word overlap similarity check, so we use at most one failure.
        let feedbacks = [
            (true, "Database migration completed flawlessly"),
            (false, "API rate limiter triggered unexpectedly"),
            (true, "Caching layer dramatically improved latency"),
            (true, "WebSocket reconnection logic handled gracefully"),
            (true, "Background scheduler dispatched tasks correctly"),
        ];

        for (i, (success, feedback)) in feedbacks.iter().enumerate() {
            let outcome = Outcome {
                intent_id: format!("multi-{}", i),
                success: *success,
                feedback: feedback.to_string(),
                timestamp: chrono::Utc::now(),
            };
            engine.extract_lesson(&outcome).await.expect("extract");
        }

        let stats = engine.stats().await;
        assert_eq!(stats.total_lessons, 5);
        assert_eq!(stats.total_outcomes, 5);

        // Reload from Mnemosyne
        let config2 = zeus_mnemosyne::MnemosyneConfig {
            db_path: dir.path().join("multi.db"),
            ..Default::default()
        };
        let mnemosyne2 = Arc::new(Mnemosyne::new(config2).await.expect("init2"));
        let engine2 = LearningEngine::with_mnemosyne(mnemosyne2)
            .await
            .expect("engine2");

        let loaded = engine2.all_lessons().await;
        assert_eq!(loaded.len(), 5, "all 5 lessons should be loaded");
    }

    #[tokio::test]
    async fn test_reinforcement_persisted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = zeus_mnemosyne::MnemosyneConfig {
            db_path: dir.path().join("reinforce.db"),
            ..Default::default()
        };
        let mnemosyne = Arc::new(Mnemosyne::new(config).await.expect("init"));
        let engine = LearningEngine::with_mnemosyne(mnemosyne.clone())
            .await
            .expect("engine");

        // Create a lesson
        let outcome1 = Outcome {
            intent_id: "reinforce-1".to_string(),
            success: true,
            feedback: "Shell command approach succeeded nicely here".to_string(),
            timestamp: chrono::Utc::now(),
        };
        let lesson1 = engine.extract_lesson(&outcome1).await.expect("extract1");
        assert_eq!(lesson1.reinforcements, 1);

        // Create a similar lesson to trigger reinforcement
        let outcome2 = Outcome {
            intent_id: "reinforce-2".to_string(),
            success: true,
            feedback: "Shell command approach succeeded again here".to_string(),
            timestamp: chrono::Utc::now(),
        };
        let lesson2 = engine.extract_lesson(&outcome2).await.expect("extract2");
        // Similar lesson should have been reinforced
        assert_eq!(lesson2.reinforcements, 2);
        assert!(lesson2.confidence > 0.5);
    }
}
