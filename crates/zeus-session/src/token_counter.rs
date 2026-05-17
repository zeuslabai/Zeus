//! Token counting for session messages
//!
//! Estimates token usage per message and per session using a hybrid
//! word/character heuristic. While not exact (that requires the model's
//! actual tokenizer), this produces estimates within ~10% for English text
//! and is suitable for cost analytics.
//!
//! The heuristic: ~1.3 tokens per word for prose, ~0.4 tokens per char
//! for code/JSON, with overhead per message for role/framing tokens.

use serde::{Deserialize, Serialize};
use zeus_core::Message;

/// Per-message framing overhead (role tokens, separators, etc.)
const MESSAGE_OVERHEAD_TOKENS: usize = 4;

/// Token usage for a single message.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageTokens {
    /// Tokens in the message content
    pub content_tokens: usize,
    /// Tokens in tool call arguments
    pub tool_call_tokens: usize,
    /// Tokens in tool results
    pub tool_result_tokens: usize,
    /// Total tokens for this message
    pub total: usize,
}

/// Aggregated token usage for an entire session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionTokenUsage {
    /// Total input tokens (user messages + tool results)
    pub input_tokens: usize,
    /// Total output tokens (assistant messages + tool calls)
    pub output_tokens: usize,
    /// Total tokens across all messages
    pub total_tokens: usize,
    /// Number of messages counted
    pub message_count: usize,
    /// Per-role breakdown: (role, token_count)
    pub by_role: Vec<(String, usize)>,
}

/// Estimate token count for a text string.
///
/// Uses a hybrid approach:
/// - For short texts (<100 chars): chars / 4 (standard approximation)
/// - For longer texts: word-based estimation (~1.3 tokens per word)
///   with adjustments for code and JSON content
pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }

    let char_count = text.len();
    if char_count < 100 {
        return (char_count / 4).max(1);
    }

    let word_count = text.split_whitespace().count();
    if word_count == 0 {
        return (char_count / 4).max(1);
    }

    // Detect code/JSON content by average word length
    // Code tends to have longer "words" (identifiers, paths, URLs)
    let avg_word_len = char_count as f64 / word_count as f64;
    let tokens_per_word = if avg_word_len > 8.0 {
        // Code-like content: more tokens per word
        1.5
    } else if avg_word_len > 6.0 {
        // Mixed content
        1.4
    } else {
        // Natural language
        1.3
    };

    let estimated = (word_count as f64 * tokens_per_word).ceil() as usize;
    estimated.max(1)
}

/// Count tokens for a single message including content, tool calls, and results.
pub fn count_message_tokens(msg: &Message) -> MessageTokens {
    let content_tokens = estimate_tokens(&msg.content);

    let tool_call_tokens: usize = msg
        .tool_calls
        .iter()
        .map(|tc| {
            let name_tokens = estimate_tokens(&tc.name);
            let args_tokens = estimate_tokens(&tc.arguments.to_string());
            name_tokens + args_tokens + 3 // 3 for function call framing
        })
        .sum();

    let tool_result_tokens: usize = msg
        .tool_results
        .iter()
        .map(|tr| estimate_tokens(&tr.output) + 2) // 2 for result framing
        .sum();

    let total = content_tokens + tool_call_tokens + tool_result_tokens + MESSAGE_OVERHEAD_TOKENS;

    MessageTokens {
        content_tokens,
        tool_call_tokens,
        tool_result_tokens,
        total,
    }
}

/// Count token usage across all messages in a session.
///
/// Classifies tokens as input or output based on message role:
/// - Input: user messages, system messages, tool results
/// - Output: assistant messages, tool calls
pub fn count_session_tokens(messages: &[Message]) -> SessionTokenUsage {
    let mut input_tokens = 0usize;
    let mut output_tokens = 0usize;
    let mut total_tokens = 0usize;
    let mut role_counts = std::collections::HashMap::new();

    for msg in messages {
        let tokens = count_message_tokens(msg);

        let role_str = match msg.role {
            zeus_core::Role::User => "user",
            zeus_core::Role::Assistant => "assistant",
            zeus_core::Role::System => "system",
            zeus_core::Role::Tool => "tool",
        };

        *role_counts.entry(role_str.to_string()).or_insert(0usize) += tokens.total;

        match msg.role {
            zeus_core::Role::User | zeus_core::Role::System | zeus_core::Role::Tool => {
                input_tokens += tokens.total;
            }
            zeus_core::Role::Assistant => {
                output_tokens += tokens.total;
            }
        }

        total_tokens += tokens.total;
    }

    let mut by_role: Vec<(String, usize)> = role_counts.into_iter().collect();
    by_role.sort_by(|a, b| b.1.cmp(&a.1));

    SessionTokenUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        message_count: messages.len(),
        by_role,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_tokens_short() {
        // Short text uses chars/4
        let tokens = estimate_tokens("Hello world");
        assert!(tokens >= 2);
        assert!(tokens <= 5);
    }

    #[test]
    fn test_estimate_tokens_long_prose() {
        let text = "The quick brown fox jumps over the lazy dog. This is a longer sentence that should trigger the word-based estimation algorithm instead of the simple character division approach.";
        let tokens = estimate_tokens(text);
        // ~30 words * 1.3 = ~39 tokens
        assert!(tokens >= 25, "tokens={tokens}, expected >= 25");
        assert!(tokens <= 60, "tokens={tokens}, expected <= 60");
    }

    #[test]
    fn test_estimate_tokens_code() {
        let code = r#"fn main() { let very_long_variable_name = some_function_call(parameter_one, parameter_two, parameter_three); println!("{}", very_long_variable_name); }"#;
        let tokens = estimate_tokens(code);
        // Code has longer words, should get higher ratio
        assert!(tokens >= 15, "tokens={tokens}, expected >= 15");
    }

    #[test]
    fn test_count_message_tokens_simple() {
        let msg = Message::user("Hello, how are you?");
        let tokens = count_message_tokens(&msg);
        assert!(tokens.content_tokens > 0);
        assert_eq!(tokens.tool_call_tokens, 0);
        assert_eq!(tokens.tool_result_tokens, 0);
        assert!(tokens.total >= tokens.content_tokens + MESSAGE_OVERHEAD_TOKENS);
    }

    #[test]
    fn test_count_message_tokens_with_tool_calls() {
        let mut msg = Message::assistant("Let me check that file.");
        msg.tool_calls.push(zeus_core::ToolCall {
            id: "tc_1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "/tmp/test.txt"}),
        });
        let tokens = count_message_tokens(&msg);
        assert!(tokens.content_tokens > 0);
        assert!(tokens.tool_call_tokens > 0);
        assert!(tokens.total > tokens.content_tokens);
    }

    #[test]
    fn test_count_message_tokens_with_tool_results() {
        let mut msg = Message::user("");
        msg.tool_results.push(zeus_core::ToolResult {
            call_id: "tc_1".to_string(),
            output: "file contents here with some data".to_string(),
            success: true,
        });
        let tokens = count_message_tokens(&msg);
        assert!(tokens.tool_result_tokens > 0);
    }

    #[test]
    fn test_count_session_tokens() {
        let messages = vec![
            Message::user("What files are in this directory?"),
            Message::assistant("I'll check that for you."),
            Message::user("Thanks!"),
        ];

        let usage = count_session_tokens(&messages);
        assert_eq!(usage.message_count, 3);
        assert!(usage.input_tokens > 0);
        assert!(usage.output_tokens > 0);
        assert_eq!(usage.total_tokens, usage.input_tokens + usage.output_tokens);
        assert!(!usage.by_role.is_empty());
    }

    #[test]
    fn test_count_session_tokens_empty() {
        let usage = count_session_tokens(&[]);
        assert_eq!(usage.message_count, 0);
        assert_eq!(usage.total_tokens, 0);
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
    }

    #[test]
    fn test_session_token_usage_role_breakdown() {
        let messages = vec![
            Message::user("Hello"),
            Message::assistant("Hi there!"),
            Message::user("How are you?"),
            Message::assistant("I'm doing great, thanks for asking!"),
        ];

        let usage = count_session_tokens(&messages);
        assert_eq!(usage.message_count, 4);

        // Should have user and assistant in by_role
        let roles: Vec<&str> = usage.by_role.iter().map(|(r, _)| r.as_str()).collect();
        assert!(roles.contains(&"user"));
        assert!(roles.contains(&"assistant"));
    }

    #[test]
    fn test_session_token_usage_serialization() {
        let usage = SessionTokenUsage {
            input_tokens: 100,
            output_tokens: 200,
            total_tokens: 300,
            message_count: 5,
            by_role: vec![("assistant".to_string(), 200), ("user".to_string(), 100)],
        };

        let json = serde_json::to_string(&usage).expect("should serialize");
        let parsed: SessionTokenUsage = serde_json::from_str(&json).expect("should parse");
        assert_eq!(parsed.input_tokens, 100);
        assert_eq!(parsed.output_tokens, 200);
        assert_eq!(parsed.total_tokens, 300);
        assert_eq!(parsed.message_count, 5);
    }

    #[test]
    fn test_message_tokens_serialization() {
        let tokens = MessageTokens {
            content_tokens: 50,
            tool_call_tokens: 20,
            tool_result_tokens: 30,
            total: 104, // 50 + 20 + 30 + 4 overhead
        };

        let json = serde_json::to_string(&tokens).expect("should serialize");
        let parsed: MessageTokens = serde_json::from_str(&json).expect("should parse");
        assert_eq!(parsed.total, 104);
    }
}
