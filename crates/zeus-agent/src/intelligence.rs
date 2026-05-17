//! Agent Intelligence — Loop detection and context window guard
//!
//! Two safety mechanisms for the agent loop:
//! - LoopDetector: catches repetitive tool calls (same tool N times in a row)
//! - ContextGuard: estimates token usage and truncates when approaching limits

use tracing::{info, warn};
use zeus_core::Message;

// ============================================================================
// LoopDetector
// ============================================================================

/// Detects when the LLM is stuck calling the same tool repeatedly.
///
/// Tracks consecutive calls to the same tool name. When the count reaches
/// the threshold, returns a warning message to inject into the conversation.
#[derive(Debug, Clone)]
pub struct LoopDetector {
    /// How many consecutive same-tool calls before triggering
    threshold: usize,
    /// Current tool being tracked
    current_tool: Option<String>,
    /// Consecutive count for current tool
    consecutive_count: usize,
}

impl LoopDetector {
    /// Create a new loop detector with the given threshold.
    pub fn new(threshold: usize) -> Self {
        Self {
            threshold: threshold.max(2), // minimum 2 to be meaningful
            current_tool: None,
            consecutive_count: 0,
        }
    }

    /// Default threshold of 5 consecutive calls.
    pub fn default_threshold() -> Self {
        Self::new(5)
    }

    /// Record a tool call. Returns Some(warning_message) if the loop
    /// threshold has been reached, None otherwise.
    pub fn record_call(&mut self, tool_name: &str) -> Option<String> {
        match &self.current_tool {
            Some(current) if current == tool_name => {
                self.consecutive_count += 1;
            }
            _ => {
                self.current_tool = Some(tool_name.to_string());
                self.consecutive_count = 1;
            }
        }

        if self.consecutive_count >= self.threshold {
            let count = self.consecutive_count;
            let name = tool_name.to_string();
            warn!(
                "Tool loop detected: '{}' called {} consecutive times (threshold: {})",
                name, count, self.threshold
            );
            // Reset so we don't fire every single call after threshold
            self.reset();
            Some(format!(
                "WARNING: You have called the tool '{}' {} times consecutively without making progress. \
                 This appears to be a loop. Please try a different approach: \
                 use a different tool, modify your arguments, or explain what you're trying to accomplish.",
                name, count
            ))
        } else {
            None
        }
    }

    /// Reset the detector state.
    pub fn reset(&mut self) {
        self.current_tool = None;
        self.consecutive_count = 0;
    }

    /// Current consecutive count for the tracked tool.
    pub fn consecutive_count(&self) -> usize {
        self.consecutive_count
    }

    /// The currently tracked tool name.
    pub fn current_tool(&self) -> Option<&str> {
        self.current_tool.as_deref()
    }

    /// The configured threshold.
    pub fn threshold(&self) -> usize {
        self.threshold
    }
}

// ============================================================================
// ContextGuard
// ============================================================================

/// Estimates token count of messages and truncates when approaching the
/// model's context window limit.
///
/// Uses a simple heuristic: ~4 characters per token (conservative estimate).
/// When the estimated token count exceeds `threshold_pct` of `max_tokens`,
/// the oldest non-system messages are removed.
#[derive(Debug, Clone)]
pub struct ContextGuard {
    /// Maximum tokens for the model's context window
    max_tokens: usize,
    /// Threshold percentage (0.0 - 1.0) at which to start truncating
    threshold_pct: f32,
}

impl ContextGuard {
    /// Create a new context guard.
    ///
    /// - `max_tokens`: model's maximum context window size
    /// - `threshold_pct`: fraction (0.0–1.0) of max_tokens that triggers truncation
    pub fn new(max_tokens: usize, threshold_pct: f32) -> Self {
        Self {
            max_tokens: max_tokens.max(1000),
            threshold_pct: threshold_pct.clamp(0.1, 0.99),
        }
    }

    /// Default: 200k context, 80% threshold.
    pub fn default_guard() -> Self {
        Self::new(200_000, 0.8)
    }

    /// Estimate token count for a slice of messages.
    ///
    /// Uses ~4 chars/token heuristic. This is intentionally conservative
    /// (overestimates) to avoid hitting true limits.
    pub fn estimate_tokens(messages: &[Message]) -> usize {
        let total_chars: usize = messages
            .iter()
            .map(|m| {
                let content_len = m.content.len();
                let tool_len: usize = m
                    .tool_calls
                    .iter()
                    .map(|tc| tc.name.len() + tc.arguments.to_string().len())
                    .sum();
                let result_len: usize = m.tool_results.iter().map(|tr| tr.output.len()).sum();
                content_len + tool_len + result_len
            })
            .sum();
        // ~4 chars per token (conservative)
        total_chars / 4
    }

    /// Check if messages exceed the threshold and truncate if needed.
    ///
    /// Returns the number of messages removed. System messages (role == System)
    /// are never removed. The most recent messages are preserved; oldest
    /// non-system messages are dropped first.
    ///
    /// Pair-safe: tool_calls (tool_use) and tool_results (tool_result) are
    /// always removed together to avoid orphaned blocks that cause API 400s.
    pub fn guard(&self, messages: &mut Vec<Message>) -> usize {
        let threshold = (self.max_tokens as f32 * self.threshold_pct) as usize;
        let mut current_estimate = Self::estimate_tokens(messages);

        if current_estimate <= threshold {
            return 0;
        }

        info!(
            "Context guard: estimated {} tokens exceeds threshold {} ({}% of {}). Truncating.",
            current_estimate,
            threshold,
            (self.threshold_pct * 100.0) as u32,
            self.max_tokens
        );

        let mut removed = 0;
        // O(n): pre-computed estimate, subtract per removal instead of re-scanning.
        // Pair-safe: tool_calls/tool_results removed together.
        while current_estimate > threshold && messages.len() > 1 {
            let idx = match messages
                .iter()
                .position(|m| m.role != zeus_core::Role::System)
            {
                Some(i) => i,
                None => break,
            };

            let has_tool_calls = !messages[idx].tool_calls.is_empty();
            let has_tool_results = !messages[idx].tool_results.is_empty();

            current_estimate = current_estimate
                .saturating_sub(Self::estimate_tokens(&messages[idx..idx + 1]));
            messages.remove(idx);
            removed += 1;

            // Pair-safe: removed assistant with tool_calls → also remove the
            // paired tool_results message (now shifted to same idx).
            if has_tool_calls
                && idx < messages.len()
                && !messages[idx].tool_results.is_empty()
            {
                current_estimate = current_estimate
                    .saturating_sub(Self::estimate_tokens(&messages[idx..idx + 1]));
                messages.remove(idx);
                removed += 1;
            }
            // Pair-safe: removed tool_results → also remove the preceding
            // assistant message with the paired tool_calls.
            else if has_tool_results
                && idx > 0
                && !messages[idx - 1].tool_calls.is_empty()
                && messages[idx - 1].role != zeus_core::Role::System
            {
                current_estimate = current_estimate
                    .saturating_sub(Self::estimate_tokens(&messages[idx - 1..idx]));
                messages.remove(idx - 1);
                removed += 1;
            }
        }

        if removed > 0 {
            // Post-truncation repair: fix any orphaned tool_use blocks that
            // would cause Anthropic API 400 errors.
            Self::repair_orphaned_tool_calls(messages);

            let note = Message::system(format!(
                "[Context truncated: {} older messages removed to stay within context window. \
                 Some earlier conversation history is no longer available.]",
                removed
            ));
            let insert_pos = messages
                .iter()
                .position(|m| m.role != zeus_core::Role::System)
                .unwrap_or(messages.len());
            // Count truncation note's tokens in the running estimate
            current_estimate += Self::estimate_tokens(std::slice::from_ref(&note));
            messages.insert(insert_pos, note);

            info!(
                "Context guard: removed {} messages, ~{} tokens remaining",
                removed, current_estimate
            );
        }

        removed
    }

    /// Repair orphaned tool_use blocks after truncation.
    ///
    /// Delegates to `zeus_session::repair_orphaned_tool_calls` which is the
    /// canonical implementation (also called at Session::load time).
    fn repair_orphaned_tool_calls(messages: &mut Vec<Message>) {
        zeus_session::repair_orphaned_tool_calls(messages, None);
    }

    /// The configured max tokens.
    pub fn max_tokens(&self) -> usize {
        self.max_tokens
    }

    /// The configured threshold percentage.
    pub fn threshold_pct(&self) -> f32 {
        self.threshold_pct
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use zeus_core::Message;

    // ---- LoopDetector tests ----

    #[test]
    fn test_loop_detector_no_loop() {
        let mut det = LoopDetector::new(5);
        assert!(det.record_call("shell").is_none());
        assert!(det.record_call("read_file").is_none());
        assert!(det.record_call("shell").is_none());
        assert!(det.record_call("write_file").is_none());
        assert_eq!(det.consecutive_count(), 1);
    }

    #[test]
    fn test_loop_detector_triggers_at_threshold() {
        let mut det = LoopDetector::new(3);
        assert!(det.record_call("shell").is_none());
        assert!(det.record_call("shell").is_none());
        let warning = det.record_call("shell");
        assert!(warning.is_some());
        assert!(warning.unwrap().contains("shell"));
    }

    #[test]
    fn test_loop_detector_resets_on_different_tool() {
        let mut det = LoopDetector::new(3);
        assert!(det.record_call("shell").is_none());
        assert!(det.record_call("shell").is_none());
        // Different tool resets counter
        assert!(det.record_call("read_file").is_none());
        assert_eq!(det.consecutive_count(), 1);
        assert_eq!(det.current_tool(), Some("read_file"));
    }

    #[test]
    fn test_loop_detector_resets_after_trigger() {
        let mut det = LoopDetector::new(3);
        det.record_call("shell");
        det.record_call("shell");
        let warning = det.record_call("shell");
        assert!(warning.is_some());
        // After trigger, state is reset
        assert_eq!(det.consecutive_count(), 0);
        assert!(det.current_tool().is_none());
    }

    #[test]
    fn test_loop_detector_default_threshold() {
        let det = LoopDetector::default_threshold();
        assert_eq!(det.threshold(), 5);
    }

    #[test]
    fn test_loop_detector_minimum_threshold() {
        let det = LoopDetector::new(1);
        // Minimum is 2
        assert_eq!(det.threshold(), 2);
    }

    #[test]
    fn test_loop_detector_manual_reset() {
        let mut det = LoopDetector::new(5);
        det.record_call("shell");
        det.record_call("shell");
        det.record_call("shell");
        assert_eq!(det.consecutive_count(), 3);
        det.reset();
        assert_eq!(det.consecutive_count(), 0);
        assert!(det.current_tool().is_none());
    }

    #[test]
    fn test_loop_detector_exact_threshold() {
        let mut det = LoopDetector::new(5);
        for _ in 0..4 {
            assert!(det.record_call("web_fetch").is_none());
        }
        // 5th call triggers
        let warning = det.record_call("web_fetch");
        assert!(warning.is_some());
        let msg = warning.unwrap();
        assert!(msg.contains("web_fetch"));
        assert!(msg.contains("5 times"));
    }

    #[test]
    fn test_loop_detector_warning_message_content() {
        let mut det = LoopDetector::new(2);
        det.record_call("list_dir");
        let warning = det.record_call("list_dir").unwrap();
        assert!(warning.contains("list_dir"));
        assert!(warning.contains("different approach"));
    }

    // ---- ContextGuard tests ----

    #[test]
    fn test_context_guard_estimate_tokens() {
        let messages = vec![
            Message::user("Hello world"), // 11 chars => ~2 tokens
        ];
        let tokens = ContextGuard::estimate_tokens(&messages);
        assert_eq!(tokens, 11 / 4); // 2
    }

    #[test]
    fn test_context_guard_no_truncation_under_threshold() {
        let guard = ContextGuard::new(10000, 0.8);
        let mut messages = vec![
            Message::system("You are a helpful assistant."),
            Message::user("Hello"),
            Message::assistant("Hi there!"),
        ];
        let removed = guard.guard(&mut messages);
        assert_eq!(removed, 0);
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn test_context_guard_truncates_when_over_threshold() {
        // min 1000 tokens = 4000 chars threshold at 50% = 2000 chars
        let guard = ContextGuard::new(1000, 0.5);
        let mut messages = vec![
            Message::system("System prompt."),
            Message::user(&"x".repeat(3000)),
            Message::assistant(&"y".repeat(3000)),
            Message::user(&"z".repeat(3000)),
        ];
        let removed = guard.guard(&mut messages);
        assert!(removed > 0);
        // System messages should be preserved
        assert!(messages.iter().any(|m| m.role == zeus_core::Role::System));
    }

    #[test]
    fn test_context_guard_preserves_system_messages() {
        let guard = ContextGuard::new(1000, 0.5);
        let mut messages = vec![
            Message::system("Important system context"),
            Message::user(&"a".repeat(5000)),
        ];
        let removed = guard.guard(&mut messages);
        assert!(removed > 0);
        // System message must survive
        assert!(
            messages
                .iter()
                .any(|m| m.role == zeus_core::Role::System && m.content.contains("Important"))
        );
    }

    #[test]
    fn test_context_guard_injects_truncation_note() {
        let guard = ContextGuard::new(1000, 0.5);
        let mut messages = vec![
            Message::system("sys"),
            Message::user(&"x".repeat(3000)),
            Message::assistant(&"y".repeat(3000)),
            Message::user("recent"),
        ];
        let removed = guard.guard(&mut messages);
        assert!(removed > 0);
        assert!(
            messages
                .iter()
                .any(|m| m.content.contains("Context truncated"))
        );
    }

    #[test]
    fn test_context_guard_default() {
        let guard = ContextGuard::default_guard();
        assert_eq!(guard.max_tokens(), 200_000);
        assert!((guard.threshold_pct() - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_context_guard_clamps_values() {
        let guard = ContextGuard::new(500, 1.5); // over 1.0
        assert!((guard.threshold_pct() - 0.99).abs() < f32::EPSILON);

        let guard2 = ContextGuard::new(500, 0.01); // under 0.1
        assert!((guard2.threshold_pct() - 0.1).abs() < f32::EPSILON);

        let guard3 = ContextGuard::new(0, 0.5); // under minimum
        assert_eq!(guard3.max_tokens(), 1000);
    }

    #[test]
    fn test_context_guard_empty_messages() {
        let guard = ContextGuard::new(1000, 0.8);
        let mut messages: Vec<Message> = vec![];
        let removed = guard.guard(&mut messages);
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_context_guard_only_system_messages() {
        let guard = ContextGuard::new(10, 0.5);
        let mut messages = vec![
            Message::system(&"a".repeat(500)),
            Message::system(&"b".repeat(500)),
        ];
        let original_len = messages.len();
        let removed = guard.guard(&mut messages);
        assert_eq!(removed, 0);
        assert_eq!(messages.len(), original_len);
    }

    #[test]
    fn test_context_guard_pair_safe_tool_calls() {
        // Removing an assistant with tool_calls must also remove
        // the paired tool_results to avoid orphaned tool_result (API 400).
        let guard = ContextGuard::new(1000, 0.5);
        let mut assistant_msg = Message::assistant(&"x".repeat(2000));
        assistant_msg.tool_calls.push(zeus_core::ToolCall {
            id: "tc_pair".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "echo hello"}),
        });
        let tool_msg = Message {
            role: zeus_core::Role::User,
            content: String::new(),
            tool_calls: vec![],
            tool_results: vec![zeus_core::ToolResult {
                call_id: "tc_pair".to_string(),
                success: true,
                output: "y".repeat(2000),
            }],
            timestamp: chrono::Utc::now(),
            attachments: vec![],
            message_id: None,
            parent_id: None,
            thread_id: None,
            direction: Default::default(),
            channel_source: None,
                compaction_hint: Default::default(),
        };
        let mut messages = vec![
            Message::system("sys"),
            assistant_msg,
            tool_msg,
            Message::user("recent question"),
        ];
        let removed = guard.guard(&mut messages);
        assert!(removed >= 2, "Expected pair removal, got {}", removed);
        // No orphaned tool_results without a prior tool_calls
        for (i, m) in messages.iter().enumerate() {
            if !m.tool_results.is_empty() {
                assert!(
                    i > 0 && !messages[i - 1].tool_calls.is_empty(),
                    "Orphaned tool_results at index {}",
                    i
                );
            }
        }
    }

    #[test]
    fn test_context_guard_pair_safe_tool_results_first() {
        // If a user message is removed first, then the next oldest is an
        // assistant+tool_calls pair — both must be removed together.
        let guard = ContextGuard::new(1000, 0.5);
        let mut assistant_msg = Message::assistant(&"a".repeat(1500));
        assistant_msg.tool_calls.push(zeus_core::ToolCall {
            id: "tc2".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "/tmp/test"}),
        });
        let tool_msg = Message {
            role: zeus_core::Role::User,
            content: String::new(),
            tool_calls: vec![],
            tool_results: vec![zeus_core::ToolResult {
                call_id: "tc2".to_string(),
                success: true,
                output: "b".repeat(1500),
            }],
            timestamp: chrono::Utc::now(),
            attachments: vec![],
            message_id: None,
            parent_id: None,
            thread_id: None,
            direction: Default::default(),
            channel_source: None,
                compaction_hint: Default::default(),
        };
        let mut messages = vec![
            Message::system("sys"),
            Message::user(&"u".repeat(1500)),
            assistant_msg,
            tool_msg,
            Message::user("keep me"),
        ];
        let removed = guard.guard(&mut messages);
        assert!(removed > 0);
        // Verify no orphans
        for (i, m) in messages.iter().enumerate() {
            if !m.tool_results.is_empty() {
                assert!(
                    i > 0 && !messages[i - 1].tool_calls.is_empty(),
                    "Orphaned tool_results at index {}",
                    i
                );
            }
            if !m.tool_calls.is_empty() {
                assert!(
                    i + 1 < messages.len() && !messages[i + 1].tool_results.is_empty(),
                    "Orphaned tool_calls at index {}",
                    i
                );
            }
        }
    }

    #[test]
    fn test_estimate_tokens_with_tool_calls() {
        let mut msg = Message::user("hello");
        msg.tool_calls.push(zeus_core::ToolCall {
            id: "tc1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "ls -la /some/path"}),
        });
        let tokens = ContextGuard::estimate_tokens(&[msg]);
        // Should include tool call content in estimate
        assert!(tokens > 1);
    }

    #[test]
    fn test_estimate_tokens_with_tool_results() {
        let msg = Message {
            role: zeus_core::Role::Tool,
            content: String::new(),
            tool_calls: vec![],
            tool_results: vec![zeus_core::ToolResult {
                call_id: "tc1".to_string(),
                success: true,
                output: "file1.txt\nfile2.txt\nfile3.txt".to_string(),
            }],
            timestamp: chrono::Utc::now(),
            attachments: vec![],
            message_id: None,
            parent_id: None,
            thread_id: None,
            direction: Default::default(),
            channel_source: None,
                compaction_hint: Default::default(),
        };
        let tokens = ContextGuard::estimate_tokens(&[msg]);
        assert!(tokens > 0);
    }

    #[test]
    fn test_repair_orphaned_tool_calls_no_tool_msg() {
        // Assistant with tool_calls but no following tool message
        let mut messages = vec![
            Message::system("sys".to_string()),
            Message {
                role: zeus_core::Role::Assistant,
                content: "calling tool".to_string(),
                tool_calls: vec![zeus_core::ToolCall {
                    id: "tc_orphan".to_string(),
                    name: "shell".to_string(),
                    arguments: Default::default(),
                }],
                tool_results: vec![],
                timestamp: chrono::Utc::now(),
                attachments: vec![],
                message_id: None,
                parent_id: None,
                thread_id: None,
                direction: Default::default(),
            channel_source: None,
                compaction_hint: Default::default(),
            },
            Message {
                role: zeus_core::Role::User,
                content: "next msg".to_string(),
                tool_calls: vec![],
                tool_results: vec![],
                timestamp: chrono::Utc::now(),
                attachments: vec![],
                message_id: None,
                parent_id: None,
                thread_id: None,
                direction: Default::default(),
            channel_source: None,
                compaction_hint: Default::default(),
            },
        ];
        ContextGuard::repair_orphaned_tool_calls(&mut messages);
        // Should inject a synthetic tool message after the assistant
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[2].role, zeus_core::Role::Tool);
        assert_eq!(messages[2].tool_results.len(), 1);
        assert_eq!(messages[2].tool_results[0].call_id, "tc_orphan");
        assert!(!messages[2].tool_results[0].success);
    }

    #[test]
    fn test_repair_orphaned_tool_calls_partial_results() {
        // Assistant with 2 tool_calls but only 1 matching result
        let mut messages = vec![
            Message::system("sys".to_string()),
            Message {
                role: zeus_core::Role::Assistant,
                content: "calling tools".to_string(),
                tool_calls: vec![
                    zeus_core::ToolCall {
                        id: "tc_a".to_string(),
                        name: "shell".to_string(),
                        arguments: Default::default(),
                    },
                    zeus_core::ToolCall {
                        id: "tc_b".to_string(),
                        name: "shell".to_string(),
                        arguments: Default::default(),
                    },
                ],
                tool_results: vec![],
                timestamp: chrono::Utc::now(),
                attachments: vec![],
                message_id: None,
                parent_id: None,
                thread_id: None,
                direction: Default::default(),
            channel_source: None,
                compaction_hint: Default::default(),
            },
            Message {
                role: zeus_core::Role::Tool,
                content: String::new(),
                tool_calls: vec![],
                tool_results: vec![zeus_core::ToolResult {
                    call_id: "tc_a".to_string(),
                    success: true,
                    output: "ok".to_string(),
                }],
                timestamp: chrono::Utc::now(),
                attachments: vec![],
                message_id: None,
                parent_id: None,
                thread_id: None,
                direction: Default::default(),
            channel_source: None,
                compaction_hint: Default::default(),
            },
        ];
        ContextGuard::repair_orphaned_tool_calls(&mut messages);
        // Should append synthetic result for tc_b to existing tool message
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].tool_results.len(), 2);
        let ids: Vec<&str> = messages[2].tool_results.iter().map(|r| r.call_id.as_str()).collect();
        assert!(ids.contains(&"tc_a"));
        assert!(ids.contains(&"tc_b"));
    }

    #[test]
    fn test_repair_orphaned_tool_calls_no_orphans() {
        // All tool_calls have matching results — no repair needed
        let mut messages = vec![
            Message {
                role: zeus_core::Role::Assistant,
                content: "calling tool".to_string(),
                tool_calls: vec![zeus_core::ToolCall {
                    id: "tc_ok".to_string(),
                    name: "shell".to_string(),
                    arguments: Default::default(),
                }],
                tool_results: vec![],
                timestamp: chrono::Utc::now(),
                attachments: vec![],
                message_id: None,
                parent_id: None,
                thread_id: None,
                direction: Default::default(),
            channel_source: None,
                compaction_hint: Default::default(),
            },
            Message {
                role: zeus_core::Role::Tool,
                content: String::new(),
                tool_calls: vec![],
                tool_results: vec![zeus_core::ToolResult {
                    call_id: "tc_ok".to_string(),
                    success: true,
                    output: "done".to_string(),
                }],
                timestamp: chrono::Utc::now(),
                attachments: vec![],
                message_id: None,
                parent_id: None,
                thread_id: None,
                direction: Default::default(),
            channel_source: None,
                compaction_hint: Default::default(),
            },
        ];
        let original_len = messages.len();
        ContextGuard::repair_orphaned_tool_calls(&mut messages);
        assert_eq!(messages.len(), original_len);
    }
}
