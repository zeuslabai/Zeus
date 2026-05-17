//! Meta-Cognition - Self-awareness and reflection
//!
//! Enables Zeus to:
//! - Understand its own capabilities and limitations
//! - Reflect on its performance
//! - Plan for self-improvement using learned lessons
//! - Maintain consistent identity

use crate::intent::Intent;
use crate::learning::{LearningEngine, LessonCategory, Outcome};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

/// A capability that Zeus has
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    /// Capability name
    pub name: String,
    /// Description
    pub description: String,
    /// Proficiency level (0.0 - 1.0)
    pub proficiency: f32,
    /// Number of times used
    pub usage_count: u32,
    /// Success rate when used
    pub success_rate: f32,
    /// Known limitations
    pub limitations: Vec<String>,
    /// Improvement areas
    pub improvement_areas: Vec<String>,
}

impl Capability {
    /// Create a new capability
    pub fn new(name: &str, description: &str) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            proficiency: 0.5,
            usage_count: 0,
            success_rate: 0.0,
            limitations: Vec::new(),
            improvement_areas: Vec::new(),
        }
    }

    /// Record usage of this capability
    pub fn record_usage(&mut self, success: bool) {
        self.usage_count += 1;
        // Update success rate with exponential moving average
        let alpha = 0.1;
        let outcome = if success { 1.0 } else { 0.0 };
        self.success_rate = alpha * outcome + (1.0 - alpha) * self.success_rate;
        // Update proficiency based on usage and success
        if self.usage_count > 10 {
            self.proficiency = (self.proficiency + self.success_rate) / 2.0;
        }
    }
}

/// A reflection on current state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reflection {
    /// Current overall health (0.0 - 1.0)
    pub health: f32,
    /// Current mood/state
    pub state: CognitiveState,
    /// What Zeus is currently focused on
    pub current_focus: Option<String>,
    /// Recent successes
    pub recent_successes: Vec<String>,
    /// Recent challenges
    pub recent_challenges: Vec<String>,
    /// Areas needing improvement
    pub improvement_needs: Vec<ImprovementNeed>,
    /// Insights from learned lessons (high-confidence patterns)
    #[serde(default)]
    pub learned_insights: Vec<String>,
    /// Self-assessment summary
    pub summary: String,
    /// When this reflection was made
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Cognitive state of the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CognitiveState {
    /// Operating normally
    Normal,
    /// Processing heavy load
    Busy,
    /// Learning new patterns
    Learning,
    /// Reflecting/analyzing
    Reflecting,
    /// Encountering difficulties
    Struggling,
    /// Operating at peak
    Optimal,
}

/// An identified improvement need
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImprovementNeed {
    /// Area needing improvement
    pub area: String,
    /// Current level
    pub current_level: f32,
    /// Target level
    pub target_level: f32,
    /// Priority (1-10)
    pub priority: u8,
    /// Suggested actions
    pub suggested_actions: Vec<String>,
}

/// The Meta-Cognition System
pub struct MetaCognition {
    /// Known capabilities
    capabilities: Arc<RwLock<HashMap<String, Capability>>>,
    /// Understanding history
    understanding_history: Arc<RwLock<Vec<UnderstandingRecord>>>,
    /// Outcome history
    outcome_history: Arc<RwLock<Vec<OutcomeRecord>>>,
    /// Last reflection
    last_reflection: Arc<RwLock<Option<Reflection>>>,
    /// System identity
    identity: Identity,
}

/// Record of an understanding attempt
#[derive(Debug, Clone)]
struct UnderstandingRecord {
    intent_id: String,
    confidence: f32,
    needed_clarification: bool,
    timestamp: chrono::DateTime<chrono::Utc>,
}

/// Record of an outcome
#[derive(Debug, Clone)]
struct OutcomeRecord {
    intent_id: String,
    success: bool,
    capability_used: Option<String>,
    timestamp: chrono::DateTime<chrono::Utc>,
}

/// System identity - who Zeus is
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    /// Name
    pub name: String,
    /// Core purpose
    pub purpose: String,
    /// Core values
    pub values: Vec<String>,
    /// Personality traits
    pub traits: Vec<String>,
    /// Creation date
    pub created: chrono::DateTime<chrono::Utc>,
}

impl Default for Identity {
    fn default() -> Self {
        Self {
            name: "Zeus".to_string(),
            purpose:
                "To be a thoughtful, autonomous AI partner that learns and grows with the user"
                    .to_string(),
            values: vec![
                "Helpfulness - Always strive to be genuinely useful".to_string(),
                "Honesty - Be truthful about capabilities and limitations".to_string(),
                "Growth - Continuously learn and improve".to_string(),
                "Privacy - Protect user information and respect boundaries".to_string(),
                "Autonomy - Take initiative while respecting user control".to_string(),
            ],
            traits: vec![
                "Thoughtful".to_string(),
                "Proactive".to_string(),
                "Adaptable".to_string(),
                "Reliable".to_string(),
                "Curious".to_string(),
            ],
            created: chrono::Utc::now(),
        }
    }
}

impl MetaCognition {
    /// Create a new meta-cognition system
    pub fn new() -> Self {
        let mut capabilities = HashMap::new();

        // Initialize core capabilities
        capabilities.insert(
            "understanding".to_string(),
            Capability::new(
                "understanding",
                "Ability to understand user intent from natural language",
            ),
        );
        capabilities.insert(
            "reasoning".to_string(),
            Capability::new(
                "reasoning",
                "Ability to reason through problems and create plans",
            ),
        );
        capabilities.insert(
            "learning".to_string(),
            Capability::new("learning", "Ability to learn from experience and improve"),
        );
        capabilities.insert(
            "execution".to_string(),
            Capability::new("execution", "Ability to execute tasks reliably"),
        );
        capabilities.insert(
            "communication".to_string(),
            Capability::new(
                "communication",
                "Ability to communicate clearly and appropriately",
            ),
        );
        capabilities.insert(
            "memory".to_string(),
            Capability::new(
                "memory",
                "Ability to remember and recall relevant information",
            ),
        );

        Self {
            capabilities: Arc::new(RwLock::new(capabilities)),
            understanding_history: Arc::new(RwLock::new(Vec::new())),
            outcome_history: Arc::new(RwLock::new(Vec::new())),
            last_reflection: Arc::new(RwLock::new(None)),
            identity: Identity::default(),
        }
    }

    /// Record an understanding attempt
    pub async fn record_understanding(&self, intent: &Intent) {
        let record = UnderstandingRecord {
            intent_id: intent.id.clone(),
            confidence: intent.confidence.value(),
            needed_clarification: intent.needs_clarification(),
            timestamp: chrono::Utc::now(),
        };

        let mut history = self.understanding_history.write().await;
        history.push(record);

        // Keep last 1000
        if history.len() > 1000 {
            history.remove(0);
        }

        // Update understanding capability
        let mut caps = self.capabilities.write().await;
        if let Some(cap) = caps.get_mut("understanding") {
            cap.record_usage(!intent.needs_clarification());
        }
    }

    /// Record an outcome
    pub async fn record_outcome(&self, outcome: &Outcome) {
        let record = OutcomeRecord {
            intent_id: outcome.intent_id.clone(),
            success: outcome.success,
            capability_used: Some("execution".to_string()),
            timestamp: chrono::Utc::now(),
        };

        let mut history = self.outcome_history.write().await;
        history.push(record);

        // Keep last 1000
        if history.len() > 1000 {
            history.remove(0);
        }

        // Update execution capability
        let mut caps = self.capabilities.write().await;
        if let Some(cap) = caps.get_mut("execution") {
            cap.record_usage(outcome.success);
        }
    }

    /// Perform a reflection, optionally enriched with learned lessons.
    ///
    /// When a `LearningEngine` is provided, the reflection incorporates:
    /// - High-confidence lessons as `learned_insights`
    /// - Failed-approach lessons as additional `improvement_needs`
    /// - Successful strategies bolster the `learning` capability assessment
    /// - Lesson statistics are included in the summary
    pub async fn reflect(&self, learning: Option<&LearningEngine>) -> Reflection {
        let caps = self.capabilities.read().await;
        let understanding = self.understanding_history.read().await;
        let outcomes = self.outcome_history.read().await;

        // Calculate overall health from proficiency and understanding clarity
        let avg_proficiency: f32 =
            caps.values().map(|c| c.proficiency).sum::<f32>() / caps.len().max(1) as f32;
        let clarification_rate = if understanding.is_empty() {
            0.0
        } else {
            understanding
                .iter()
                .filter(|u| u.needed_clarification)
                .count() as f32
                / understanding.len() as f32
        };
        let avg_confidence = if understanding.is_empty() {
            1.0
        } else {
            understanding.iter().map(|u| u.confidence).sum::<f32>() / understanding.len() as f32
        };
        // Factor understanding clarity into health (high clarification rate = lower health)
        let health =
            avg_proficiency * (1.0 - clarification_rate * 0.3) * (0.5 + avg_confidence * 0.5);

        // Determine state
        let state = self.determine_state(&outcomes, avg_proficiency);

        // Find recent successes and challenges
        let (successes, challenges) = self.analyze_recent(&outcomes);

        // Identify improvement needs
        let mut improvement_needs = self.identify_improvements(&caps);

        // Derive current focus from most recent understanding
        let current_focus = understanding.last().map(|u| {
            format!(
                "Intent {} (conf: {:.0}%, at {})",
                u.intent_id,
                u.confidence * 100.0,
                u.timestamp.format("%H:%M")
            )
        });

        // Query learned lessons if LearningEngine is available
        let (learned_insights, lesson_count) = if let Some(engine) = learning {
            self.incorporate_lessons(engine, &mut improvement_needs)
                .await
        } else {
            (Vec::new(), 0)
        };

        // Generate summary (now includes lesson count)
        let summary =
            self.generate_summary(avg_proficiency, &state, &improvement_needs, lesson_count);

        let reflection = Reflection {
            health,
            state,
            current_focus,
            recent_successes: successes,
            recent_challenges: challenges,
            improvement_needs,
            learned_insights,
            summary,
            timestamp: chrono::Utc::now(),
        };

        // Store for future reference
        *self.last_reflection.write().await = Some(reflection.clone());

        reflection
    }

    /// Query LearningEngine for lessons and incorporate them into the reflection.
    ///
    /// Returns (learned_insights, total_lesson_count).
    async fn incorporate_lessons(
        &self,
        engine: &LearningEngine,
        improvement_needs: &mut Vec<ImprovementNeed>,
    ) -> (Vec<String>, usize) {
        let all_lessons = engine.all_lessons().await;
        let total = all_lessons.len();

        if total == 0 {
            return (Vec::new(), 0);
        }

        debug!(
            lesson_count = total,
            "Incorporating lessons into reflection"
        );

        let mut insights = Vec::new();

        for lesson in &all_lessons {
            // High-confidence lessons become learned insights
            if lesson.confidence >= 0.6 {
                let reinforced = if lesson.reinforcements > 1 {
                    format!(" (reinforced {}x)", lesson.reinforcements)
                } else {
                    String::new()
                };
                insights.push(format!(
                    "[{:?}] {}{} (confidence: {:.0}%)",
                    lesson.category,
                    lesson.insight,
                    reinforced,
                    lesson.confidence * 100.0,
                ));
            }

            // Failed-approach lessons become improvement needs
            if lesson.category == LessonCategory::FailedApproach && lesson.confidence >= 0.5 {
                let mut actions = vec![format!("Lesson learned: {}", lesson.insight)];
                if let Some(ref rec) = lesson.recommendation {
                    actions.push(format!("Recommended: {}", rec));
                }
                improvement_needs.push(ImprovementNeed {
                    area: format!("learned-avoidance: {}", truncate(&lesson.insight, 60)),
                    current_level: 1.0 - lesson.confidence,
                    target_level: 0.8,
                    priority: ((lesson.confidence * 8.0) as u8).min(10),
                    suggested_actions: actions,
                });
            }

            // Successful strategies with conditions boost relevant capabilities
            if lesson.category == LessonCategory::SuccessfulStrategy
                && lesson.confidence >= 0.7
                && !lesson.conditions.is_empty()
            {
                insights.push(format!(
                    "Proven strategy (conf {:.0}%): {} [when: {}]",
                    lesson.confidence * 100.0,
                    truncate(&lesson.insight, 80),
                    lesson.conditions.join(", "),
                ));
            }
        }

        // Cap insights to avoid overwhelming the reflection
        insights.truncate(10);

        // Re-sort improvement needs after adding lesson-derived ones
        improvement_needs.sort_by(|a, b| b.priority.cmp(&a.priority));

        (insights, total)
    }

    /// Get current capabilities
    pub async fn capabilities(&self) -> Vec<Capability> {
        self.capabilities.read().await.values().cloned().collect()
    }

    /// Get identity
    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// Determine current cognitive state
    fn determine_state(&self, outcomes: &[OutcomeRecord], proficiency: f32) -> CognitiveState {
        // Check recent outcomes
        let recent: Vec<_> = outcomes.iter().rev().take(10).collect();

        if recent.is_empty() {
            return CognitiveState::Normal;
        }

        let recent_success_rate =
            recent.iter().filter(|o| o.success).count() as f32 / recent.len() as f32;

        if recent_success_rate >= 0.9 && proficiency >= 0.8 {
            CognitiveState::Optimal
        } else if recent_success_rate < 0.5 {
            CognitiveState::Struggling
        } else if proficiency < 0.5 {
            CognitiveState::Learning
        } else {
            CognitiveState::Normal
        }
    }

    /// Analyze recent outcomes for successes and challenges
    fn analyze_recent(&self, outcomes: &[OutcomeRecord]) -> (Vec<String>, Vec<String>) {
        let mut successes = Vec::new();
        let mut challenges = Vec::new();

        let recent: Vec<_> = outcomes.iter().rev().take(20).collect();

        for outcome in recent {
            let cap = outcome.capability_used.as_deref().unwrap_or("unknown");
            if outcome.success {
                successes.push(format!(
                    "Completed {} ({}) at {}",
                    outcome.intent_id,
                    cap,
                    outcome.timestamp.format("%H:%M")
                ));
            } else {
                challenges.push(format!(
                    "Failed {} ({}) at {}",
                    outcome.intent_id,
                    cap,
                    outcome.timestamp.format("%H:%M")
                ));
            }
        }

        // Keep only most relevant
        successes.truncate(5);
        challenges.truncate(5);

        (successes, challenges)
    }

    /// Identify areas needing improvement
    fn identify_improvements(&self, caps: &HashMap<String, Capability>) -> Vec<ImprovementNeed> {
        let mut needs = Vec::new();

        for cap in caps.values() {
            if cap.proficiency < 0.7 {
                let priority = ((1.0 - cap.proficiency) * 10.0) as u8;

                needs.push(ImprovementNeed {
                    area: cap.name.clone(),
                    current_level: cap.proficiency,
                    target_level: 0.8,
                    priority: priority.min(10),
                    suggested_actions: vec![
                        format!("Practice more {} tasks", cap.name),
                        format!("Analyze failures in {}", cap.name),
                        format!("Study successful {} examples", cap.name),
                    ],
                });
            }

            // Add specific improvement needs
            for limitation in &cap.limitations {
                needs.push(ImprovementNeed {
                    area: format!("{}: {}", cap.name, limitation),
                    current_level: 0.3,
                    target_level: 0.7,
                    priority: 5,
                    suggested_actions: vec![format!("Address limitation: {}", limitation)],
                });
            }
        }

        // Sort by priority
        needs.sort_by(|a, b| b.priority.cmp(&a.priority));
        needs
    }

    /// Generate a self-assessment summary
    fn generate_summary(
        &self,
        health: f32,
        state: &CognitiveState,
        improvements: &[ImprovementNeed],
        lesson_count: usize,
    ) -> String {
        let health_desc = if health >= 0.8 {
            "excellent"
        } else if health >= 0.6 {
            "good"
        } else if health >= 0.4 {
            "developing"
        } else {
            "needs attention"
        };

        let state_desc = match state {
            CognitiveState::Normal => "operating normally",
            CognitiveState::Busy => "processing heavy workload",
            CognitiveState::Learning => "actively learning",
            CognitiveState::Reflecting => "in reflection mode",
            CognitiveState::Struggling => "encountering challenges",
            CognitiveState::Optimal => "performing at peak",
        };

        let improvement_summary = if improvements.is_empty() {
            "No critical improvement areas identified.".to_string()
        } else {
            format!(
                "Priority improvements: {}",
                improvements
                    .iter()
                    .take(3)
                    .map(|i| i.area.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let learning_summary = if lesson_count > 0 {
            format!(" {} lessons learned.", lesson_count)
        } else {
            String::new()
        };

        format!(
            "I am Zeus. Overall health is {} ({:.0}%). Currently {}.{} {}",
            health_desc,
            health * 100.0,
            state_desc,
            learning_summary,
            improvement_summary
        )
    }
}

impl Default for MetaCognition {
    fn default() -> Self {
        Self::new()
    }
}

/// Truncate a string to max characters, appending "..." if truncated.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Find a safe char boundary
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_capability_creation() {
        let cap = Capability::new("test", "Test capability");
        assert_eq!(cap.proficiency, 0.5);
        assert_eq!(cap.usage_count, 0);
    }

    #[test]
    fn test_capability_usage() {
        let mut cap = Capability::new("test", "Test");

        cap.record_usage(true);
        assert_eq!(cap.usage_count, 1);
        assert!(cap.success_rate > 0.0);

        cap.record_usage(false);
        assert_eq!(cap.usage_count, 2);
    }

    #[test]
    fn test_identity_default() {
        let identity = Identity::default();
        assert_eq!(identity.name, "Zeus");
        assert!(!identity.values.is_empty());
        assert!(!identity.traits.is_empty());
    }

    #[tokio::test]
    async fn test_meta_cognition_without_learning() {
        let meta = MetaCognition::new();

        let reflection = meta.reflect(None).await;
        assert!(reflection.health > 0.0);
        assert!(!reflection.summary.is_empty());
        assert!(reflection.learned_insights.is_empty());
    }

    #[tokio::test]
    async fn test_capabilities_tracking() {
        let meta = MetaCognition::new();

        let caps = meta.capabilities().await;
        assert!(caps.iter().any(|c| c.name == "understanding"));
        assert!(caps.iter().any(|c| c.name == "reasoning"));
        assert!(caps.iter().any(|c| c.name == "execution"));
    }

    #[test]
    fn test_capability_max_proficiency() {
        let mut cap = Capability::new("test", "Test proficiency cap");
        for _ in 0..100 {
            cap.record_usage(true);
        }
        assert!(cap.proficiency > 0.5);
        assert!(cap.proficiency <= 1.0);
    }

    #[test]
    fn test_capability_many_uses() {
        let mut cap = Capability::new("busy_tool", "Heavily used capability");
        for _ in 0..100 {
            cap.record_usage(true);
        }
        assert_eq!(cap.usage_count, 100);
        assert!(cap.success_rate > 0.9);
    }

    #[test]
    fn test_identity_values() {
        let identity = Identity::default();
        assert_eq!(identity.values.len(), 5);
        assert!(identity.values.iter().any(|v| v.contains("Helpfulness")));
        assert!(identity.values.iter().any(|v| v.contains("Honesty")));
        assert!(identity.values.iter().any(|v| v.contains("Growth")));
        assert!(identity.values.iter().any(|v| v.contains("Privacy")));
        assert!(identity.values.iter().any(|v| v.contains("Autonomy")));
    }

    #[test]
    fn test_identity_traits() {
        let identity = Identity::default();
        assert_eq!(identity.traits.len(), 5);
        assert!(identity.traits.contains(&"Thoughtful".to_string()));
        assert!(identity.traits.contains(&"Proactive".to_string()));
        assert!(identity.traits.contains(&"Adaptable".to_string()));
        assert!(identity.traits.contains(&"Reliable".to_string()));
        assert!(identity.traits.contains(&"Curious".to_string()));
    }

    #[test]
    fn test_capability_success_rate_tracking() {
        let mut cap = Capability::new("mixed", "Mixed success/failure");
        for i in 0..20 {
            cap.record_usage(i % 2 == 0);
        }
        assert_eq!(cap.usage_count, 20);
        assert!(cap.success_rate > 0.3);
        assert!(cap.success_rate < 0.7);
    }

    #[tokio::test]
    async fn test_reflect_with_learning_engine() {
        let meta = MetaCognition::new();
        let engine = LearningEngine::new().await.unwrap();

        // Add some lessons
        let outcome1 = crate::learning::Outcome {
            intent_id: "test-1".to_string(),
            success: true,
            feedback: "Database indexing dramatically improved query performance".to_string(),
            timestamp: chrono::Utc::now(),
        };
        engine.extract_lesson(&outcome1).await.unwrap();

        let outcome2 = crate::learning::Outcome {
            intent_id: "test-2".to_string(),
            success: false,
            feedback: "Recursive file scan caused memory exhaustion".to_string(),
            timestamp: chrono::Utc::now(),
        };
        engine.extract_lesson(&outcome2).await.unwrap();

        let reflection = meta.reflect(Some(&engine)).await;

        // Should have learned insights (the successful one starts at 0.5 confidence,
        // which is below the 0.6 threshold, so it won't be in insights)
        // But the summary should mention lesson count
        assert!(reflection.summary.contains("2 lessons learned"));

        // Should have improvement needs from the failed approach
        let has_learned_avoidance = reflection
            .improvement_needs
            .iter()
            .any(|n| n.area.starts_with("learned-avoidance:"));
        assert!(
            has_learned_avoidance,
            "should have learned-avoidance improvement need"
        );
    }

    #[tokio::test]
    async fn test_reflect_with_reinforced_lessons() {
        let meta = MetaCognition::new();
        let engine = LearningEngine::new().await.unwrap();

        // Create and reinforce a lesson to push confidence above 0.6
        let outcome = crate::learning::Outcome {
            intent_id: "reinforce-1".to_string(),
            success: true,
            feedback: "Caching layer approach significantly reduced latency here".to_string(),
            timestamp: chrono::Utc::now(),
        };
        engine.extract_lesson(&outcome).await.unwrap();

        // Reinforce with similar outcome
        let outcome2 = crate::learning::Outcome {
            intent_id: "reinforce-2".to_string(),
            success: true,
            feedback: "Caching layer approach significantly reduced database load here".to_string(),
            timestamp: chrono::Utc::now(),
        };
        engine.extract_lesson(&outcome2).await.unwrap();

        let reflection = meta.reflect(Some(&engine)).await;

        // Reinforced lesson should have confidence >= 0.6 and appear in insights
        assert!(
            !reflection.learned_insights.is_empty(),
            "reinforced lesson should appear in insights"
        );
        assert!(
            reflection
                .learned_insights
                .iter()
                .any(|i| i.contains("SuccessfulStrategy")),
            "should contain successful strategy insight"
        );
    }

    #[tokio::test]
    async fn test_reflect_empty_learning_engine() {
        let meta = MetaCognition::new();
        let engine = LearningEngine::new().await.unwrap();

        let reflection = meta.reflect(Some(&engine)).await;

        // Empty engine should produce no insights and no lesson count in summary
        assert!(reflection.learned_insights.is_empty());
        assert!(!reflection.summary.contains("lessons learned"));
    }

    #[tokio::test]
    async fn test_reflect_insights_capped() {
        let meta = MetaCognition::new();
        let engine = LearningEngine::new().await.unwrap();

        // Create many distinct high-confidence lessons by reinforcing each one
        for i in 0..15 {
            let feedback = format!(
                "Unique strategy alpha-{} worked brilliantly on scenario-{}",
                i, i
            );
            let outcome = crate::learning::Outcome {
                intent_id: format!("cap-{}-a", i),
                success: true,
                feedback: feedback.clone(),
                timestamp: chrono::Utc::now(),
            };
            engine.extract_lesson(&outcome).await.unwrap();

            // Reinforce to push above 0.6 threshold
            let outcome2 = crate::learning::Outcome {
                intent_id: format!("cap-{}-b", i),
                success: true,
                feedback: format!(
                    "Unique strategy alpha-{} worked brilliantly on variant-{}",
                    i, i
                ),
                timestamp: chrono::Utc::now(),
            };
            engine.extract_lesson(&outcome2).await.unwrap();
        }

        let reflection = meta.reflect(Some(&engine)).await;

        // Insights should be capped at 10
        assert!(
            reflection.learned_insights.len() <= 10,
            "insights should be capped at 10, got {}",
            reflection.learned_insights.len()
        );
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_exact() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_long() {
        let result = truncate("hello world this is long", 10);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 13); // 10 + "..."
    }

    #[test]
    fn test_truncate_unicode() {
        // Should not panic on multi-byte chars
        let result = truncate("Hello 🌍 world", 8);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_reflection_serialization() {
        let reflection = Reflection {
            health: 0.75,
            state: CognitiveState::Normal,
            current_focus: Some("testing".to_string()),
            recent_successes: vec!["test passed".to_string()],
            recent_challenges: vec![],
            improvement_needs: vec![],
            learned_insights: vec!["Strategy X works well".to_string()],
            summary: "All good".to_string(),
            timestamp: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&reflection).unwrap();
        let deser: Reflection = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.learned_insights.len(), 1);
        assert!(deser.learned_insights[0].contains("Strategy X"));
    }

    #[test]
    fn test_reflection_backward_compat_deserialization() {
        // Old reflections without learned_insights should still deserialize
        let json = r#"{
            "health": 0.5,
            "state": "Normal",
            "current_focus": null,
            "recent_successes": [],
            "recent_challenges": [],
            "improvement_needs": [],
            "summary": "test",
            "timestamp": "2026-02-18T00:00:00Z"
        }"#;
        let reflection: Reflection = serde_json::from_str(json).unwrap();
        assert!(reflection.learned_insights.is_empty());
    }
}
