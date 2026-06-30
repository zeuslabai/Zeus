//! Context overflow prevention and progressive summarization.
//!
//! Provides pre-emptive tool result capping, context budget tracking,
//! and progressive summarization to prevent hitting LLM context limits.

use serde::{Deserialize, Serialize};
use zeus_core::{Message, Role};

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for context overflow prevention.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverflowConfig {
    /// Maximum context window in tokens.
    pub max_context_tokens: usize,
    /// Maximum characters per single tool result before truncation.
    pub tool_result_cap_chars: usize,
    /// Percentage of capacity at which to start warning.
    pub warning_threshold_pct: u8,
    /// Percentage of capacity at which to start summarizing older messages.
    pub summarize_threshold_pct: u8,
    /// Percentage of capacity at which to force session rotation.
    pub critical_threshold_pct: u8,
    /// Number of recent messages to always preserve.
    pub keep_recent_messages: usize,
    /// Maximum characters for a summary block.
    pub summary_max_chars: usize,
}

impl Default for OverflowConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: zeus_core::MAX_CONTENT_BYTES,
            tool_result_cap_chars: zeus_core::TOOL_RESULT_CAP_CHARS,
            warning_threshold_pct: 80,
            summarize_threshold_pct: 85,
            critical_threshold_pct: 95,
            keep_recent_messages: 10,
            summary_max_chars: 2_000,
        }
    }
}

// ============================================================================
// Status Types
// ============================================================================

/// Current overflow status of the context window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverflowStatus {
    /// Under warning threshold -- no action needed.
    Normal,
    /// Between warning and summarize thresholds -- cap tool results.
    Warning,
    /// Between summarize and critical thresholds -- summarize older messages.
    Summarizing,
    /// Above critical threshold -- force session rotation.
    Critical,
}

/// Snapshot of the current context budget.
#[derive(Debug, Clone)]
pub struct ContextBudget {
    /// Total token capacity.
    pub total_tokens: usize,
    /// Estimated tokens currently used.
    pub used_tokens: usize,
    /// Current overflow status.
    pub status: OverflowStatus,
    /// Tokens remaining before limit.
    pub available_tokens: usize,
    /// Usage as a percentage (0.0 - 100.0).
    pub usage_pct: f64,
}

// ============================================================================
// Recovery Actions
// ============================================================================

/// Action to take in response to context overflow pressure.
#[derive(Debug, Clone, PartialEq)]
pub enum RecoveryAction {
    /// No action needed -- context is within safe bounds.
    None,
    /// Tool results should be capped to configured limit.
    CapToolResults,
    /// Older messages should be replaced with a summary.
    SummarizeOlder {
        /// Summary text for the older messages.
        summary: String,
        /// Index from which to keep messages (everything before is summarized).
        keep_from_index: usize,
    },
    /// Must rotate to a new session -- context is critically full.
    ForceRotation {
        /// Summary of the session being rotated out.
        summary: String,
    },
}

// ============================================================================
// OverflowRecovery
// ============================================================================

/// Engine for detecting and recovering from context overflow.
pub struct OverflowRecovery {
    config: OverflowConfig,
}

impl OverflowRecovery {
    /// Create a new recovery engine with the given configuration.
    pub fn new(config: OverflowConfig) -> Self {
        Self { config }
    }

    /// Estimate the token count for a text string.
    ///
    /// Uses the chars / 4 heuristic, matching the existing ContextManager.
    pub fn estimate_tokens(text: &str) -> usize {
        // Consistent with zeus-session ContextManager
        text.len() / 4
    }

    /// Estimate the total token count for a slice of messages.
    pub fn estimate_message_tokens(messages: &[Message]) -> usize {
        messages
            .iter()
            .map(|m| {
                let mut chars = m.content.len();
                for tc in &m.tool_calls {
                    chars += tc.name.len();
                    chars += tc.arguments.to_string().len();
                }
                for tr in &m.tool_results {
                    chars += tr.output.len();
                }
                Self::estimate_tokens_from_chars(chars)
            })
            .sum()
    }

    /// Check the current context budget for the given messages.
    pub fn check_budget(&self, messages: &[Message]) -> ContextBudget {
        let used_tokens = Self::estimate_message_tokens(messages);
        let total_tokens = self.config.max_context_tokens;
        let available_tokens = total_tokens.saturating_sub(used_tokens);
        let usage_pct = if total_tokens == 0 {
            100.0
        } else {
            (used_tokens as f64 / total_tokens as f64) * 100.0
        };

        let status = self.classify_usage(usage_pct);

        ContextBudget {
            total_tokens,
            used_tokens,
            status,
            available_tokens,
            usage_pct,
        }
    }

    /// Cap a tool result string if it exceeds the configured limit.
    ///
    /// Returns the original string if within limits, or a truncated version
    /// with a notice appended.
    pub fn cap_tool_result(&self, result: &str) -> String {
        let cap = self.config.tool_result_cap_chars;
        if result.len() <= cap {
            return result.to_string();
        }

        let safe = zeus_core::truncate_str(result, cap);
        let omitted = result.len() - safe.len();
        let mut truncated = safe.to_string();
        truncated.push_str(&format!("\n[truncated, {} chars omitted]", omitted));
        truncated
    }

    /// Build a summary of older messages, preserving the most recent ones.
    ///
    /// The summary captures:
    /// - Count of user and assistant messages
    /// - List of tool calls made
    /// - Key user requests and assistant decisions
    pub fn build_summary(messages: &[Message], keep_recent: usize) -> String {
        if messages.len() <= keep_recent {
            return String::new();
        }

        let older = &messages[..messages.len() - keep_recent];

        let mut user_count = 0usize;
        let mut assistant_count = 0usize;
        let mut tool_count = 0usize;
        let mut system_count = 0usize;
        let mut tool_names: Vec<String> = Vec::new();
        let mut key_requests: Vec<String> = Vec::new();
        let mut key_responses: Vec<String> = Vec::new();

        for msg in older {
            match msg.role {
                Role::User => {
                    user_count += 1;
                    // Capture a snippet of each user message as a key request
                    let snippet = truncate_str(&msg.content, 120);
                    if !snippet.is_empty() {
                        key_requests.push(snippet);
                    }
                }
                Role::Assistant => {
                    assistant_count += 1;
                    // Capture a snippet of each assistant response
                    let snippet = truncate_str(&msg.content, 120);
                    if !snippet.is_empty() {
                        key_responses.push(snippet);
                    }
                }
                Role::Tool => {
                    tool_count += 1;
                }
                Role::System => {
                    system_count += 1;
                }
            }

            for tc in &msg.tool_calls {
                if !tool_names.contains(&tc.name) {
                    tool_names.push(tc.name.clone());
                }
            }
        }

        let mut summary = String::new();
        summary.push_str("[Context Summary]\n");
        summary.push_str(&format!(
            "Summarized {} messages ({} user, {} assistant, {} tool, {} system).\n",
            older.len(),
            user_count,
            assistant_count,
            tool_count,
            system_count,
        ));

        if !tool_names.is_empty() {
            summary.push_str(&format!("Tools used: {}.\n", tool_names.join(", ")));
        }

        if !key_requests.is_empty() {
            summary.push_str("Key requests:\n");
            for (i, req) in key_requests.iter().enumerate() {
                if i >= 5 {
                    summary.push_str(&format!(
                        "  ... and {} more requests\n",
                        key_requests.len() - 5
                    ));
                    break;
                }
                summary.push_str(&format!("  - {}\n", req));
            }
        }

        if !key_responses.is_empty() {
            summary.push_str("Key responses:\n");
            for (i, resp) in key_responses.iter().enumerate() {
                if i >= 5 {
                    summary.push_str(&format!(
                        "  ... and {} more responses\n",
                        key_responses.len() - 5
                    ));
                    break;
                }
                summary.push_str(&format!("  - {}\n", resp));
            }
        }

        summary
    }

    /// Determine the recovery action to take based on current context usage.
    pub fn recover(&self, messages: &[Message]) -> RecoveryAction {
        let budget = self.check_budget(messages);

        match budget.status {
            OverflowStatus::Normal => RecoveryAction::None,
            OverflowStatus::Warning => RecoveryAction::CapToolResults,
            OverflowStatus::Summarizing => {
                let keep = self.config.keep_recent_messages.min(messages.len());
                let keep_from_index = messages.len().saturating_sub(keep);
                let summary = Self::build_summary(messages, keep);
                RecoveryAction::SummarizeOlder {
                    summary,
                    keep_from_index,
                }
            }
            OverflowStatus::Critical => {
                let summary = Self::build_summary(messages, 0);
                RecoveryAction::ForceRotation { summary }
            }
        }
    }

    // ---- Private helpers ----

    fn estimate_tokens_from_chars(chars: usize) -> usize {
        chars / 4
    }

    fn classify_usage(&self, usage_pct: f64) -> OverflowStatus {
        if usage_pct >= self.config.critical_threshold_pct as f64 {
            OverflowStatus::Critical
        } else if usage_pct >= self.config.summarize_threshold_pct as f64 {
            OverflowStatus::Summarizing
        } else if usage_pct >= self.config.warning_threshold_pct as f64 {
            OverflowStatus::Warning
        } else {
            OverflowStatus::Normal
        }
    }
}

/// Truncate a string to at most `max_len` characters, appending "..." if cut.
fn truncate_str(s: &str, max_len: usize) -> String {
    let trimmed = s.trim();
    if trimmed.len() <= max_len {
        trimmed.to_string()
    } else {
        let end = zeus_core::floor_char_boundary(trimmed, max_len.saturating_sub(3));
        format!("{}...", &trimmed[..end])
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use zeus_core::{Message, ToolCall};

    // -- OverflowConfig tests --

    #[test]
    fn test_config_defaults() {
        let cfg = OverflowConfig::default();
        assert_eq!(cfg.max_context_tokens, 100_000);
        assert_eq!(cfg.tool_result_cap_chars, 50_000);
        assert_eq!(cfg.warning_threshold_pct, 80);
        assert_eq!(cfg.summarize_threshold_pct, 85);
        assert_eq!(cfg.critical_threshold_pct, 95);
        assert_eq!(cfg.keep_recent_messages, 10);
        assert_eq!(cfg.summary_max_chars, 2_000);
    }

    #[test]
    fn test_config_serialization_roundtrip() {
        let cfg = OverflowConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: OverflowConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.max_context_tokens, cfg.max_context_tokens);
        assert_eq!(
            deserialized.tool_result_cap_chars,
            cfg.tool_result_cap_chars
        );
        assert_eq!(
            deserialized.warning_threshold_pct,
            cfg.warning_threshold_pct
        );
        assert_eq!(
            deserialized.summarize_threshold_pct,
            cfg.summarize_threshold_pct
        );
        assert_eq!(
            deserialized.critical_threshold_pct,
            cfg.critical_threshold_pct
        );
        assert_eq!(deserialized.keep_recent_messages, cfg.keep_recent_messages);
        assert_eq!(deserialized.summary_max_chars, cfg.summary_max_chars);
    }

    // -- estimate_tokens tests --

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(OverflowRecovery::estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_tokens_short() {
        // 12 chars / 4 = 3 tokens
        assert_eq!(OverflowRecovery::estimate_tokens("hello world!"), 3);
    }

    #[test]
    fn test_estimate_tokens_long() {
        let text = "a".repeat(4000);
        assert_eq!(OverflowRecovery::estimate_tokens(&text), 1000);
    }

    // -- estimate_message_tokens tests --

    #[test]
    fn test_estimate_message_tokens_multiple() {
        let messages = vec![
            Message::user("hello world"),          // 11 chars -> 2 tokens
            Message::assistant("hi there friend"), // 15 chars -> 3 tokens
        ];
        let tokens = OverflowRecovery::estimate_message_tokens(&messages);
        assert_eq!(tokens, 2 + 3);
    }

    #[test]
    fn test_estimate_message_tokens_with_tool_calls() {
        let msg = Message::assistant("response").with_tool_calls(vec![ToolCall {
            id: "tc_1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "/tmp/test.txt"}),
        }]);
        let tokens = OverflowRecovery::estimate_message_tokens(&[msg]);
        // "response" (8) + "read_file" (9) + json args string length
        assert!(tokens > 2); // should include tool call overhead
    }

    #[test]
    fn test_estimate_message_tokens_with_tool_results() {
        let msg = Message::tool("tc_1", true, "file contents here");
        let tokens = OverflowRecovery::estimate_message_tokens(&[msg]);
        // content is empty, but tool_results[0].output = "file contents here" (18 chars) -> 4 tokens
        assert_eq!(tokens, 4);
    }

    // -- check_budget tests --

    #[test]
    fn test_check_budget_normal() {
        let cfg = OverflowConfig {
            max_context_tokens: 1000,
            ..Default::default()
        };
        let recovery = OverflowRecovery::new(cfg);
        // 100 chars -> 25 tokens -> 2.5% of 1000
        let messages = vec![Message::user("a".repeat(100))];
        let budget = recovery.check_budget(&messages);
        assert_eq!(budget.status, OverflowStatus::Normal);
        assert_eq!(budget.total_tokens, 1000);
        assert_eq!(budget.used_tokens, 25);
        assert_eq!(budget.available_tokens, 975);
        assert!((budget.usage_pct - 2.5).abs() < 0.1);
    }

    #[test]
    fn test_check_budget_warning() {
        let cfg = OverflowConfig {
            max_context_tokens: 100,
            warning_threshold_pct: 80,
            summarize_threshold_pct: 90,
            ..Default::default()
        };
        let recovery = OverflowRecovery::new(cfg);
        // 328 chars -> 82 tokens -> 82% of 100 (between 80 warning and 90 summarize)
        let messages = vec![Message::user("a".repeat(328))];
        let budget = recovery.check_budget(&messages);
        assert_eq!(budget.status, OverflowStatus::Warning);
    }

    #[test]
    fn test_check_budget_summarizing() {
        let cfg = OverflowConfig {
            max_context_tokens: 100,
            warning_threshold_pct: 80,
            summarize_threshold_pct: 85,
            ..Default::default()
        };
        let recovery = OverflowRecovery::new(cfg);
        // 360 chars -> 90 tokens -> 90% of 100
        let messages = vec![Message::user("a".repeat(360))];
        let budget = recovery.check_budget(&messages);
        assert_eq!(budget.status, OverflowStatus::Summarizing);
    }

    #[test]
    fn test_check_budget_critical() {
        let cfg = OverflowConfig {
            max_context_tokens: 100,
            critical_threshold_pct: 95,
            ..Default::default()
        };
        let recovery = OverflowRecovery::new(cfg);
        // 400 chars -> 100 tokens -> 100% of 100
        let messages = vec![Message::user("a".repeat(400))];
        let budget = recovery.check_budget(&messages);
        assert_eq!(budget.status, OverflowStatus::Critical);
    }

    // -- cap_tool_result tests --

    #[test]
    fn test_cap_tool_result_under_limit() {
        let cfg = OverflowConfig {
            tool_result_cap_chars: 100,
            ..Default::default()
        };
        let recovery = OverflowRecovery::new(cfg);
        let input = "short result";
        let output = recovery.cap_tool_result(input);
        assert_eq!(output, input);
    }

    #[test]
    fn test_cap_tool_result_over_limit() {
        let cfg = OverflowConfig {
            tool_result_cap_chars: 20,
            ..Default::default()
        };
        let recovery = OverflowRecovery::new(cfg);
        let input = "a".repeat(50);
        let output = recovery.cap_tool_result(&input);
        assert!(output.starts_with(&"a".repeat(20)));
        assert!(output.contains("[truncated, 30 chars omitted]"));
        assert!(output.len() < input.len() + 50); // capped + notice is shorter than uncapped
    }

    // -- build_summary tests --

    #[test]
    fn test_build_summary_meaningful_output() {
        let messages = vec![
            Message::user("Please read the config file"),
            Message::assistant("I'll read that file for you").with_tool_calls(vec![ToolCall {
                id: "tc_1".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({"path": "config.toml"}),
            }]),
            Message::tool("tc_1", true, "model = 'gpt-4'"),
            Message::assistant("The config file contains model = gpt-4"),
            Message::user("Now update the model to claude"),
            // These two are the "recent" ones we keep
            Message::user("Thanks for your help"),
            Message::assistant("You're welcome!"),
        ];

        let summary = OverflowRecovery::build_summary(&messages, 2);
        assert!(summary.contains("[Context Summary]"));
        assert!(summary.contains("user"));
        assert!(summary.contains("assistant"));
        assert!(summary.contains("read_file"));
        assert!(summary.contains("read the config file"));
    }

    #[test]
    fn test_build_summary_no_older_messages() {
        let messages = vec![Message::user("hello"), Message::assistant("hi")];
        let summary = OverflowRecovery::build_summary(&messages, 5);
        assert!(summary.is_empty());
    }

    // -- recover tests --

    #[test]
    fn test_recover_none_under_threshold() {
        let cfg = OverflowConfig {
            max_context_tokens: 10_000,
            ..Default::default()
        };
        let recovery = OverflowRecovery::new(cfg);
        let messages = vec![Message::user("short message")];
        let action = recovery.recover(&messages);
        assert_eq!(action, RecoveryAction::None);
    }

    #[test]
    fn test_recover_cap_tool_results_at_warning() {
        let cfg = OverflowConfig {
            max_context_tokens: 100,
            warning_threshold_pct: 80,
            summarize_threshold_pct: 90,
            ..Default::default()
        };
        let recovery = OverflowRecovery::new(cfg);
        // 340 chars -> 85 tokens -> 85% (between 80 warning and 90 summarize)
        let messages = vec![Message::user("a".repeat(340))];
        let action = recovery.recover(&messages);
        assert_eq!(action, RecoveryAction::CapToolResults);
    }

    #[test]
    fn test_recover_summarize_older_at_summarize() {
        let cfg = OverflowConfig {
            max_context_tokens: 100,
            warning_threshold_pct: 70,
            summarize_threshold_pct: 80,
            critical_threshold_pct: 95,
            keep_recent_messages: 2,
            ..Default::default()
        };
        let recovery = OverflowRecovery::new(cfg);
        // Each message ~25 tokens, 4 messages ~= 100 tokens total -> could hit summarize
        // We need ~88% usage: 88 tokens from 100, so 352 chars total
        // 3 messages: 120 + 120 + 112 = 352 chars -> 88 tokens -> 88%
        let messages = vec![
            Message::user("b".repeat(120)),
            Message::assistant("c".repeat(120)),
            Message::user("d".repeat(112)),
        ];
        let action = recovery.recover(&messages);
        match action {
            RecoveryAction::SummarizeOlder {
                summary,
                keep_from_index,
            } => {
                assert!(!summary.is_empty());
                assert_eq!(keep_from_index, 1); // keep last 2
            }
            other => panic!("Expected SummarizeOlder, got {:?}", other),
        }
    }

    #[test]
    fn test_recover_force_rotation_at_critical() {
        let cfg = OverflowConfig {
            max_context_tokens: 100,
            critical_threshold_pct: 95,
            ..Default::default()
        };
        let recovery = OverflowRecovery::new(cfg);
        // 400 chars -> 100 tokens -> 100% of 100 -> Critical
        let messages = vec![Message::user("x".repeat(400))];
        let action = recovery.recover(&messages);
        match action {
            RecoveryAction::ForceRotation { summary } => {
                assert!(summary.contains("[Context Summary]"));
            }
            other => panic!("Expected ForceRotation, got {:?}", other),
        }
    }

    // -- OverflowStatus serialization --

    #[test]
    fn test_overflow_status_serialization() {
        let statuses = vec![
            OverflowStatus::Normal,
            OverflowStatus::Warning,
            OverflowStatus::Summarizing,
            OverflowStatus::Critical,
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: OverflowStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, status);
        }
    }

    // -- ContextBudget calculation accuracy --

    #[test]
    fn test_context_budget_calculation_accuracy() {
        let cfg = OverflowConfig {
            max_context_tokens: 500,
            ..Default::default()
        };
        let recovery = OverflowRecovery::new(cfg);
        // 800 chars -> 200 tokens -> 40% of 500
        let messages = vec![Message::user("a".repeat(800))];
        let budget = recovery.check_budget(&messages);
        assert_eq!(budget.total_tokens, 500);
        assert_eq!(budget.used_tokens, 200);
        assert_eq!(budget.available_tokens, 300);
        assert!((budget.usage_pct - 40.0).abs() < 0.01);
        assert_eq!(budget.status, OverflowStatus::Normal);
    }

    // -- RecoveryAction variants --

    #[test]
    fn test_recovery_action_none_variant() {
        let action = RecoveryAction::None;
        assert_eq!(action, RecoveryAction::None);
    }

    #[test]
    fn test_recovery_action_cap_variant() {
        let action = RecoveryAction::CapToolResults;
        assert_eq!(action, RecoveryAction::CapToolResults);
    }

    #[test]
    fn test_recovery_action_summarize_variant() {
        let action = RecoveryAction::SummarizeOlder {
            summary: "test summary".to_string(),
            keep_from_index: 5,
        };
        match action {
            RecoveryAction::SummarizeOlder {
                summary,
                keep_from_index,
            } => {
                assert_eq!(summary, "test summary");
                assert_eq!(keep_from_index, 5);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_recovery_action_force_rotation_variant() {
        let action = RecoveryAction::ForceRotation {
            summary: "session summary".to_string(),
        };
        match action {
            RecoveryAction::ForceRotation { summary } => {
                assert_eq!(summary, "session summary");
            }
            _ => panic!("wrong variant"),
        }
    }

    // -- Edge case: zero max_context_tokens --

    #[test]
    fn test_check_budget_zero_max_tokens() {
        let cfg = OverflowConfig {
            max_context_tokens: 0,
            ..Default::default()
        };
        let recovery = OverflowRecovery::new(cfg);
        let messages = vec![Message::user("anything")];
        let budget = recovery.check_budget(&messages);
        assert_eq!(budget.status, OverflowStatus::Critical);
        assert!((budget.usage_pct - 100.0).abs() < 0.01);
    }
}
