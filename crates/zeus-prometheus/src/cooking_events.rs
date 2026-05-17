//! Streaming Events
//!
//! Event types for real-time streaming of cooking loop progress,
//! enabling UIs and channels to show live updates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Events emitted during the cooking loop
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CookingEvent {
    /// Message generation started
    MessageStart {
        session_id: String,
        provider: String,
        model: String,
        timestamp: DateTime<Utc>,
    },

    /// Text content being streamed
    TextDelta {
        text: String,
        timestamp: DateTime<Utc>,
    },

    /// Tool execution started
    ToolExecutionStart {
        id: String,
        name: String,
        input: Option<serde_json::Value>,
        timestamp: DateTime<Utc>,
    },

    /// Tool execution completed
    ToolExecutionComplete {
        id: String,
        name: String,
        result: String,
        is_error: bool,
        duration_ms: u64,
        timestamp: DateTime<Utc>,
    },

    /// Thinking/reasoning indicator (for extended thinking)
    ThinkingStart { timestamp: DateTime<Utc> },

    /// Thinking/reasoning content delta
    ThinkingDelta {
        text: String,
        timestamp: DateTime<Utc>,
    },

    /// Thinking completed
    ThinkingEnd { timestamp: DateTime<Utc> },

    /// Message generation completed
    MessageEnd {
        stop_reason: String,
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_input_tokens: Option<u64>,
        cache_read_input_tokens: Option<u64>,
        timestamp: DateTime<Utc>,
    },

    /// Context compaction started
    CompactionStart {
        session_id: String,
        from_tokens: u64,
        timestamp: DateTime<Utc>,
    },

    /// Context compaction completed
    CompactionComplete {
        session_id: String,
        from_tokens: u64,
        to_tokens: u64,
        success: bool,
        summary_preview: Option<String>,
        timestamp: DateTime<Utc>,
    },

    /// Auth profile rotated
    ProfileRotated {
        from_profile: String,
        to_profile: String,
        reason: String,
        timestamp: DateTime<Utc>,
    },

    /// Retry attempt
    RetryAttempt {
        attempt_number: u32,
        max_attempts: u32,
        backoff_ms: u64,
        reason: String,
        timestamp: DateTime<Utc>,
    },

    /// Error occurred (non-fatal, cooking continues)
    Warning {
        message: String,
        recoverable: bool,
        timestamp: DateTime<Utc>,
    },

    /// Session started
    SessionStart {
        session_id: String,
        provider: String,
        model: String,
        timestamp: DateTime<Utc>,
    },

    /// Session ended
    SessionEnd {
        session_id: String,
        total_input_tokens: u64,
        total_output_tokens: u64,
        total_messages: u32,
        timestamp: DateTime<Utc>,
    },
}

impl CookingEvent {
    /// Create a MessageStart event
    pub fn message_start(session_id: &str, provider: &str, model: &str) -> Self {
        Self::MessageStart {
            session_id: session_id.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            timestamp: Utc::now(),
        }
    }

    /// Create a TextDelta event
    pub fn text_delta(text: impl Into<String>) -> Self {
        Self::TextDelta {
            text: text.into(),
            timestamp: Utc::now(),
        }
    }

    /// Create a ToolExecutionStart event
    pub fn tool_start(id: &str, name: &str, input: Option<serde_json::Value>) -> Self {
        Self::ToolExecutionStart {
            id: id.to_string(),
            name: name.to_string(),
            input,
            timestamp: Utc::now(),
        }
    }

    /// Create a ToolExecutionComplete event
    pub fn tool_complete(
        id: &str,
        name: &str,
        result: &str,
        is_error: bool,
        duration_ms: u64,
    ) -> Self {
        Self::ToolExecutionComplete {
            id: id.to_string(),
            name: name.to_string(),
            result: result.to_string(),
            is_error,
            duration_ms,
            timestamp: Utc::now(),
        }
    }

    /// Create a MessageEnd event
    pub fn message_end(stop_reason: &str, input_tokens: u64, output_tokens: u64) -> Self {
        Self::MessageEnd {
            stop_reason: stop_reason.to_string(),
            input_tokens,
            output_tokens,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
            timestamp: Utc::now(),
        }
    }

    /// Create a CompactionStart event
    pub fn compaction_start(session_id: &str, from_tokens: u64) -> Self {
        Self::CompactionStart {
            session_id: session_id.to_string(),
            from_tokens,
            timestamp: Utc::now(),
        }
    }

    /// Create a CompactionComplete event
    pub fn compaction_complete(
        session_id: &str,
        from_tokens: u64,
        to_tokens: u64,
        success: bool,
        summary_preview: Option<String>,
    ) -> Self {
        Self::CompactionComplete {
            session_id: session_id.to_string(),
            from_tokens,
            to_tokens,
            success,
            summary_preview,
            timestamp: Utc::now(),
        }
    }

    /// Create a ProfileRotated event
    pub fn profile_rotated(from: &str, to: &str, reason: &str) -> Self {
        Self::ProfileRotated {
            from_profile: from.to_string(),
            to_profile: to.to_string(),
            reason: reason.to_string(),
            timestamp: Utc::now(),
        }
    }

    /// Create a RetryAttempt event
    pub fn retry_attempt(
        attempt_number: u32,
        max_attempts: u32,
        backoff_ms: u64,
        reason: &str,
    ) -> Self {
        Self::RetryAttempt {
            attempt_number,
            max_attempts,
            backoff_ms,
            reason: reason.to_string(),
            timestamp: Utc::now(),
        }
    }

    /// Create a Warning event
    pub fn warning(message: impl Into<String>, recoverable: bool) -> Self {
        Self::Warning {
            message: message.into(),
            recoverable,
            timestamp: Utc::now(),
        }
    }

    /// Create a SessionStart event
    pub fn session_start(session_id: &str, provider: &str, model: &str) -> Self {
        Self::SessionStart {
            session_id: session_id.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            timestamp: Utc::now(),
        }
    }

    /// Create a SessionEnd event
    pub fn session_end(
        session_id: &str,
        total_input_tokens: u64,
        total_output_tokens: u64,
        total_messages: u32,
    ) -> Self {
        Self::SessionEnd {
            session_id: session_id.to_string(),
            total_input_tokens,
            total_output_tokens,
            total_messages,
            timestamp: Utc::now(),
        }
    }

    /// Get the timestamp of the event
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Self::MessageStart { timestamp, .. } => *timestamp,
            Self::TextDelta { timestamp, .. } => *timestamp,
            Self::ToolExecutionStart { timestamp, .. } => *timestamp,
            Self::ToolExecutionComplete { timestamp, .. } => *timestamp,
            Self::ThinkingStart { timestamp } => *timestamp,
            Self::ThinkingDelta { timestamp, .. } => *timestamp,
            Self::ThinkingEnd { timestamp } => *timestamp,
            Self::MessageEnd { timestamp, .. } => *timestamp,
            Self::CompactionStart { timestamp, .. } => *timestamp,
            Self::CompactionComplete { timestamp, .. } => *timestamp,
            Self::ProfileRotated { timestamp, .. } => *timestamp,
            Self::RetryAttempt { timestamp, .. } => *timestamp,
            Self::Warning { timestamp, .. } => *timestamp,
            Self::SessionStart { timestamp, .. } => *timestamp,
            Self::SessionEnd { timestamp, .. } => *timestamp,
        }
    }

    /// Check if this is a terminal event (message end or session end)
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::MessageEnd { .. } | Self::SessionEnd { .. })
    }

    /// Get the event type as a string
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::MessageStart { .. } => "message_start",
            Self::TextDelta { .. } => "text_delta",
            Self::ToolExecutionStart { .. } => "tool_execution_start",
            Self::ToolExecutionComplete { .. } => "tool_execution_complete",
            Self::ThinkingStart { .. } => "thinking_start",
            Self::ThinkingDelta { .. } => "thinking_delta",
            Self::ThinkingEnd { .. } => "thinking_end",
            Self::MessageEnd { .. } => "message_end",
            Self::CompactionStart { .. } => "compaction_start",
            Self::CompactionComplete { .. } => "compaction_complete",
            Self::ProfileRotated { .. } => "profile_rotated",
            Self::RetryAttempt { .. } => "retry_attempt",
            Self::Warning { .. } => "warning",
            Self::SessionStart { .. } => "session_start",
            Self::SessionEnd { .. } => "session_end",
        }
    }
}

/// Event emitter for broadcasting cooking events
pub struct EventEmitter {
    sender: broadcast::Sender<CookingEvent>,
}

impl EventEmitter {
    /// Create a new event emitter
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(1000);
        Self { sender }
    }

    /// Create with custom channel capacity
    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Emit an event to all subscribers
    pub fn emit(&self, event: CookingEvent) {
        // We ignore send errors - they just mean no one is listening
        let _ = self.sender.send(event);
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<CookingEvent> {
        self.sender.subscribe()
    }

    /// Get the number of current subscribers
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }

    /// Emit a message start event
    pub fn message_start(&self, session_id: &str, provider: &str, model: &str) {
        self.emit(CookingEvent::message_start(session_id, provider, model));
    }

    /// Emit a text delta event
    pub fn text_delta(&self, text: impl Into<String>) {
        self.emit(CookingEvent::text_delta(text));
    }

    /// Emit a tool execution start event
    pub fn tool_start(&self, id: &str, name: &str, input: Option<serde_json::Value>) {
        self.emit(CookingEvent::tool_start(id, name, input));
    }

    /// Emit a tool execution complete event
    pub fn tool_complete(
        &self,
        id: &str,
        name: &str,
        result: &str,
        is_error: bool,
        duration_ms: u64,
    ) {
        self.emit(CookingEvent::tool_complete(
            id,
            name,
            result,
            is_error,
            duration_ms,
        ));
    }

    /// Emit a message end event
    pub fn message_end(&self, stop_reason: &str, input_tokens: u64, output_tokens: u64) {
        self.emit(CookingEvent::message_end(
            stop_reason,
            input_tokens,
            output_tokens,
        ));
    }
}

impl Default for EventEmitter {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for EventEmitter {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_creation() {
        let event =
            CookingEvent::message_start("session-1", "anthropic", "claude-sonnet-4-20250514");
        assert!(matches!(event, CookingEvent::MessageStart { .. }));
        assert_eq!(event.event_type(), "message_start");
    }

    #[test]
    fn test_text_delta() {
        let event = CookingEvent::text_delta("Hello, world!");
        if let CookingEvent::TextDelta { text, .. } = event {
            assert_eq!(text, "Hello, world!");
        } else {
            panic!("Expected TextDelta");
        }
    }

    #[test]
    fn test_event_emitter() {
        let emitter = EventEmitter::new();
        let mut receiver = emitter.subscribe();

        emitter.text_delta("test");

        let event = receiver.try_recv().unwrap();
        assert!(matches!(event, CookingEvent::TextDelta { .. }));
    }

    #[test]
    fn test_event_serialization() {
        let event = CookingEvent::tool_complete("tool-1", "read_file", "content here", false, 150);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("tool_execution_complete"));
        assert!(json.contains("read_file"));
    }

    #[test]
    fn test_is_terminal() {
        let text = CookingEvent::text_delta("hello");
        assert!(!text.is_terminal());

        let end = CookingEvent::message_end("end_turn", 100, 50);
        assert!(end.is_terminal());
    }
}
