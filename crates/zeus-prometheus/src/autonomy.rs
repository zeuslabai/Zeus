//! Autonomous Decision Engine
//!
//! Determines the optimal action strategy based on intent analysis, available
//! context, and confidence thresholds. The engine sits between the intent
//! classifier and the executor/planner, deciding whether to proceed
//! autonomously, ask the user for confirmation, plan first, or reflect
//! on recent errors.
//!
//! The core principle: the agent should be autonomous enough to be useful,
//! but supervised enough to be safe. The [`AutonomyLevel`] and per-tool
//! confirmation requirements give users fine-grained control.

use crate::intent::{Intent, IntentAnalysis};
use crate::spawner::SpawnRequest;
use serde::{Deserialize, Serialize};
use tracing::debug;

// ============================================================================
// Types
// ============================================================================

/// The action the agent should take in response to a classified intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Decision {
    /// Respond directly to the user without tool use.
    RespondDirectly(String),
    /// Execute a specific tool.
    ExecuteTool(String),
    /// Decompose the task into steps, then execute the plan.
    PlanAndExecute,
    /// Ask the user a clarifying question before proceeding.
    AskUser(String),
    /// Delegate to a specific subsystem (e.g. "nous", "athena", "talos").
    Delegate(String),
    /// Pause and reflect on recent failures before continuing.
    Reflect,
    /// Spawn additional agents to handle subtasks in parallel.
    SpawnAgents(Vec<SpawnRequest>),
}

impl std::fmt::Display for Decision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Decision::RespondDirectly(reason) => write!(f, "respond_directly({})", reason),
            Decision::ExecuteTool(tool) => write!(f, "execute_tool({})", tool),
            Decision::PlanAndExecute => write!(f, "plan_and_execute"),
            Decision::AskUser(q) => write!(f, "ask_user({})", q),
            Decision::Delegate(sub) => write!(f, "delegate({})", sub),
            Decision::Reflect => write!(f, "reflect"),
            Decision::SpawnAgents(agents) => write!(f, "spawn_agents({})", agents.len()),
        }
    }
}

/// How much autonomy the agent is granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyLevel {
    /// The agent can execute any tool without confirmation.
    Full,
    /// The agent asks for confirmation on destructive tools but proceeds
    /// autonomously on safe ones.
    Supervised,
    /// The agent asks for confirmation on all tool uses.
    Restricted,
}

impl std::fmt::Display for AutonomyLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AutonomyLevel::Full => write!(f, "full"),
            AutonomyLevel::Supervised => write!(f, "supervised"),
            AutonomyLevel::Restricted => write!(f, "restricted"),
        }
    }
}

/// Configuration for the autonomy engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomyConfig {
    /// The autonomy level governing confirmation behavior.
    pub level: AutonomyLevel,
    /// Minimum confidence score required to proceed autonomously.
    /// If the intent classifier's confidence is below this threshold,
    /// the engine will ask the user for clarification.
    pub confidence_threshold: f32,
    /// Maximum number of tools the agent can execute autonomously in a
    /// single session before requiring user check-in.
    pub max_autonomous_tools: usize,
    /// Tool names that always require user confirmation, regardless of
    /// autonomy level (e.g. destructive operations).
    pub require_confirmation_for: Vec<String>,
    /// Number of consecutive errors that triggers a reflection step.
    pub error_threshold: usize,
}

impl Default for AutonomyConfig {
    fn default() -> Self {
        Self {
            level: AutonomyLevel::Supervised,
            confidence_threshold: 0.7,
            max_autonomous_tools: 5,
            require_confirmation_for: vec![
                "shell".to_string(),
                "write_file".to_string(),
                "edit_file".to_string(),
            ],
            error_threshold: 3,
        }
    }
}

/// Context provided to the autonomy engine for making a decision.
///
/// Captures the current state of the session and environment so the
/// engine can make informed choices.
#[derive(Debug, Clone)]
pub struct DecisionContext {
    /// The intent analysis from the classifier.
    pub intent: IntentAnalysis,
    /// Whether workspace memory context is available.
    pub has_memory_context: bool,
    /// Number of messages in the current session so far.
    pub session_message_count: usize,
    /// Number of consecutive recent errors (tool failures, LLM errors, etc.).
    pub recent_error_count: usize,
    /// Names of tools available in the current environment.
    pub available_tools: Vec<String>,
    /// Number of tools already executed autonomously in this session.
    pub autonomous_tool_count: usize,
}

// ============================================================================
// AutonomyEngine
// ============================================================================

/// The autonomous decision engine.
///
/// Given an [`IntentAnalysis`] and session context, produces a [`Decision`]
/// that tells the orchestration layer what to do next.
pub struct AutonomyEngine {
    config: AutonomyConfig,
}

impl AutonomyEngine {
    /// Create a new autonomy engine with the given configuration.
    pub fn new(config: AutonomyConfig) -> Self {
        Self { config }
    }

    /// Decide what action to take based on the current context.
    ///
    /// This is the core decision function. It considers the classified intent,
    /// confidence level, error history, autonomy level, and tool restrictions
    /// to produce a single [`Decision`].
    pub fn decide(&self, context: &DecisionContext) -> Decision {
        let intent = &context.intent;

        debug!(
            "Deciding action for intent={} confidence={:.2} errors={} level={}",
            intent.intent, intent.confidence, context.recent_error_count, self.config.level
        );

        // --- Error recovery: reflect after too many consecutive errors ---
        if context.recent_error_count >= self.config.error_threshold {
            debug!(
                "Error count ({}) exceeds threshold ({}), triggering reflection",
                context.recent_error_count, self.config.error_threshold
            );
            return Decision::Reflect;
        }

        // --- Route based on intent ---
        match intent.intent {
            Intent::SystemCommand => {
                Decision::RespondDirectly("System command detected".to_string())
            }

            Intent::Conversation => Decision::RespondDirectly("Conversational message".to_string()),

            Intent::Clarification => Decision::AskUser(
                "Could you provide more details about what you'd like me to do?".to_string(),
            ),

            Intent::SimpleQuery => {
                if intent.confidence >= self.config.confidence_threshold {
                    Decision::RespondDirectly("High-confidence query".to_string())
                } else {
                    Decision::AskUser(
                        "Could you rephrase that? I want to make sure I take the right action.".to_string(),
                    )
                }
            }

            Intent::ToolUse => self.decide_tool_use(context),

            Intent::ComplexTask => {
                // Complex tasks always go through planning, but we may want
                // confirmation first if confidence is low
                if intent.confidence < self.config.confidence_threshold {
                    Decision::AskUser(
                        "This looks like a complex task. Could you confirm or provide more details?"
                            .to_string(),
                    )
                } else {
                    Decision::PlanAndExecute
                }
            }
        }
    }

    /// Check whether a specific tool requires user confirmation.
    ///
    /// A tool requires confirmation if:
    /// - The autonomy level is [`AutonomyLevel::Restricted`], OR
    /// - The tool name appears in [`AutonomyConfig::require_confirmation_for`].
    pub fn should_confirm(&self, tool_name: &str) -> bool {
        match self.config.level {
            AutonomyLevel::Full => false,
            AutonomyLevel::Restricted => true,
            AutonomyLevel::Supervised => self
                .config
                .require_confirmation_for
                .iter()
                .any(|t| t == tool_name),
        }
    }

    /// Adjust a decision based on error history.
    ///
    /// If the error count is high, inject a reflection step regardless of
    /// what the original decision was. Otherwise returns the decision unchanged.
    pub fn adjust_for_errors(&self, decision: Decision, error_count: usize) -> Decision {
        if error_count >= self.config.error_threshold {
            debug!(
                "Overriding decision {:?} with Reflect due to {} errors",
                decision, error_count
            );
            Decision::Reflect
        } else if error_count > 0 {
            // With some errors but below threshold, add caution
            match decision {
                Decision::ExecuteTool(tool) => {
                    if self.should_confirm(&tool) {
                        Decision::AskUser(format!(
                            "There have been {} recent errors. Shall I proceed with {}?",
                            error_count, tool
                        ))
                    } else {
                        Decision::ExecuteTool(tool)
                    }
                }
                other => other,
            }
        } else {
            decision
        }
    }

    /// Get a reference to the current configuration.
    pub fn config(&self) -> &AutonomyConfig {
        &self.config
    }

    // ========================================================================
    // Private helpers
    // ========================================================================

    /// Decide what to do for a tool-use intent.
    fn decide_tool_use(&self, context: &DecisionContext) -> Decision {
        let intent = &context.intent;

        // Check if we've exceeded the autonomous tool limit
        if context.autonomous_tool_count >= self.config.max_autonomous_tools {
            return Decision::AskUser(format!(
                "I've executed {} tools autonomously this session. Shall I continue?",
                context.autonomous_tool_count
            ));
        }

        // Low confidence: ask the user
        if intent.confidence < self.config.confidence_threshold {
            return Decision::AskUser(
                "Which tool should I use here? Need a bit more context to pick the right one.".to_string(),
            );
        }

        // Determine the primary tool
        let primary_tool = intent.suggested_tools.first();

        match primary_tool {
            Some(tool_name) => {
                // Check confirmation requirement
                if self.should_confirm(tool_name) {
                    Decision::AskUser(format!(
                        "I'd like to use the `{}` tool. Shall I proceed?",
                        tool_name
                    ))
                } else {
                    Decision::ExecuteTool(tool_name.clone())
                }
            }
            None => {
                // No specific tool identified but intent says tool use
                // In Full mode, try to infer; otherwise ask
                match self.config.level {
                    AutonomyLevel::Full => Decision::RespondDirectly(
                        "Tool use detected but no specific tool identified".to_string(),
                    ),
                    _ => Decision::AskUser(
                        "Sounds like a tool call — which one are you thinking? Happy to run it once I know."
                            .to_string(),
                    ),
                }
            }
        }
    }
}

impl Default for AutonomyEngine {
    fn default() -> Self {
        Self::new(AutonomyConfig::default())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::{Intent, IntentAnalysis, TaskComplexity};

    /// Helper: create an IntentAnalysis with the given parameters.
    fn make_intent(
        intent: Intent,
        complexity: TaskComplexity,
        confidence: f32,
        tools: Vec<String>,
    ) -> IntentAnalysis {
        IntentAnalysis {
            intent,
            complexity,
            confidence,
            suggested_tools: tools,
            requires_confirmation: false,
            reasoning: "test".to_string(),
        }
    }

    /// Helper: create a basic DecisionContext.
    fn make_context(intent: IntentAnalysis) -> DecisionContext {
        DecisionContext {
            intent,
            has_memory_context: true,
            session_message_count: 5,
            recent_error_count: 0,
            available_tools: vec![
                "read_file".to_string(),
                "write_file".to_string(),
                "shell".to_string(),
            ],
            autonomous_tool_count: 0,
        }
    }

    #[test]
    fn test_decide_conversation() {
        let engine = AutonomyEngine::default();
        let intent = make_intent(Intent::Conversation, TaskComplexity::Trivial, 0.9, vec![]);
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert!(
            matches!(decision, Decision::RespondDirectly(_)),
            "Conversation should respond directly, got: {:?}",
            decision
        );
    }

    #[test]
    fn test_decide_simple_query_high_confidence() {
        let engine = AutonomyEngine::default();
        let intent = make_intent(Intent::SimpleQuery, TaskComplexity::Trivial, 0.85, vec![]);
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert!(
            matches!(decision, Decision::RespondDirectly(_)),
            "High-confidence simple query should respond directly, got: {:?}",
            decision
        );
    }

    #[test]
    fn test_decide_simple_query_low_confidence() {
        let engine = AutonomyEngine::default();
        let intent = make_intent(Intent::SimpleQuery, TaskComplexity::Trivial, 0.4, vec![]);
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert!(
            matches!(decision, Decision::AskUser(_)),
            "Low-confidence simple query should ask user, got: {:?}",
            decision
        );
    }

    #[test]
    fn test_decide_tool_use_allowed() {
        let engine = AutonomyEngine::default();
        // read_file is NOT in the default require_confirmation_for list
        let intent = make_intent(
            Intent::ToolUse,
            TaskComplexity::Simple,
            0.85,
            vec!["read_file".to_string()],
        );
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert_eq!(
            decision,
            Decision::ExecuteTool("read_file".to_string()),
            "read_file should be executed without confirmation"
        );
    }

    #[test]
    fn test_decide_tool_use_confirmation_required() {
        let engine = AutonomyEngine::default();
        // shell IS in the default require_confirmation_for list
        let intent = make_intent(
            Intent::ToolUse,
            TaskComplexity::Simple,
            0.85,
            vec!["shell".to_string()],
        );
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert!(
            matches!(decision, Decision::AskUser(_)),
            "shell should require confirmation in Supervised mode, got: {:?}",
            decision
        );
    }

    #[test]
    fn test_decide_tool_use_full_autonomy() {
        let config = AutonomyConfig {
            level: AutonomyLevel::Full,
            ..Default::default()
        };
        let engine = AutonomyEngine::new(config);
        // Even shell should proceed without confirmation in Full mode
        let intent = make_intent(
            Intent::ToolUse,
            TaskComplexity::Simple,
            0.85,
            vec!["shell".to_string()],
        );
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert_eq!(
            decision,
            Decision::ExecuteTool("shell".to_string()),
            "Full autonomy should execute shell without confirmation"
        );
    }

    #[test]
    fn test_decide_complex_task() {
        let engine = AutonomyEngine::default();
        let intent = make_intent(Intent::ComplexTask, TaskComplexity::Complex, 0.8, vec![]);
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert_eq!(
            decision,
            Decision::PlanAndExecute,
            "Complex tasks should trigger planning"
        );
    }

    #[test]
    fn test_decide_complex_task_low_confidence() {
        let engine = AutonomyEngine::default();
        let intent = make_intent(Intent::ComplexTask, TaskComplexity::Complex, 0.5, vec![]);
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert!(
            matches!(decision, Decision::AskUser(_)),
            "Low-confidence complex task should ask user, got: {:?}",
            decision
        );
    }

    #[test]
    fn test_decide_clarification() {
        let engine = AutonomyEngine::default();
        let intent = make_intent(Intent::Clarification, TaskComplexity::Trivial, 0.7, vec![]);
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert!(
            matches!(decision, Decision::AskUser(_)),
            "Clarification intent should ask user, got: {:?}",
            decision
        );
    }

    #[test]
    fn test_decide_system_command() {
        let engine = AutonomyEngine::default();
        let intent = make_intent(Intent::SystemCommand, TaskComplexity::Trivial, 0.95, vec![]);
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert!(
            matches!(decision, Decision::RespondDirectly(_)),
            "System commands should respond directly, got: {:?}",
            decision
        );
    }

    #[test]
    fn test_decide_with_errors_triggers_reflect() {
        let engine = AutonomyEngine::default();
        let intent = make_intent(
            Intent::ToolUse,
            TaskComplexity::Simple,
            0.9,
            vec!["read_file".to_string()],
        );
        let mut ctx = make_context(intent);
        ctx.recent_error_count = 5; // exceeds default threshold of 3

        let decision = engine.decide(&ctx);
        assert_eq!(
            decision,
            Decision::Reflect,
            "High error count should trigger reflection"
        );
    }

    #[test]
    fn test_should_confirm_shell() {
        let engine = AutonomyEngine::default();
        assert!(
            engine.should_confirm("shell"),
            "shell should require confirmation by default"
        );
    }

    #[test]
    fn test_should_confirm_write_file() {
        let engine = AutonomyEngine::default();
        assert!(
            engine.should_confirm("write_file"),
            "write_file should require confirmation by default"
        );
    }

    #[test]
    fn test_should_confirm_edit_file() {
        let engine = AutonomyEngine::default();
        assert!(
            engine.should_confirm("edit_file"),
            "edit_file should require confirmation by default"
        );
    }

    #[test]
    fn test_should_not_confirm_read_file() {
        let engine = AutonomyEngine::default();
        assert!(
            !engine.should_confirm("read_file"),
            "read_file should NOT require confirmation by default"
        );
    }

    #[test]
    fn test_should_confirm_restricted_mode() {
        let config = AutonomyConfig {
            level: AutonomyLevel::Restricted,
            ..Default::default()
        };
        let engine = AutonomyEngine::new(config);

        // In Restricted mode, ALL tools require confirmation
        assert!(engine.should_confirm("read_file"));
        assert!(engine.should_confirm("list_dir"));
        assert!(engine.should_confirm("web_fetch"));
    }

    #[test]
    fn test_autonomy_levels_display() {
        assert_eq!(format!("{}", AutonomyLevel::Full), "full");
        assert_eq!(format!("{}", AutonomyLevel::Supervised), "supervised");
        assert_eq!(format!("{}", AutonomyLevel::Restricted), "restricted");
    }

    #[test]
    fn test_default_config() {
        let config = AutonomyConfig::default();

        assert_eq!(config.level, AutonomyLevel::Supervised);
        assert!((config.confidence_threshold - 0.7).abs() < f32::EPSILON);
        assert_eq!(config.max_autonomous_tools, 5);
        assert_eq!(config.error_threshold, 3);
        assert_eq!(
            config.require_confirmation_for,
            vec![
                "shell".to_string(),
                "write_file".to_string(),
                "edit_file".to_string(),
            ]
        );
    }

    #[test]
    fn test_adjust_for_errors_no_errors() {
        let engine = AutonomyEngine::default();
        let decision = Decision::ExecuteTool("read_file".to_string());

        let adjusted = engine.adjust_for_errors(decision.clone(), 0);
        assert_eq!(adjusted, decision, "No errors should not change decision");
    }

    #[test]
    fn test_adjust_for_errors_below_threshold() {
        let engine = AutonomyEngine::default();
        // shell requires confirmation, so with 1 error it should ask
        let decision = Decision::ExecuteTool("shell".to_string());

        let adjusted = engine.adjust_for_errors(decision, 1);
        assert!(
            matches!(adjusted, Decision::AskUser(_)),
            "Errors on a confirmation-required tool should ask user, got: {:?}",
            adjusted
        );
    }

    #[test]
    fn test_adjust_for_errors_above_threshold() {
        let engine = AutonomyEngine::default();
        let decision = Decision::ExecuteTool("read_file".to_string());

        let adjusted = engine.adjust_for_errors(decision, 5);
        assert_eq!(
            adjusted,
            Decision::Reflect,
            "Errors above threshold should trigger reflection"
        );
    }

    #[test]
    fn test_adjust_for_errors_non_tool_decision() {
        let engine = AutonomyEngine::default();
        let decision = Decision::PlanAndExecute;

        let adjusted = engine.adjust_for_errors(decision.clone(), 1);
        assert_eq!(
            adjusted, decision,
            "Non-tool decisions with low errors should pass through"
        );
    }

    #[test]
    fn test_decide_tool_use_exceeds_limit() {
        let engine = AutonomyEngine::default();
        let intent = make_intent(
            Intent::ToolUse,
            TaskComplexity::Simple,
            0.9,
            vec!["read_file".to_string()],
        );
        let mut ctx = make_context(intent);
        ctx.autonomous_tool_count = 10; // exceeds max_autonomous_tools (5)

        let decision = engine.decide(&ctx);
        assert!(
            matches!(decision, Decision::AskUser(_)),
            "Exceeding tool limit should ask user, got: {:?}",
            decision
        );
    }

    #[test]
    fn test_decide_tool_use_no_tool_identified() {
        let engine = AutonomyEngine::default();
        let intent = make_intent(
            Intent::ToolUse,
            TaskComplexity::Simple,
            0.8,
            vec![], // no tools identified
        );
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert!(
            matches!(decision, Decision::AskUser(_)),
            "Tool use with no identified tool should ask user in Supervised mode, got: {:?}",
            decision
        );
    }

    #[test]
    fn test_decide_tool_use_no_tool_full_mode() {
        let config = AutonomyConfig {
            level: AutonomyLevel::Full,
            ..Default::default()
        };
        let engine = AutonomyEngine::new(config);
        let intent = make_intent(
            Intent::ToolUse,
            TaskComplexity::Simple,
            0.8,
            vec![], // no tools identified
        );
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert!(
            matches!(decision, Decision::RespondDirectly(_)),
            "Full mode with no tool should respond directly, got: {:?}",
            decision
        );
    }

    #[test]
    fn test_decision_serialization_roundtrip() {
        let decisions = vec![
            Decision::RespondDirectly("test".to_string()),
            Decision::ExecuteTool("shell".to_string()),
            Decision::PlanAndExecute,
            Decision::AskUser("question?".to_string()),
            Decision::Delegate("nous".to_string()),
            Decision::Reflect,
            Decision::SpawnAgents(vec![SpawnRequest {
                id: "test-1".to_string(),
                role: "worker".to_string(),
                task: "do stuff".to_string(),
                tools: vec!["shell".to_string()],
                system_prompt: None,
                capabilities: vec![],
                parallel: true,
                depends_on: vec![],
                depth: 0,
            }]),
        ];

        for decision in decisions {
            let json = serde_json::to_string(&decision).unwrap();
            let deserialized: Decision = serde_json::from_str(&json).unwrap();
            assert_eq!(
                deserialized, decision,
                "Roundtrip failed for {:?}",
                decision
            );
        }
    }

    #[test]
    fn test_decision_display() {
        assert_eq!(
            format!("{}", Decision::RespondDirectly("test".to_string())),
            "respond_directly(test)"
        );
        assert_eq!(
            format!("{}", Decision::ExecuteTool("shell".to_string())),
            "execute_tool(shell)"
        );
        assert_eq!(format!("{}", Decision::PlanAndExecute), "plan_and_execute");
        assert_eq!(format!("{}", Decision::Reflect), "reflect");
        assert_eq!(
            format!("{}", Decision::SpawnAgents(vec![])),
            "spawn_agents(0)"
        );
    }

    #[test]
    fn test_config_serialization() {
        let config = AutonomyConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: AutonomyConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.level, config.level);
        assert!(
            (deserialized.confidence_threshold - config.confidence_threshold).abs() < f32::EPSILON
        );
        assert_eq!(
            deserialized.max_autonomous_tools,
            config.max_autonomous_tools
        );
        assert_eq!(
            deserialized.require_confirmation_for,
            config.require_confirmation_for
        );
    }

    #[test]
    fn test_decide_tool_low_confidence() {
        let engine = AutonomyEngine::default();
        let intent = make_intent(
            Intent::ToolUse,
            TaskComplexity::Simple,
            0.3, // well below threshold
            vec!["read_file".to_string()],
        );
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert!(
            matches!(decision, Decision::AskUser(_)),
            "Low-confidence tool use should ask user, got: {:?}",
            decision
        );
    }

    #[test]
    fn test_decide_tool_use_web_fetch() {
        let engine = AutonomyEngine::default();
        // web_fetch is NOT in the default require_confirmation_for list
        let intent = make_intent(
            Intent::ToolUse,
            TaskComplexity::Simple,
            0.9,
            vec!["web_fetch".to_string()],
        );
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert_eq!(
            decision,
            Decision::ExecuteTool("web_fetch".to_string()),
            "web_fetch should execute without confirmation in Supervised mode"
        );
    }

    #[test]
    fn test_decide_tool_use_edit_file() {
        let engine = AutonomyEngine::default();
        // edit_file IS in the default require_confirmation_for list
        let intent = make_intent(
            Intent::ToolUse,
            TaskComplexity::Simple,
            0.9,
            vec!["edit_file".to_string()],
        );
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert!(
            matches!(decision, Decision::AskUser(_)),
            "edit_file should require confirmation in Supervised mode, got: {:?}",
            decision
        );
    }

    #[test]
    fn test_decide_multiple_error_types() {
        let engine = AutonomyEngine::default();

        // Test with error count exactly at threshold (3)
        let intent1 = make_intent(
            Intent::ToolUse,
            TaskComplexity::Simple,
            0.9,
            vec!["read_file".to_string()],
        );
        let mut ctx1 = make_context(intent1);
        ctx1.recent_error_count = 3; // exactly at threshold
        let decision1 = engine.decide(&ctx1);
        assert_eq!(decision1, Decision::Reflect);

        // Test with error count just below threshold (2)
        let intent2 = make_intent(
            Intent::ToolUse,
            TaskComplexity::Simple,
            0.9,
            vec!["read_file".to_string()],
        );
        let mut ctx2 = make_context(intent2);
        ctx2.recent_error_count = 2;
        let decision2 = engine.decide(&ctx2);
        // read_file doesn't require confirmation, so it should proceed
        assert_eq!(decision2, Decision::ExecuteTool("read_file".to_string()));
    }

    #[test]
    fn test_autonomy_config_custom() {
        let config = AutonomyConfig {
            level: AutonomyLevel::Full,
            confidence_threshold: 0.5,
            max_autonomous_tools: 100,
            require_confirmation_for: vec!["dangerous_tool".to_string()],
            error_threshold: 10,
        };
        let engine = AutonomyEngine::new(config);

        let cfg = engine.config();
        assert_eq!(cfg.level, AutonomyLevel::Full);
        assert!((cfg.confidence_threshold - 0.5).abs() < f32::EPSILON);
        assert_eq!(cfg.max_autonomous_tools, 100);
        assert_eq!(
            cfg.require_confirmation_for,
            vec!["dangerous_tool".to_string()]
        );
        assert_eq!(cfg.error_threshold, 10);

        // In Full mode, even dangerous_tool should not require confirmation
        assert!(!engine.should_confirm("dangerous_tool"));
        assert!(!engine.should_confirm("shell"));
    }

    #[test]
    fn test_decide_high_complexity_task() {
        let engine = AutonomyEngine::default();
        // High confidence complex task should go to PlanAndExecute
        let intent = make_intent(
            Intent::ComplexTask,
            TaskComplexity::Complex,
            0.95,
            vec!["shell".to_string(), "write_file".to_string()],
        );
        let ctx = make_context(intent);

        let decision = engine.decide(&ctx);
        assert_eq!(
            decision,
            Decision::PlanAndExecute,
            "High-confidence complex task should trigger planning"
        );

        // Verify that low confidence complex task asks user
        let intent_low = make_intent(
            Intent::ComplexTask,
            TaskComplexity::Complex,
            0.3,
            vec!["shell".to_string()],
        );
        let ctx_low = make_context(intent_low);
        let decision_low = engine.decide(&ctx_low);
        assert!(
            matches!(decision_low, Decision::AskUser(_)),
            "Low-confidence complex task should ask user, got: {:?}",
            decision_low
        );
    }
}
