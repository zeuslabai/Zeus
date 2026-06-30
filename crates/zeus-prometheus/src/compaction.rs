//! Session compaction — summarizes older messages to prevent context overflow.
//!
//! Two-phase approach (inspired by Claude Code + OpenClaw):
//! 1. Strip tool result details from older messages (cheap, high token savings)
//! 2. Summarize older conversation into a compact summary via LLM
//!
//! Runs BETWEEN Discord messages (before history injection into cooking loop),
//! not just within cooking loop iterations.

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use zeus_core::{Message, Result, Role};
use zeus_llm::LlmClient;

/// Configuration for session compaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Maximum context tokens (model-dependent).
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: u64,

    /// Trigger compaction when estimated tokens exceed this fraction (0.0-1.0).
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: f64,

    /// Number of recent messages to always keep verbatim (never summarize).
    #[serde(default = "default_recent_preserve")]
    pub recent_preserve: usize,

    /// Maximum tokens for the generated summary.
    #[serde(default = "default_summary_max_tokens")]
    pub summary_max_tokens: usize,

    /// Whether compaction is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_max_context_tokens() -> u64 { 180_000 }
fn default_compaction_threshold() -> f64 { 0.80 }
fn default_recent_preserve() -> usize { 20 } // 5 was too aggressive — agents lost task context
fn default_summary_max_tokens() -> usize { 2000 }
fn default_enabled() -> bool { true }

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: default_max_context_tokens(),
            compaction_threshold: default_compaction_threshold(),
            recent_preserve: default_recent_preserve(),
            summary_max_tokens: default_summary_max_tokens(),
            enabled: default_enabled(),
        }
    }
}

/// Result of a compaction operation.
#[derive(Debug)]
pub struct CompactionResult {
    /// Whether compaction was performed.
    pub compacted: bool,
    /// Number of messages before compaction.
    pub messages_before: usize,
    /// Number of messages after compaction.
    pub messages_after: usize,
    /// Estimated tokens before compaction.
    pub tokens_before: u64,
    /// Estimated tokens after compaction.
    pub tokens_after: u64,
    /// The compacted message list (use this for the cooking loop).
    pub messages: Vec<Message>,
}

/// Estimate token count for a string (char/4 heuristic with 20% safety margin).
fn estimate_str_tokens(s: &str) -> u64 {
    ((s.len() as f64 / 4.0) * 1.2) as u64
}

/// Estimate total tokens for a message list.
fn estimate_messages_tokens(messages: &[Message]) -> u64 {
    messages.iter().map(|m| {
        let mut tokens = estimate_str_tokens(&m.content);
        // Tool calls add tokens
        for tc in &m.tool_calls {
            tokens += estimate_str_tokens(&tc.name);
            tokens += estimate_str_tokens(&tc.arguments.to_string());
        }
        // Tool results add tokens
        for tr in &m.tool_results {
            tokens += estimate_str_tokens(&tr.output);
        }
        tokens
    }).sum()
}

/// Classify whether a message contains "real conversation" worth preserving
/// in a summary, vs tool scaffolding that can be discarded.
fn is_real_conversation(msg: &Message) -> bool {
    match msg.role {
        Role::User => {
            let content = msg.content.trim();
            // Skip empty, heartbeat-only, and pure tool result messages
            if content.is_empty() { return false; }
            if content == "HEARTBEAT_OK" { return false; }
            if content.starts_with("[heartbeat]") { return false; }
            // Messages with only tool results and no text are scaffolding
            if content.is_empty() && !msg.tool_results.is_empty() { return false; }
            true
        }
        Role::Assistant => {
            let content = msg.content.trim();
            if content.is_empty() && !msg.tool_calls.is_empty() {
                // Pure tool call with no text — scaffolding
                return false;
            }
            if content == "HEARTBEAT_OK" { return false; }
            true
        }
        Role::System => true, // System messages are always relevant
        Role::Tool => false,  // Pure tool results are scaffolding
    }
}

/// Phase 1: Strip tool result details from older messages.
/// Replaces verbose tool outputs with short summaries, preserving the tool name
/// and success/failure status but not the full output.
fn strip_tool_outputs(messages: &mut [Message], preserve_recent: usize) {
    let strip_count = messages.len().saturating_sub(preserve_recent);
    for msg in messages.iter_mut().take(strip_count) {
        for tr in &mut msg.tool_results {
            if tr.output.len() > 200 {
                let status = if tr.success { "succeeded" } else { "failed" };
                tr.output = format!(
                    "[tool {} — {} chars, {}]",
                    tr.call_id, tr.output.len(), status
                );
            }
        }
        // Tool call arguments are NEVER compacted — they represent what the
        // agent asked to do and must stay intact for the LLM to understand
        // its own history. Only tool RESULTS are compacted (above).
    }
}

/// Walk split_point backward until it lands on a safe turn boundary —
/// one that does NOT bisect a tool_call/tool_result pair.
///
/// A safe boundary is: split_point == 0, or messages[split_point] is a User
/// message, or messages[split_point] is an Assistant message whose tool_calls
/// (if any) are ALL satisfied by tool_results in older[..split_point].
///
/// The unsafe case we're preventing: messages[split_point] is a Tool message
/// with tool_results whose call_ids live in an Assistant message at
/// older[split_point-1] or earlier — summarizing older drops those tool_calls
/// and leaves the tool_results as orphans on every subsequent LLM call.
fn adjust_split_to_turn_boundary(messages: &[Message], initial_split: usize) -> usize {
    let mut split = initial_split;
    while split > 0 && split < messages.len() {
        let msg = &messages[split];
        let safe = match msg.role {
            // User message is always a safe turn boundary
            Role::User => true,
            // System message is safe (shouldn't be mid-sequence anyway)
            Role::System => true,
            // Tool message at the boundary => orphaned results ahead. Walk back.
            Role::Tool => false,
            // Assistant message: safe iff it has no tool_calls OR its tool_calls
            // are all satisfied by tool_results strictly before `split`
            // (i.e. we're not about to summarize the calls and keep the results).
            Role::Assistant => {
                if msg.tool_calls.is_empty() {
                    true
                } else {
                    // Unsafe: tool_calls at boundary usually have results AFTER them in `recent`.
                    // Walk back to find a prior User/clean-Assistant boundary.
                    false
                }
            }
        };
        if safe {
            return split;
        }
        split -= 1;
    }
    split
}

/// Phase 2: Summarize older messages into a compact summary via LLM.
pub async fn compact_session_history(
    messages: &[Message],
    config: &CompactionConfig,
    llm: &LlmClient,
) -> Result<CompactionResult> {
    if !config.enabled {
        return Ok(CompactionResult {
            compacted: false,
            messages_before: messages.len(),
            messages_after: messages.len(),
            tokens_before: 0,
            tokens_after: 0,
            messages: messages.to_vec(),
        });
    }

    let tokens_before = estimate_messages_tokens(messages);
    let threshold = (config.max_context_tokens as f64 * config.compaction_threshold) as u64;

    // Compact if EITHER token threshold exceeded OR message count > 2x preserve count.
    // Message-count trigger catches short-message sessions (Discord chat) where tokens
    // stay low but the echo chamber builds from accumulated assistant responses.
    let message_count_trigger = messages.len() > 50; // was recent_preserve*2 (=10), fired too early

    if tokens_before < threshold && !message_count_trigger {
        debug!(
            "Session compaction skipped: {} tokens < {} threshold, {} msgs < {} trigger",
            tokens_before, threshold, messages.len(), config.recent_preserve * 2
        );
        return Ok(CompactionResult {
            compacted: false,
            messages_before: messages.len(),
            messages_after: messages.len(),
            tokens_before,
            tokens_after: tokens_before,
            messages: messages.to_vec(),
        });
    }

    info!(
        "Session compaction triggered: {} tokens > {} threshold ({} messages)",
        tokens_before, threshold, messages.len()
    );

    // Split into older (to summarize) and recent (to preserve)
    let preserve_count = config.recent_preserve.min(messages.len());
    let mut split_point = messages.len().saturating_sub(preserve_count);

    // Adjust split_point so we never bisect a tool_call / tool_result pair.
    // If recent[0] is a Tool message (or an Assistant message whose tool_calls
    // reference results that come later in `recent`), walk split_point backward
    // until we land on a clean turn boundary (User message, or Assistant with
    // no pending tool_calls). This prevents orphaned tool_results in `recent`
    // that reference tool_calls we're about to summarize away.
    split_point = adjust_split_to_turn_boundary(messages, split_point);

    if split_point == 0 {
        // Nothing to compact — all messages are "recent"
        return Ok(CompactionResult {
            compacted: false,
            messages_before: messages.len(),
            messages_after: messages.len(),
            tokens_before,
            tokens_after: tokens_before,
            messages: messages.to_vec(),
        });
    }

    let older = &messages[..split_point];
    let recent = &messages[split_point..];

    // Phase 1: Strip tool outputs from older messages (for the summary input)
    let mut older_stripped = older.to_vec();
    strip_tool_outputs(&mut older_stripped, 0);

    // Filter to real conversation for the summary
    let conversation_for_summary: Vec<&Message> = older_stripped.iter()
        .filter(|m| is_real_conversation(m))
        .collect();

    if conversation_for_summary.is_empty() {
        // No real conversation to summarize — just strip tool outputs and return
        let mut result = older_stripped.clone();
        result.extend_from_slice(recent);
        let tokens_after = estimate_messages_tokens(&result);
        return Ok(CompactionResult {
            compacted: true,
            messages_before: messages.len(),
            messages_after: result.len(),
            tokens_before,
            tokens_after,
            messages: result,
        });
    }

    // Build transcript for summarization
    let mut transcript = String::new();
    for msg in &conversation_for_summary {
        let role = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::System => "System",
            Role::Tool => "Tool",
        };
        if !msg.content.trim().is_empty() {
            transcript.push_str(&format!("[{}]: {}\n", role, msg.content.trim()));
        }
    }

    // Generate summary via LLM
    let summary_prompt = format!(
        "You are a conversation summarizer. Create a concise summary of the conversation below.\n\n\
         The purpose of this summary is to provide continuity so you can continue to make progress \
         in a future context where the raw history will be replaced with this summary.\n\n\
         Focus on:\n\
         - Current state of work (what's done, what's pending)\n\
         - Key decisions made\n\
         - Next steps or open tasks\n\
         - Important context (names, IDs, file paths mentioned)\n\
         - Any errors or blockers encountered\n\n\
         Do NOT include:\n\
         - Repetitive status reports\n\
         - HEARTBEAT_OK messages\n\
         - Tool execution details (just outcomes)\n\
         - Duplicate information\n\n\
         Keep the summary under {} tokens. Be direct and factual.\n\n\
         --- CONVERSATION ---\n{}",
        config.summary_max_tokens, transcript
    );

    let summary_messages = vec![Message::user(&summary_prompt)];
    let summary = match llm.complete(&summary_messages, &[], None).await {
        Ok(response) => response.content,
        Err(e) => {
            warn!("Compaction summary failed (non-fatal): {}. Falling back to tool stripping only.", e);
            // Fallback: just strip tool outputs, don't summarize
            let mut result = older_stripped;
            result.extend_from_slice(recent);
            let tokens_after = estimate_messages_tokens(&result);
            return Ok(CompactionResult {
                compacted: true,
                messages_before: messages.len(),
                messages_after: result.len(),
                tokens_before,
                tokens_after,
                messages: result,
            });
        }
    };

    // Build compacted message list: summary + recent messages
    let summary_msg = Message::system(&format!(
        "[Session Summary — {} older messages compacted]\n{}",
        split_point, summary.trim()
    ));

    let mut result = vec![summary_msg];
    result.extend_from_slice(recent);

    // S94: Sanitize orphaned tool_results — remove tool_result entries whose
    // tool_use_id doesn't appear in any preceding assistant message's tool_calls.
    // This prevents Anthropic API 400 errors after compaction.
    {
        let mut valid_tool_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for msg in &result {
            for tc in &msg.tool_calls {
                valid_tool_ids.insert(tc.id.clone());
            }
        }
        for msg in result.iter_mut() {
            if !msg.tool_results.is_empty() {
                msg.tool_results.retain(|tr| valid_tool_ids.contains(&tr.call_id));
            }
        }
        // Remove messages that became empty after stripping orphaned tool_results
        result.retain(|msg| {
            !msg.content.is_empty() || !msg.tool_calls.is_empty() || !msg.tool_results.is_empty()
                || msg.role == Role::System
        });
    }

    let tokens_after = estimate_messages_tokens(&result);

    info!(
        "Session compacted: {} → {} messages, {} → {} tokens (saved {})",
        messages.len(), result.len(),
        tokens_before, tokens_after,
        tokens_before.saturating_sub(tokens_after)
    );

    Ok(CompactionResult {
        compacted: true,
        messages_before: messages.len(),
        messages_after: result.len(),
        tokens_before,
        tokens_after,
        messages: result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_msg(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
            tool_calls: vec![],
            tool_results: vec![],
            timestamp: Utc::now(),
            attachments: vec![],
            message_id: None,
            parent_id: None,
            thread_id: None,
            direction: Default::default(),
            channel_source: None,
            compaction_hint: Default::default(),
        }
    }

    #[test]
    fn test_estimate_tokens() {
        // 100 chars ≈ 25 tokens * 1.2 safety = 30
        let tokens = estimate_str_tokens("a]".repeat(50).as_str());
        assert!(tokens > 0);
    }

    #[test]
    fn test_is_real_conversation() {
        assert!(is_real_conversation(&make_msg(Role::User, "hello")));
        assert!(is_real_conversation(&make_msg(Role::Assistant, "hi there")));
        assert!(!is_real_conversation(&make_msg(Role::User, "HEARTBEAT_OK")));
        assert!(!is_real_conversation(&make_msg(Role::Assistant, "HEARTBEAT_OK")));
        assert!(!is_real_conversation(&make_msg(Role::User, "")));
        assert!(is_real_conversation(&make_msg(Role::System, "you are zeus")));
        assert!(!is_real_conversation(&make_msg(Role::Tool, "result")));
    }

    #[test]
    fn test_strip_tool_outputs() {
        let mut msgs = vec![
            make_msg(Role::User, "hello"),
            {
                let mut m = make_msg(Role::Assistant, "running tool");
                m.tool_results.push(zeus_core::ToolResult {
                    call_id: "tc1".to_string(),
                    success: true,
                    output: "x".repeat(500),
                });
                m
            },
            make_msg(Role::User, "thanks"),
        ];
        strip_tool_outputs(&mut msgs, 1);
        // Last message preserved, first two stripped
        assert!(msgs[1].tool_results[0].output.len() < 100);
        assert_eq!(msgs[2].content, "thanks"); // preserved
    }

    #[test]
    fn test_compact_needs_threshold() {
        // Verify that small message sets don't trigger compaction
        let config = CompactionConfig {
            max_context_tokens: 180_000,
            compaction_threshold: 0.8,
            recent_preserve: 5,
            summary_max_tokens: 2000,
            enabled: true,
        };
        let msgs = vec![
            make_msg(Role::User, "hello"),
            make_msg(Role::Assistant, "hi"),
        ];
        let tokens = estimate_messages_tokens(&msgs);
        let threshold = (config.max_context_tokens as f64 * config.compaction_threshold) as u64;
        // Small messages should be way below threshold
        assert!(tokens < threshold);
    }

    #[test]
    fn test_adjust_split_avoids_bisecting_tool_pair() {
        // Sequence: [User, Assistant(tool_calls=[tc1]), Tool(results=[tc1]), User, Assistant]
        // If initial split lands on idx 2 (the Tool message), it would orphan tc1's result.
        // Expected: walk back to idx 1 or idx 0 (the prior User boundary).
        let mut msgs = vec![
            make_msg(Role::User, "first"),
            {
                let mut m = make_msg(Role::Assistant, "");
                m.tool_calls.push(zeus_core::ToolCall {
                    id: "tc1".into(),
                    name: "shell".into(),
                    arguments: serde_json::json!({}),
                });
                m
            },
            {
                let mut m = make_msg(Role::Tool, "");
                m.tool_results.push(zeus_core::ToolResult {
                    call_id: "tc1".into(),
                    success: true,
                    output: "ok".into(),
                });
                m
            },
            make_msg(Role::User, "second"),
            make_msg(Role::Assistant, "done"),
        ];

        // Initial split at 2 (Tool msg) — unsafe, would orphan tc1.
        let adjusted = adjust_split_to_turn_boundary(&msgs, 2);
        assert!(
            adjusted == 0 || (adjusted < msgs.len() && matches!(msgs[adjusted].role, Role::User)),
            "adjusted split {} should land on a User boundary or 0, got role {:?}",
            adjusted, msgs.get(adjusted).map(|m| &m.role)
        );

        // Initial split at 1 (Assistant with tool_calls) — also unsafe.
        let adjusted = adjust_split_to_turn_boundary(&msgs, 1);
        assert_eq!(adjusted, 0, "assistant-with-tool_calls boundary should walk back to 0");

        // Initial split at 3 (User) — already safe.
        let adjusted = adjust_split_to_turn_boundary(&msgs, 3);
        assert_eq!(adjusted, 3);

        // Split at 0 — trivially safe.
        let adjusted = adjust_split_to_turn_boundary(&msgs, 0);
        assert_eq!(adjusted, 0);

        // Split at len (nothing to compact) — returned as-is.
        let len = msgs.len();
        let adjusted = adjust_split_to_turn_boundary(&msgs, len);
        assert_eq!(adjusted, len);

        // Assistant with no tool_calls is a safe boundary.
        msgs.push(make_msg(Role::Assistant, "trailing"));
        let last_idx = msgs.len() - 1;
        let adjusted = adjust_split_to_turn_boundary(&msgs, last_idx);
        assert_eq!(adjusted, last_idx);
    }

    #[test]
    fn test_disabled_config() {
        let config = CompactionConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(!config.enabled);
    }
}
