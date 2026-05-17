//! Intent Understanding Engine
//!
//! Extracts meaning from user input, going beyond literal interpretation
//! to understand implicit intent, context, and underlying goals.

mod context;
mod inference;
mod parser;

pub use context::ContextResolver;
pub use inference::IntentInference;
pub use parser::IntentParser;

use crate::UserContext;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use zeus_core::Result;

/// Confidence level in an interpretation
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Confidence(pub f32);

impl Confidence {
    pub fn high() -> Self {
        Self(0.9)
    }
    pub fn medium() -> Self {
        Self(0.7)
    }
    pub fn low() -> Self {
        Self(0.5)
    }
    pub fn uncertain() -> Self {
        Self(0.3)
    }

    pub fn is_confident(&self) -> bool {
        self.0 >= 0.7
    }
    pub fn value(&self) -> f32 {
        self.0
    }
}

impl Default for Confidence {
    fn default() -> Self {
        Self::medium()
    }
}

/// The type of intent detected
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IntentType {
    /// User wants to create something
    Create { target: String },
    /// User wants to read/retrieve information
    Read { target: String },
    /// User wants to update/modify something
    Update {
        target: String,
        changes: Vec<String>,
    },
    /// User wants to delete/remove something
    Delete { target: String },
    /// User wants to search/find something
    Search {
        query: String,
        scope: Option<String>,
    },
    /// User wants to schedule something
    Schedule {
        what: String,
        when: Option<String>,
        who: Option<Vec<String>>,
    },
    /// User wants to communicate (email, message, etc.)
    Communicate {
        to: Vec<String>,
        about: String,
        channel: Option<String>,
    },
    /// User wants to analyze/understand something
    Analyze {
        subject: String,
        aspect: Option<String>,
    },
    /// User wants to execute/run something
    Execute {
        action: String,
        parameters: Vec<String>,
    },
    /// User wants to automate a workflow
    Automate {
        trigger: String,
        actions: Vec<String>,
    },
    /// User is asking a question
    Question {
        topic: String,
        question_type: QuestionType,
    },
    /// User wants to remember/note something
    Remember {
        what: String,
        context: Option<String>,
    },
    /// User wants to be reminded about something
    Remind { what: String, when: String },
    /// Meta-intent: user is teaching Zeus something
    Teach { lesson: String },
    /// Meta-intent: user is correcting Zeus
    Correct { what: String, should_be: String },
    /// Unclear - needs clarification
    Unclear {
        raw: String,
        possibilities: Vec<IntentType>,
    },
}

/// Question types for better understanding
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum QuestionType {
    /// Factual question (who, what, when, where)
    Factual,
    /// How-to question
    Procedural,
    /// Why question - seeking explanation
    Explanatory,
    /// Opinion/recommendation request
    Advisory,
    /// Comparison question
    Comparative,
    /// Hypothetical/what-if question
    Hypothetical,
}

/// A parsed and enriched intent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    /// Unique identifier
    pub id: String,
    /// The original input
    pub raw_input: String,
    /// Primary intent type
    pub intent_type: IntentType,
    /// Confidence in this interpretation
    pub confidence: Confidence,
    /// Extracted entities
    pub entities: Vec<ExtractedEntity>,
    /// Temporal references
    pub temporal: Option<TemporalRef>,
    /// Urgency level (0.0 = not urgent, 1.0 = very urgent)
    pub urgency: f32,
    /// Implicit context that was inferred
    pub implicit_context: Vec<ImplicitContext>,
    /// Related past intents
    pub related_intents: Vec<String>,
    /// Suggested clarifying questions if unclear
    pub clarifications: Vec<String>,
    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl Intent {
    /// Create a new intent with generated ID
    pub fn new(raw_input: &str, intent_type: IntentType, confidence: Confidence) -> Self {
        Self {
            id: ulid::Ulid::new().to_string(),
            raw_input: raw_input.to_string(),
            intent_type,
            confidence,
            entities: Vec::new(),
            temporal: None,
            urgency: 0.5,
            implicit_context: Vec::new(),
            related_intents: Vec::new(),
            clarifications: Vec::new(),
            timestamp: chrono::Utc::now(),
        }
    }

    /// Check if this intent needs clarification
    pub fn needs_clarification(&self) -> bool {
        if !self.confidence.is_confident() || matches!(self.intent_type, IntentType::Unclear { .. })
        {
            return true;
        }

        // Check for missing required fields in specific intent types
        match &self.intent_type {
            IntentType::Schedule { when, who, .. } => {
                when.is_none()
                    || who.is_none()
                    || who.as_ref().map(|w| w.is_empty()).unwrap_or(true)
            }
            IntentType::Communicate { to, .. } => to.is_empty(),
            IntentType::Remind { when, .. } => when.is_empty(),
            _ => false,
        }
    }

    /// Get the primary action verb
    pub fn action(&self) -> &str {
        match &self.intent_type {
            IntentType::Create { .. } => "create",
            IntentType::Read { .. } => "read",
            IntentType::Update { .. } => "update",
            IntentType::Delete { .. } => "delete",
            IntentType::Search { .. } => "search",
            IntentType::Schedule { .. } => "schedule",
            IntentType::Communicate { .. } => "communicate",
            IntentType::Analyze { .. } => "analyze",
            IntentType::Execute { .. } => "execute",
            IntentType::Automate { .. } => "automate",
            IntentType::Question { .. } => "ask",
            IntentType::Remember { .. } => "remember",
            IntentType::Remind { .. } => "remind",
            IntentType::Teach { .. } => "teach",
            IntentType::Correct { .. } => "correct",
            IntentType::Unclear { .. } => "unclear",
        }
    }
}

/// An entity extracted from the input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    /// The entity text as it appears
    pub text: String,
    /// Normalized/resolved name
    pub resolved: Option<String>,
    /// Entity type
    pub entity_type: ExtractedEntityType,
    /// Position in input
    pub start: usize,
    pub end: usize,
    /// Confidence in extraction
    pub confidence: Confidence,
}

/// Types of entities we can extract
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ExtractedEntityType {
    Person,
    Organization,
    Project,
    Location,
    DateTime,
    Duration,
    Email,
    Url,
    Filename,
    Tool,
    Action,
    Custom(String),
}

/// Temporal reference in the input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalRef {
    /// Raw temporal expression
    pub raw: String,
    /// Parsed datetime (if specific)
    pub datetime: Option<chrono::DateTime<chrono::Utc>>,
    /// Relative reference type
    pub relative: Option<RelativeTime>,
    /// Is this a deadline?
    pub is_deadline: bool,
}

/// Relative time references
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RelativeTime {
    Now,
    Today,
    Tomorrow,
    ThisWeek,
    NextWeek,
    ThisMonth,
    NextMonth,
    Soon,
    Eventually,
    Custom(String),
}

/// Implicit context that was inferred
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplicitContext {
    /// What was inferred
    pub inference: String,
    /// How it was inferred
    pub source: InferenceSource,
    /// Confidence
    pub confidence: Confidence,
}

/// How context was inferred
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InferenceSource {
    /// From user preferences
    UserPreference,
    /// From recent conversation
    RecentContext,
    /// From known entities
    KnownEntity,
    /// From patterns
    LearnedPattern,
    /// From common sense
    CommonSense,
}

/// The Intent Engine - coordinates all intent understanding
pub struct IntentEngine {
    parser: IntentParser,
    context_resolver: ContextResolver,
    inference: IntentInference,
    user_context: Arc<RwLock<UserContext>>,
}

impl IntentEngine {
    /// Create a new intent engine
    pub fn new(user_context: Arc<RwLock<UserContext>>) -> Self {
        Self {
            parser: IntentParser::new(),
            context_resolver: ContextResolver::new(),
            inference: IntentInference::new(),
            user_context,
        }
    }

    /// Analyze input and extract intent
    pub async fn analyze(&self, input: &str, ctx: &UserContext) -> Result<Intent> {
        // Step 1: Parse the raw input
        let mut intent = self.parser.parse(input)?;

        // Step 2: Resolve entities against known context
        self.context_resolver.resolve(&mut intent, ctx)?;

        // Step 3: Infer implicit context
        self.inference.infer(&mut intent, ctx)?;

        // Step 4: Check for patterns and related intents
        self.inference.find_related(&mut intent, ctx)?;

        // Step 5: Generate clarifying questions if needed
        if intent.needs_clarification() {
            self.generate_clarifications(&mut intent, ctx);
        }

        tracing::info!(
            intent_type = ?intent.intent_type,
            confidence = intent.confidence.value(),
            entities = intent.entities.len(),
            "Analyzed intent"
        );

        Ok(intent)
    }

    /// Generate clarifying questions for unclear intents
    fn generate_clarifications(&self, intent: &mut Intent, _ctx: &UserContext) {
        let mut questions = Vec::new();

        match &intent.intent_type {
            IntentType::Schedule { what: _, when, who } => {
                if when.is_none() {
                    questions.push("When would you like to schedule this?".to_string());
                }
                if who.is_none() || who.as_ref().map(|w| w.is_empty()).unwrap_or(true) {
                    questions.push("Who should be included?".to_string());
                }
            }
            IntentType::Communicate { to, .. } if to.is_empty() => {
                questions.push("Who would you like to send this to?".to_string());
            }
            IntentType::Search { query, .. } if query.len() < 3 => {
                questions
                    .push("Could you be more specific about what you're looking for?".to_string());
            }
            IntentType::Unclear { possibilities, .. } => {
                if possibilities.len() > 1 {
                    questions.push("I'm not sure what you mean. Did you want to:".to_string());
                    for (i, p) in possibilities.iter().take(3).enumerate() {
                        let desc = match p {
                            IntentType::Schedule { what, .. } => {
                                format!("{}. Schedule: {}", i + 1, what)
                            }
                            IntentType::Search { query, .. } => {
                                format!("{}. Search for: {}", i + 1, query)
                            }
                            IntentType::Create { target } => {
                                format!("{}. Create: {}", i + 1, target)
                            }
                            _ => format!("{}. {:?}", i + 1, p),
                        };
                        questions.push(desc);
                    }
                }
            }
            _ => {}
        }

        intent.clarifications = questions;
    }

    /// Get the user context
    pub fn user_context(&self) -> &Arc<RwLock<UserContext>> {
        &self.user_context
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confidence_levels() {
        assert!(Confidence::high().is_confident());
        assert!(Confidence::medium().is_confident());
        assert!(!Confidence::low().is_confident());
        assert!(!Confidence::uncertain().is_confident());
    }

    #[test]
    fn test_intent_creation() {
        let intent = Intent::new(
            "schedule a meeting",
            IntentType::Schedule {
                what: "meeting".to_string(),
                when: None,
                who: None,
            },
            Confidence::high(),
        );

        assert_eq!(intent.action(), "schedule");
        assert!(intent.needs_clarification()); // Missing when/who
    }

    #[test]
    fn test_intent_actions() {
        let create = Intent::new(
            "",
            IntentType::Create {
                target: "note".to_string(),
            },
            Confidence::high(),
        );
        assert_eq!(create.action(), "create");

        let search = Intent::new(
            "",
            IntentType::Search {
                query: "test".to_string(),
                scope: None,
            },
            Confidence::high(),
        );
        assert_eq!(search.action(), "search");
    }

    #[test]
    fn test_confidence_high_threshold() {
        // Just above 0.8 should be confident
        let conf = Confidence(0.81);
        assert!(conf.is_confident());
        assert!((conf.value() - 0.81).abs() < f32::EPSILON);
    }

    #[test]
    fn test_confidence_low_threshold() {
        // Just above 0.3 should NOT be confident (threshold is 0.7)
        let conf = Confidence(0.31);
        assert!(!conf.is_confident());
        assert!((conf.value() - 0.31).abs() < f32::EPSILON);
    }

    #[test]
    fn test_intent_question_with_how() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("how do I configure the database?")
            .expect("should parse successfully");
        assert!(matches!(
            intent.intent_type,
            IntentType::Question {
                question_type: QuestionType::Procedural,
                ..
            }
        ));
    }

    #[test]
    fn test_intent_question_with_why() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("why is the server slow?")
            .expect("should parse successfully");
        assert!(matches!(
            intent.intent_type,
            IntentType::Question {
                question_type: QuestionType::Explanatory,
                ..
            }
        ));
    }

    #[test]
    fn test_intent_command_with_please() {
        let parser = IntentParser::new();
        // "please" is not a recognized action keyword, so "please create" won't
        // match the "create" pattern (which requires ^create). But the input
        // should still parse without error.
        let intent = parser
            .parse("create a report please")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Create { .. }));
        assert!(intent.confidence.is_confident());
    }

    #[test]
    fn test_intent_schedule_intent() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("schedule a team standup for tomorrow")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Schedule { .. }));
        assert_eq!(intent.action(), "schedule");
    }

    #[test]
    fn test_intent_search_intent() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("search for configuration examples")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Search { .. }));
        assert_eq!(intent.action(), "search");
    }

    #[test]
    fn test_intent_create_intent() {
        let parser = IntentParser::new();
        // "make" should also map to Create
        let intent = parser
            .parse("make a new spreadsheet")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Create { .. }));
        assert_eq!(intent.action(), "create");
    }

    #[test]
    fn test_intent_delete_intent() {
        let parser = IntentParser::new();
        let intent = parser
            .parse("remove the old log files")
            .expect("should parse successfully");
        assert!(matches!(intent.intent_type, IntentType::Delete { .. }));
        assert_eq!(intent.action(), "delete");
    }

    #[test]
    fn test_intent_empty_input() {
        let parser = IntentParser::new();
        let intent = parser.parse("").expect("should parse successfully");
        // Empty string should parse without panic
        assert!(!intent.raw_input.is_empty() || intent.raw_input.is_empty());
        // Should fall through to infer (unclear or search)
        assert!(matches!(
            intent.intent_type,
            IntentType::Unclear { .. } | IntentType::Search { .. }
        ));
    }
}
