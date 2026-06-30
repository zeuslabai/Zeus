use zeus_core::{Message, Role};

/// Determines whether a message is worth storing in Mnemosyne.
///
/// Strategy: cheap heuristic first, escalate to Haiku for ambiguous cases.
/// This runs synchronously before every `store_with_embedding` call.
#[derive(Debug, Clone, Default)]
pub struct MessageStoreFilter {
    /// Haiku-escalation threshold: if content_len >= this, run Haiku classification.
    /// Below this, heuristic filter decides alone.
    haiku_threshold_chars: usize,
}

impl MessageStoreFilter {
    pub fn new() -> Self {
        Self {
            haiku_threshold_chars: 200,
        }
    }

    /// Returns true if the message should be stored, false if it should be skipped.
    pub async fn should_store(&self, msg: &Message) -> StoreDecision {
        // Rule 1: System messages are routing wrappers — skip
        if msg.role == Role::System {
            return StoreDecision::Skip("system role".to_string());
        }

        // Rule 2: Empty or near-empty content — skip
        let trimmed = msg.content.trim();
        if trimmed.is_empty() && msg.tool_results.is_empty() {
            return StoreDecision::Skip("empty content".to_string());
        }

        // Rule 3: Tool results with outcomes — store
        if !msg.tool_results.is_empty() {
            // Check if any tool result has meaningful output
            let has_outcome = msg.tool_results.iter().any(|tr| {
                let out = tr.output.trim();
                !out.is_empty() && out.len() > 2 && out != "{}"
            });
            if has_outcome {
                return StoreDecision::Store("tool result with outcome".to_string());
            }
        }

        // Rule 4: General chat acks — skip
        let ack_patterns = ["ok", "oke", "okay", "nice", "got it", "thanks", "sure", "yes", "no", "lol", "lol", "👍", "👍🏿", "😅"];
        let lower = trimmed.to_lowercase();
        if ack_patterns.iter().any(|p| lower == *p || lower == format!("ok {}", p)) {
            return StoreDecision::Skip("chat ack".to_string());
        }

        // Rule 5: Heartbeat reports — skip if no state change
        // These typically say "HEARTBEAT_OK" or contain session status with no task progress
        if trimmed.starts_with("HEARTBEAT_OK") || trimmed.starts_with("[HEARTBEAT]") {
            // Check for task/order content — if present, store it
            if self.contains_task_signal(msg) {
                return StoreDecision::Store("heartbeat with task signal".to_string());
            }
            return StoreDecision::Skip("heartbeat no-op".to_string());
        }

        // Rule 6: @mention with task/order — store
        if self.contains_task_signal(msg) {
            return StoreDecision::Store("contains task signal".to_string());
        }

        // Rule 7: Short messages below threshold — heuristic only
        if trimmed.len() < self.haiku_threshold_chars {
            // Ambiguous: could be a short task or could be noise
            // For now: default to SKIP for very short messages
            // This avoids storing "ok" type messages that slipped past Rule 4
            return StoreDecision::Skip("below threshold".to_string());
        }

        // Rule 8: Medium+ content without clear signal — escalate to Haiku
        // (Haiku call would go here — deferred to Step 5)
        StoreDecision::Escalate("ambiguous content".to_string())
    }

    /// Check if message contains task/order signals: @mentions, directives, commit refs, etc.
    fn contains_task_signal(&self, msg: &Message) -> bool {
        let content = &msg.content;

        // @mention pattern
        if content.contains('@') && content.contains("task") | content.contains("do") | content.contains("build") | content.contains("fix") | content.contains("ship") | content.contains("review") {
            return true;
        }

        // Directive patterns
        let directives = ["#plan", "#todo", "[plan]", "[task]", "take ", "pick up", "assign", "priority"];
        if directives.iter().any(|d| content.to_lowercase().contains(d)) {
            return true;
        }

        // Commit reference pattern
        if content.contains("commit:") || content.contains("SHA:") || content.contains("git commit") {
            return true;
        }

        // Task ID pattern (e.g., "R1", "R2", "Track A")
        if content.contains("Track ") || content.contains(" R") && content.len() < 20 {
            return true;
        }

        false
    }
}

#[derive(Debug, Clone)]
pub enum StoreDecision {
    Store(String),
    Skip(String),
    Escalate(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
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
        }
    }

    #[tokio::test]
    async fn test_system_messages_skipped() {
        let filter = MessageStoreFilter::new();
        let msg = make_msg(Role::System, "Routing message to agent...");
        let result = filter.should_store(&msg).await;
        assert!(matches!(result, StoreDecision::Skip(_)));
    }

    #[tokio::test]
    async fn test_tool_results_stored() {
        let filter = MessageStoreFilter::new();
        let mut msg = make_msg(Role::Tool, "");
        msg.tool_results.push(zeus_core::ToolResult {
            call_id: "1".to_string(),
            output: "File created at /path/to/file.rs".to_string(),
            success: true,
        });
        let result = filter.should_store(&msg).await;
        assert!(matches!(result, StoreDecision::Store(_)));
    }

    #[tokio::test]
    async fn test_chat_acks_skipped() {
        let filter = MessageStoreFilter::new();
        for ack in &["ok", "nice", "got it", "sure", "👍"] {
            let msg = make_msg(Role::User, ack);
            let result = filter.should_store(&msg).await;
            assert!(matches!(result, StoreDecision::Skip(_)), "expected skip for: {}", ack);
        }
    }

    #[tokio::test]
    async fn test_heartbeat_ok_skipped() {
        let filter = MessageStoreFilter::new();
        let msg = make_msg(Role::User, "HEARTBEAT_OK");
        let result = filter.should_store(&msg).await;
        assert!(matches!(result, StoreDecision::Skip(_)));
    }

    #[tokio::test]
    async fn test_task_mention_stored() {
        let filter = MessageStoreFilter::new();
        let msg = make_msg(Role::User, "@molty pick up R2 from the backlog");
        let result = filter.should_store(&msg).await;
        assert!(matches!(result, StoreDecision::Store(_)));
    }

    #[tokio::test]
    async fn test_empty_content_skipped() {
        let filter = MessageStoreFilter::new();
        let msg = make_msg(Role::User, "   ");
        let result = filter.should_store(&msg).await;
        assert!(matches!(result, StoreDecision::Skip(_)));
    }
}