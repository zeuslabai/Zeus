//! Zeus Nous (νοῦς) - The Mind
//!
//! The cognitive core of Zeus that enables understanding, reasoning, learning,
//! and meta-cognition. Named after the Greek philosophical concept of "nous" -
//! the faculty of intellectual perception and understanding.
//!
//! ## Components
//!
//! - **Intent**: Deep understanding of user requests beyond literal meaning
//! - **Reasoning**: Chain-of-thought problem solving and planning
//! - **Learning**: Active learning from interactions and outcomes
//! - **Meta**: Self-awareness and reflection on capabilities
//!
//! ## Philosophy
//!
//! Nous transforms Zeus from a reactive tool into a thinking partner by:
//! 1. Understanding implicit intent, not just explicit commands
//! 2. Reasoning through complex multi-step problems
//! 3. Learning from every interaction to improve over time
//! 4. Reflecting on its own performance and limitations
//!
//! ## Example
//!
//! ```no_run
//! use zeus_nous::{Nous, Intent, UserContext};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let nous = Nous::new().await?;
//!
//!     // Understand user intent
//!     let intent = nous.understand("schedule something with the team").await?;
//!     println!("Inferred: {:?}", intent);
//!
//!     // Reason through a problem
//!     let plan = nous.reason("Plan a product launch").await?;
//!
//!     // Learn from outcome
//!     nous.learn_outcome(&intent, true, "Meeting scheduled successfully").await?;
//!
//!     Ok(())
//! }
//! ```

pub mod complexity;
pub mod confidence;
pub mod consolidation;
pub mod context_journal;
pub mod critic;
pub mod goal_tracker;
pub mod intent;
pub mod learning;
pub mod meta;
pub mod reasoning;
pub use goal_tracker::{Goal, GoalPriority, GoalStatus, GoalSummary, GoalTracker, Milestone};

pub use complexity::{ComplexityAnalyzer, ComplexityLevel, PlanCard, RiskLevel, StepSummary};
pub use confidence::{
    ConfidenceScorer, ConfidenceState, ConfidenceThresholds, LifecycleSummary, ScoredLesson,
};
pub use context_journal::{
    CalibrationReport, ContextJournal, DecisionOutcome, DecisionPattern, JournalConfig,
    JournalEntry, JournalStats,
};
pub use critic::{CriticEngine, Evaluation, ExecutionContext, TaskOutcome};
pub use intent::{Confidence, Intent, IntentEngine, IntentType};
pub use learning::{LearningEngine, Lesson, Outcome};
pub use meta::{Capability, MetaCognition, Reflection};
pub use reasoning::{ReasoningEngine, Step, ThoughtChain};

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use zeus_core::Result;
use zeus_mnemosyne::Mnemosyne;

/// The Mind of Zeus - coordinates all cognitive functions
pub struct Nous {
    /// Intent understanding engine
    intent: IntentEngine,
    /// Reasoning and planning engine
    reasoning: ReasoningEngine,
    /// Learning from experience
    learning: LearningEngine,
    /// Meta-cognitive reflection
    meta: MetaCognition,
    /// Critic for evaluating execution outcomes
    critic: CriticEngine,
    /// User context for personalization
    context: Arc<RwLock<UserContext>>,
}

impl Nous {
    /// Create a new Nous instance (in-memory learning, no persistence)
    pub async fn new() -> Result<Self> {
        let context = Arc::new(RwLock::new(UserContext::default()));

        Ok(Self {
            intent: IntentEngine::new(context.clone()),
            reasoning: ReasoningEngine::new(context.clone()),
            learning: LearningEngine::new().await?,
            meta: MetaCognition::new(),
            critic: CriticEngine::new(),
            context,
        })
    }

    /// Create a Nous instance with Mnemosyne-backed persistent learning.
    ///
    /// Lessons are persisted as Semantic memories and loaded on startup.
    pub async fn with_mnemosyne(mnemosyne: Arc<Mnemosyne>) -> Result<Self> {
        let context = Arc::new(RwLock::new(UserContext::default()));

        Ok(Self {
            intent: IntentEngine::new(context.clone()),
            reasoning: ReasoningEngine::new(context.clone()),
            learning: LearningEngine::with_mnemosyne(mnemosyne).await?,
            meta: MetaCognition::new(),
            critic: CriticEngine::new(),
            context,
        })
    }

    /// Understand a user's request, inferring implicit intent
    pub async fn understand(&self, input: &str) -> Result<Intent> {
        // Get user context for personalization
        let ctx = self.context.read().await;

        // Parse and infer intent
        let intent = self.intent.analyze(input, &ctx).await?;

        // Record for learning
        drop(ctx);
        self.learning.record_intent(&intent).await?;

        // Reflect on understanding
        self.meta.record_understanding(&intent).await;

        Ok(intent)
    }

    /// Reason through a problem, creating a plan
    pub async fn reason(&self, problem: &str) -> Result<ThoughtChain> {
        let ctx = self.context.read().await;

        // Generate reasoning chain
        let chain = self.reasoning.think(problem, &ctx).await?;

        // Record for learning
        drop(ctx);
        self.learning.record_reasoning(&chain).await?;

        Ok(chain)
    }

    /// Learn from the outcome of an action
    pub async fn learn_outcome(
        &self,
        intent: &Intent,
        success: bool,
        feedback: &str,
    ) -> Result<Lesson> {
        let outcome = Outcome {
            intent_id: intent.id.clone(),
            success,
            feedback: feedback.to_string(),
            timestamp: chrono::Utc::now(),
        };

        // Extract lesson from outcome
        let lesson = self.learning.extract_lesson(&outcome).await?;

        // Update meta-cognition
        self.meta.record_outcome(&outcome).await;

        Ok(lesson)
    }

    /// Evaluate an execution outcome using the critic engine
    pub fn evaluate(&self, ctx: &ExecutionContext) -> Evaluation {
        self.critic.evaluate(ctx)
    }

    /// Get a reference to the critic engine
    /// Wire an LLM into the reasoning engine for LLM-backed thought synthesis.
    /// Call once after construction; safe to call multiple times (replaces previous).
    pub fn set_llm(&mut self, llm: Arc<zeus_llm::LlmClient>) {
        self.reasoning = ReasoningEngine::with_llm(self.context.clone(), llm);
    }

    pub fn critic(&self) -> &CriticEngine {
        &self.critic
    }

    /// Get current self-assessment, enriched with learned lessons
    pub async fn reflect(&self) -> Reflection {
        self.meta.reflect(Some(&self.learning)).await
    }

    /// Update user context
    pub async fn update_context(&self, update: ContextUpdate) {
        let mut ctx = self.context.write().await;
        ctx.apply_update(update);
    }

    /// Get current capabilities assessment
    pub async fn capabilities(&self) -> Vec<Capability> {
        self.meta.capabilities().await
    }

    /// Get learning engine statistics
    pub async fn learning_stats(&self) -> learning::LearningStats {
        self.learning.stats().await
    }

    /// Get all learned lessons
    pub async fn all_lessons(&self) -> Vec<Lesson> {
        self.learning.all_lessons().await
    }

    /// Get lessons relevant to the current context (instinct recall)
    pub async fn get_relevant_lessons(&self, context: &str) -> Vec<Lesson> {
        self.learning.get_relevant_lessons(context).await
    }

    /// Apply time-based confidence decay to all lessons.
    /// Returns the number of lessons updated.
    pub async fn run_decay(&self) -> usize {
        let scorer = ConfidenceScorer::default();
        scorer.apply_decay_all(&self.learning).await
    }
}

/// User context for personalization
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserContext {
    /// User's name
    pub name: Option<String>,
    /// User's timezone
    pub timezone: Option<String>,
    /// Known preferences
    pub preferences: Vec<Preference>,
    /// Recent topics
    pub recent_topics: Vec<String>,
    /// Communication style preference
    pub style: CommunicationStyle,
    /// Known entities (people, projects, etc.)
    pub entities: Vec<Entity>,
    /// Historical patterns
    pub patterns: Vec<Pattern>,
}

impl UserContext {
    /// Apply an update to the context
    pub fn apply_update(&mut self, update: ContextUpdate) {
        match update {
            ContextUpdate::SetName(name) => self.name = Some(name),
            ContextUpdate::SetTimezone(tz) => self.timezone = Some(tz),
            ContextUpdate::AddPreference(pref) => self.preferences.push(pref),
            ContextUpdate::AddTopic(topic) => {
                self.recent_topics.insert(0, topic);
                if self.recent_topics.len() > 20 {
                    self.recent_topics.pop();
                }
            }
            ContextUpdate::AddEntity(entity) => self.entities.push(entity),
            ContextUpdate::AddPattern(pattern) => self.patterns.push(pattern),
            ContextUpdate::SetStyle(style) => self.style = style,
            ContextUpdate::NewInteraction { input } => {
                // Track as recent topic (truncated) for context continuity
                let topic = if input.len() > 100 {
                    format!("{}...", zeus_core::truncate_str(&input, 97))
                } else {
                    input
                };
                self.recent_topics.insert(0, topic);
                if self.recent_topics.len() > 20 {
                    self.recent_topics.pop();
                }
            }
        }
    }

    /// Check if a preference exists
    pub fn has_preference(&self, key: &str) -> Option<&Preference> {
        self.preferences.iter().find(|p| p.key == key)
    }

    /// Get an entity by name
    pub fn get_entity(&self, name: &str) -> Option<&Entity> {
        self.entities.iter().find(|e| {
            e.name.to_lowercase() == name.to_lowercase()
                || e.aliases
                    .iter()
                    .any(|a| a.to_lowercase() == name.to_lowercase())
        })
    }
}

/// Context update types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContextUpdate {
    SetName(String),
    SetTimezone(String),
    AddPreference(Preference),
    AddTopic(String),
    AddEntity(Entity),
    AddPattern(Pattern),
    SetStyle(CommunicationStyle),
    /// New user interaction — tracks input as topic + enables pattern recognition
    NewInteraction {
        input: String,
    },
}

/// A user preference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preference {
    /// Preference key
    pub key: String,
    /// Preference value
    pub value: String,
    /// Confidence (0.0 - 1.0)
    pub confidence: f32,
    /// How the preference was learned
    pub source: PreferenceSource,
    /// When it was last confirmed
    pub last_confirmed: chrono::DateTime<chrono::Utc>,
}

/// How a preference was learned
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreferenceSource {
    /// Explicitly stated by user
    Explicit,
    /// Inferred from behavior
    Inferred,
    /// Learned from corrections
    Corrected,
}

/// Communication style preference
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommunicationStyle {
    /// Preferred verbosity (0.0 = terse, 1.0 = detailed)
    pub verbosity: f32,
    /// Preferred formality (0.0 = casual, 1.0 = formal)
    pub formality: f32,
    /// Use of technical language
    pub technical: bool,
    /// Preferred response format
    pub format: ResponseFormat,
}

/// Response format preference
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum ResponseFormat {
    #[default]
    Prose,
    Bullets,
    Structured,
    Minimal,
}

/// A known entity (person, project, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// Entity name
    pub name: String,
    /// Alternative names
    pub aliases: Vec<String>,
    /// Entity type
    pub entity_type: EntityType,
    /// Attributes
    pub attributes: serde_json::Value,
    /// Relationships to other entities
    pub relationships: Vec<Relationship>,
}

/// Entity types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EntityType {
    Person,
    Project,
    Organization,
    Location,
    Event,
    Document,
    Tool,
    Custom(String),
}

/// Relationship between entities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    /// Relationship type
    pub relation: String,
    /// Target entity name
    pub target: String,
}

/// A learned pattern in user behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    /// Pattern description
    pub description: String,
    /// When the pattern typically occurs
    pub trigger: PatternTrigger,
    /// What the user typically wants
    pub typical_action: String,
    /// Confidence in this pattern
    pub confidence: f32,
    /// Number of times observed
    pub observations: u32,
}

/// What triggers a pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PatternTrigger {
    /// Time-based (e.g., "every Monday morning")
    Temporal(String),
    /// Event-based (e.g., "after receiving email from X")
    Event(String),
    /// Context-based (e.g., "when working on project X")
    Context(String),
    /// Keyword-based (e.g., "when mentioning 'deploy'")
    Keyword(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_context_default() {
        let ctx = UserContext::default();
        assert!(ctx.name.is_none());
        assert!(ctx.preferences.is_empty());
    }

    #[test]
    fn test_context_update() {
        let mut ctx = UserContext::default();
        ctx.apply_update(ContextUpdate::SetName("Mike".to_string()));
        assert_eq!(ctx.name, Some("Mike".to_string()));
    }

    #[test]
    fn test_preference_lookup() {
        let mut ctx = UserContext::default();
        ctx.preferences.push(Preference {
            key: "meeting_time".to_string(),
            value: "morning".to_string(),
            confidence: 0.9,
            source: PreferenceSource::Explicit,
            last_confirmed: chrono::Utc::now(),
        });

        assert!(ctx.has_preference("meeting_time").is_some());
        assert!(ctx.has_preference("unknown").is_none());
    }

    #[test]
    fn test_entity_lookup() {
        let mut ctx = UserContext::default();
        ctx.entities.push(Entity {
            name: "Project Zeus".to_string(),
            aliases: vec!["Zeus".to_string(), "the project".to_string()],
            entity_type: EntityType::Project,
            attributes: serde_json::json!({"language": "Rust"}),
            relationships: vec![],
        });

        assert!(ctx.get_entity("Zeus").is_some());
        assert!(ctx.get_entity("zeus").is_some()); // Case insensitive
        assert!(ctx.get_entity("Project Zeus").is_some());
        assert!(ctx.get_entity("Unknown").is_none());
    }

    #[test]
    fn test_recent_topics_limit() {
        let mut ctx = UserContext::default();
        for i in 0..25 {
            ctx.apply_update(ContextUpdate::AddTopic(format!("topic_{}", i)));
        }
        assert_eq!(ctx.recent_topics.len(), 20);
        assert_eq!(ctx.recent_topics[0], "topic_24"); // Most recent first
    }

    #[test]
    fn test_user_context_empty_topics() {
        let ctx = UserContext::default();
        assert!(ctx.recent_topics.is_empty());
        // get_entity on empty should return None
        assert!(ctx.get_entity("anything").is_none());
    }

    #[test]
    fn test_user_context_duplicate_preferences() {
        let mut ctx = UserContext::default();
        ctx.apply_update(ContextUpdate::AddPreference(Preference {
            key: "theme".to_string(),
            value: "dark".to_string(),
            confidence: 0.8,
            source: PreferenceSource::Explicit,
            last_confirmed: chrono::Utc::now(),
        }));
        ctx.apply_update(ContextUpdate::AddPreference(Preference {
            key: "theme".to_string(),
            value: "light".to_string(),
            confidence: 0.9,
            source: PreferenceSource::Corrected,
            last_confirmed: chrono::Utc::now(),
        }));
        // AddPreference pushes without dedup, so both exist
        assert_eq!(ctx.preferences.len(), 2);
        // has_preference finds the first one
        let found = ctx
            .has_preference("theme")
            .expect("has_preference should succeed");
        assert_eq!(found.value, "dark");
    }

    #[test]
    fn test_user_context_entity_alias_case_insensitive() {
        let mut ctx = UserContext::default();
        ctx.entities.push(Entity {
            name: "Alice".to_string(),
            aliases: vec!["ALICE".to_string(), "alice_dev".to_string()],
            entity_type: EntityType::Person,
            attributes: serde_json::json!({}),
            relationships: vec![],
        });

        // Lookup by alias in different case
        assert!(ctx.get_entity("alice").is_some());
        assert!(ctx.get_entity("ALICE").is_some());
        assert!(ctx.get_entity("Alice_Dev").is_some());
        assert!(ctx.get_entity("bob").is_none());
    }

    #[test]
    fn test_user_context_many_topics() {
        let mut ctx = UserContext::default();
        for i in 0..25 {
            ctx.apply_update(ContextUpdate::AddTopic(format!("topic_{}", i)));
        }
        // Max 20 topics kept
        assert_eq!(ctx.recent_topics.len(), 20);
        // Most recent is first, oldest trimmed
        assert_eq!(ctx.recent_topics[0], "topic_24");
        assert_eq!(ctx.recent_topics[19], "topic_5");
    }

    #[test]
    fn test_user_context_empty_entity() {
        let mut ctx = UserContext::default();
        ctx.entities.push(Entity {
            name: "".to_string(),
            aliases: vec![],
            entity_type: EntityType::Custom("empty".to_string()),
            attributes: serde_json::json!(null),
            relationships: vec![],
        });

        // Empty string entity: lookup with empty string matches because "".to_lowercase() == ""
        assert!(ctx.get_entity("").is_some());
        // Non-empty should not match
        assert!(ctx.get_entity("something").is_none());
    }

    #[test]
    fn test_user_context_preference_overwrite() {
        let mut ctx = UserContext::default();
        ctx.preferences.push(Preference {
            key: "lang".to_string(),
            value: "rust".to_string(),
            confidence: 0.7,
            source: PreferenceSource::Inferred,
            last_confirmed: chrono::Utc::now(),
        });

        // Verify initial value
        assert_eq!(
            ctx.has_preference("lang")
                .expect("has_preference should succeed")
                .value,
            "rust"
        );

        // Push a new preference with same key
        ctx.preferences.push(Preference {
            key: "lang".to_string(),
            value: "python".to_string(),
            confidence: 0.9,
            source: PreferenceSource::Explicit,
            last_confirmed: chrono::Utc::now(),
        });

        // has_preference finds first match (still "rust")
        assert_eq!(
            ctx.has_preference("lang")
                .expect("has_preference should succeed")
                .value,
            "rust"
        );
        assert_eq!(ctx.preferences.len(), 2);
    }

    #[test]
    fn test_user_context_default_trait() {
        let ctx = UserContext::default();
        assert!(ctx.name.is_none());
        assert!(ctx.timezone.is_none());
        assert!(ctx.preferences.is_empty());
        assert!(ctx.recent_topics.is_empty());
        assert!(ctx.entities.is_empty());
        assert!(ctx.patterns.is_empty());
        // Default CommunicationStyle
        assert_eq!(ctx.style.verbosity, 0.0);
        assert_eq!(ctx.style.formality, 0.0);
        assert!(!ctx.style.technical);
    }

    #[test]
    fn test_user_context_debug_trait() {
        let ctx = UserContext::default();
        let debug_str = format!("{:?}", ctx);
        assert!(debug_str.contains("UserContext"));
        assert!(debug_str.contains("name"));
        assert!(debug_str.contains("preferences"));
    }
}
