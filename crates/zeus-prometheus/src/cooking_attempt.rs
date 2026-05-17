//! Single Attempt Execution
//!
//! Handles a single LLM interaction attempt, including response
//! handling and outcome classification.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zeus_core::Message;
use zeus_llm::{LlmResponse, StopReason};

/// Outcome of a cooking attempt
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// Request completed successfully
    Success,
    /// Transient error - retry with backoff
    RetryTransient,
    /// Need to rotate auth profile
    RotateProfile(super::cooking_auth::FailoverReason),
    /// Context overflow - compact and retry
    CompactAndRetry,
    /// Fatal error - do not retry
    HardFailure,
}

/// Result of a single attempt
#[derive(Debug, Clone)]
pub struct AttemptResult {
    /// The response text (if successful)
    pub text: Option<String>,
    /// Tool calls made (if any)
    pub tool_calls: Vec<AttemptToolCall>,
    /// Stop reason
    pub stop_reason: Option<StopReason>,
    /// Input tokens used
    pub input_tokens: usize,
    /// Output tokens used
    pub output_tokens: usize,
    /// Error (if failed)
    pub error: Option<String>,
    /// Classified outcome
    pub outcome: AttemptOutcome,
    /// Duration of the attempt
    pub duration_ms: u64,
    /// When the attempt started
    pub started_at: DateTime<Utc>,
    /// When the attempt ended
    pub ended_at: DateTime<Utc>,
}

impl AttemptResult {
    /// Create a successful result
    pub fn success(response: LlmResponse, duration_ms: u64) -> Self {
        let started_at = Utc::now() - chrono::Duration::milliseconds(duration_ms as i64);
        Self {
            text: Some(response.content.clone()),
            tool_calls: response
                .tool_calls
                .into_iter()
                .map(|tc| AttemptToolCall {
                    id: tc.id,
                    name: tc.name,
                    input: tc.arguments,
                })
                .collect(),
            stop_reason: Some(response.stop_reason),
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
            error: None,
            outcome: AttemptOutcome::Success,
            duration_ms,
            started_at,
            ended_at: Utc::now(),
        }
    }

    /// Create a failed result
    pub fn failure(error: zeus_core::Error, outcome: AttemptOutcome, duration_ms: u64) -> Self {
        let started_at = Utc::now() - chrono::Duration::milliseconds(duration_ms as i64);
        Self {
            text: None,
            tool_calls: Vec::new(),
            stop_reason: None,
            input_tokens: 0,
            output_tokens: 0,
            error: Some(error.to_string()),
            outcome,
            duration_ms,
            started_at,
            ended_at: Utc::now(),
        }
    }

    /// Check if the attempt was successful
    pub fn is_success(&self) -> bool {
        self.outcome == AttemptOutcome::Success
    }

    /// Check if the attempt should be retried
    pub fn should_retry(&self) -> bool {
        matches!(
            self.outcome,
            AttemptOutcome::RetryTransient | AttemptOutcome::CompactAndRetry
        )
    }

    /// Check if profile rotation is needed
    pub fn needs_rotation(&self) -> Option<super::cooking_auth::FailoverReason> {
        match &self.outcome {
            AttemptOutcome::RotateProfile(reason) => Some(*reason),
            _ => None,
        }
    }

    /// Get the response text
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Total tokens used
    pub fn total_tokens(&self) -> usize {
        self.input_tokens + self.output_tokens
    }
}

/// Tool call from an attempt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// Single attempt executor
pub struct Attempt {
    /// Messages for this attempt
    messages: Vec<Message>,
    /// System prompt
    system: Option<String>,
    /// Maximum tokens
    max_tokens: u32,
    /// Current token count estimate
    current_tokens: u64,
}

impl Attempt {
    /// Create a new attempt
    pub fn new(messages: Vec<Message>, system: Option<String>, max_tokens: u32) -> Self {
        Self {
            messages,
            system,
            max_tokens,
            current_tokens: 0,
        }
    }

    /// Set the current token count estimate
    pub fn with_token_count(mut self, tokens: u64) -> Self {
        self.current_tokens = tokens;
        self
    }

    /// Get the messages
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Get the system prompt
    pub fn system(&self) -> Option<&str> {
        self.system.as_deref()
    }

    /// Get max tokens
    pub fn max_tokens(&self) -> u32 {
        self.max_tokens
    }

    /// Get current token count
    pub fn current_tokens(&self) -> u64 {
        self.current_tokens
    }
}

/// Classify the outcome of an attempt from an error
pub fn classify_attempt_outcome(error: &zeus_core::Error) -> AttemptOutcome {
    use super::cooking_auth::FailoverReason;
    use super::cooking_errors::{ErrorClass, classify_error};

    match classify_error(error) {
        ErrorClass::Transient => AttemptOutcome::RetryTransient,
        ErrorClass::RateLimit => AttemptOutcome::RotateProfile(FailoverReason::RateLimit),
        ErrorClass::Billing => AttemptOutcome::RotateProfile(FailoverReason::Billing),
        ErrorClass::Auth => AttemptOutcome::RotateProfile(FailoverReason::Auth),
        ErrorClass::ContextOverflow => AttemptOutcome::CompactAndRetry,
        ErrorClass::ProviderUnavailable => {
            AttemptOutcome::RotateProfile(FailoverReason::Unavailable)
        }
        ErrorClass::Fatal => AttemptOutcome::HardFailure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_successful_result() {
        let response = LlmResponse {
            content: "Hello!".to_string(),
            tool_calls: vec![],
            stop_reason: StopReason::EndTurn,
            input_tokens: 10,
            output_tokens: 5,
            cached_tokens: 0,
        };

        let result = AttemptResult::success(response, 100);

        assert!(result.is_success());
        assert!(!result.should_retry());
        assert!(result.needs_rotation().is_none());
        assert_eq!(result.text(), Some("Hello!"));
        assert_eq!(result.total_tokens(), 15);
    }

    #[test]
    fn test_transient_failure() {
        let error = zeus_core::Error::llm("Connection timeout");
        let result = AttemptResult::failure(error, AttemptOutcome::RetryTransient, 50);

        assert!(!result.is_success());
        assert!(result.should_retry());
        assert!(result.needs_rotation().is_none());
    }

    #[test]
    fn test_rotation_failure() {
        let error = zeus_core::Error::llm("Rate limit exceeded");
        let result = AttemptResult::failure(
            error,
            AttemptOutcome::RotateProfile(super::super::cooking_auth::FailoverReason::RateLimit),
            50,
        );

        assert!(!result.is_success());
        assert!(!result.should_retry());
        assert!(result.needs_rotation().is_some());
    }

    #[test]
    fn test_compact_failure() {
        let error = zeus_core::Error::llm("Context too long");
        let result = AttemptResult::failure(error, AttemptOutcome::CompactAndRetry, 50);

        assert!(!result.is_success());
        assert!(result.should_retry());
    }

    #[test]
    fn test_classify_attempt_outcome() {
        let rate_limit = zeus_core::Error::llm("Error 429: Rate limit exceeded");
        assert!(matches!(
            classify_attempt_outcome(&rate_limit),
            AttemptOutcome::RotateProfile(super::super::cooking_auth::FailoverReason::RateLimit)
        ));

        let context = zeus_core::Error::llm("Context length exceeds maximum");
        assert_eq!(
            classify_attempt_outcome(&context),
            AttemptOutcome::CompactAndRetry
        );
    }
}
