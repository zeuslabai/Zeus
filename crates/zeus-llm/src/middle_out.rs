//! Middle-Out Context Compression (F2)
//!
//! When a conversation approaches the model's context window limit,
//! truncating from the start loses early context (system prompts, task
//! setup) and truncating from the end loses recent conversation.
//!
//! Middle-out compression keeps the first `keep_first` messages (early
//! context) + the last `keep_last` messages (recent turns) and drops the
//! middle, optionally replacing them with a marker message so the model
//! knows compression happened.
//!
//! # Example
//!
//! ```no_run
//! use zeus_llm::middle_out::compress_middle_out;
//! use zeus_core::Message;
//!
//! let messages: Vec<Message> = vec![/* ... */];
//! let compressed = compress_middle_out(&messages, 8000, 3, 5);
//! ```
//!
//! # Algorithm
//!
//! 1. Estimate total tokens. If under `max_tokens`, return as-is.
//! 2. Keep `messages[..keep_first]` and `messages[len-keep_last..]`.
//! 3. Drop everything in between; optionally insert a marker message.
//! 4. If kept messages *still* exceed `max_tokens`, we return them anyway
//!    — the caller is responsible for handling the pathological case of
//!    `keep_first + keep_last` messages alone exceeding the budget.

use zeus_core::{CompactionHint, Message, Role, TextDirection};

/// Per-message framing overhead in tokens (role tokens, separators).
const MESSAGE_OVERHEAD_TOKENS: usize = 4;

/// Rough token estimator: ~4 chars per token (standard approximation).
///
/// This is intentionally a local, dependency-free estimator. For a more
/// accurate count, use `zeus-session::token_counter::estimate_tokens`.
#[inline]
fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    (text.len() / 4).max(1)
}

/// Estimate tokens for a single message, including tool calls/results.
fn message_tokens(msg: &Message) -> usize {
    let mut total = MESSAGE_OVERHEAD_TOKENS + estimate_tokens(&msg.content);
    for call in &msg.tool_calls {
        // arguments is serde_json::Value — render as string for estimation
        total += estimate_tokens(&call.name);
        total += estimate_tokens(&call.arguments.to_string());
    }
    for result in &msg.tool_results {
        total += estimate_tokens(&result.output);
    }
    total
}

/// Estimate tokens for a slice of messages.
fn total_tokens(messages: &[Message]) -> usize {
    messages.iter().map(message_tokens).sum()
}

/// Build a marker message indicating N messages were compressed out.
fn compression_marker(dropped: usize) -> Message {
    Message {
        role: Role::System,
        content: format!("[context compressed: {dropped} messages removed]"),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
        timestamp: chrono::Utc::now(),
        attachments: Vec::new(),
        message_id: None,
        parent_id: None,
        thread_id: None,
        direction: TextDirection::default(),
        channel_source: None,
        compaction_hint: CompactionHint::default(),
    }
}

/// Compress a message list by keeping the head and tail, dropping the middle.
///
/// # Parameters
/// - `messages`: full conversation history
/// - `max_tokens`: token budget to stay under (best-effort)
/// - `keep_first`: number of messages to preserve from the start (e.g. system prompt)
/// - `keep_last`: number of messages to preserve from the end (recent turns)
///
/// # Returns
/// A new `Vec<Message>`. If no compression is needed (total tokens ≤ `max_tokens`
/// or `keep_first + keep_last >= messages.len()`), returns a clone of the input.
/// Otherwise returns `[first N] + [marker] + [last M]`.
pub fn compress_middle_out(
    messages: &[Message],
    max_tokens: usize,
    keep_first: usize,
    keep_last: usize,
) -> Vec<Message> {
    // Nothing to compress if we already fit or we'd keep everything anyway.
    if messages.is_empty() {
        return Vec::new();
    }
    if keep_first + keep_last >= messages.len() {
        return messages.to_vec();
    }
    if total_tokens(messages) <= max_tokens {
        return messages.to_vec();
    }

    let len = messages.len();
    let dropped = len - keep_first - keep_last;

    let mut out = Vec::with_capacity(keep_first + keep_last + 1);
    out.extend_from_slice(&messages[..keep_first]);
    if dropped > 0 {
        out.push(compression_marker(dropped));
    }
    out.extend_from_slice(&messages[len - keep_last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use zeus_core::Role;

    fn msg(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
            timestamp: Utc::now(),
            attachments: Vec::new(),
            message_id: None,
            parent_id: None,
            thread_id: None,
            direction: TextDirection::default(),
            channel_source: None,
            compaction_hint: CompactionHint::default(),
        }
    }

    fn big_msg(role: Role, size: usize) -> Message {
        msg(role, &"x".repeat(size))
    }

    #[test]
    fn empty_input_returns_empty() {
        let out = compress_middle_out(&[], 1000, 3, 5);
        assert!(out.is_empty());
    }

    #[test]
    fn under_budget_returns_unchanged() {
        let messages = vec![
            msg(Role::System, "sys"),
            msg(Role::User, "hello"),
            msg(Role::Assistant, "hi"),
        ];
        let out = compress_middle_out(&messages, 10_000, 3, 5);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].content, "sys");
        assert_eq!(out[2].content, "hi");
    }

    #[test]
    fn keep_all_when_keep_params_exceed_len() {
        let messages = vec![
            msg(Role::System, "sys"),
            msg(Role::User, "hello"),
        ];
        let out = compress_middle_out(&messages, 1, 3, 5);
        assert_eq!(out.len(), 2, "keep_first+keep_last >= len — no compression");
    }

    #[test]
    fn compresses_middle_when_over_budget() {
        // 10 large messages, budget way below total.
        let messages: Vec<Message> = (0..10)
            .map(|i| {
                let role = if i == 0 { Role::System } else { Role::User };
                big_msg(role, 4000) // ~1000 tokens each
            })
            .collect();

        let out = compress_middle_out(&messages, 500, 2, 2);

        // Expected: 2 head + 1 marker + 2 tail = 5 messages
        assert_eq!(out.len(), 5, "head + marker + tail");
        assert_eq!(out[0].role, Role::System);
        assert_eq!(out[2].role, Role::System, "marker is a system message");
        assert!(
            out[2].content.contains("context compressed"),
            "marker content: {:?}",
            out[2].content
        );
        assert!(out[2].content.contains("6 messages"), "6 dropped: {:?}", out[2].content);
    }

    #[test]
    fn marker_reports_correct_dropped_count() {
        let messages: Vec<Message> = (0..20)
            .map(|_| big_msg(Role::User, 4000))
            .collect();
        let out = compress_middle_out(&messages, 100, 3, 5);
        // Dropped = 20 - 3 - 5 = 12
        assert_eq!(out.len(), 3 + 1 + 5);
        assert!(out[3].content.contains("12 messages"));
    }

    #[test]
    fn preserves_first_and_last_message_identity() {
        let mut messages: Vec<Message> = (0..10)
            .map(|i| big_msg(Role::User, 4000))
            .collect();
        messages[0].content = "FIRST_MARKER".to_string();
        messages[9].content = "LAST_MARKER".to_string();

        let out = compress_middle_out(&messages, 100, 2, 2);
        assert_eq!(out.first().unwrap().content, "FIRST_MARKER");
        assert_eq!(out.last().unwrap().content, "LAST_MARKER");
    }

    #[test]
    fn no_compression_when_barely_over_but_params_cover_all() {
        // 4 messages, keep_first=2, keep_last=2 → covers all, no drop
        let messages: Vec<Message> = (0..4)
            .map(|_| big_msg(Role::User, 10_000))
            .collect();
        let out = compress_middle_out(&messages, 10, 2, 2);
        assert_eq!(out.len(), 4, "keep params cover entire list");
    }

    #[test]
    fn zero_keep_first_drops_from_start() {
        let messages: Vec<Message> = (0..10).map(|_| big_msg(Role::User, 4000)).collect();
        let out = compress_middle_out(&messages, 100, 0, 3);
        // 0 head + 1 marker + 3 tail = 4
        assert_eq!(out.len(), 4);
        assert!(out[0].content.contains("context compressed"));
    }

    #[test]
    fn token_estimator_handles_empty_and_short() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("x"), 1);
        assert_eq!(estimate_tokens(&"a".repeat(400)), 100);
    }
}
