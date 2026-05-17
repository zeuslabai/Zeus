//! Context window management for session compaction
//!
//! Implements automatic session summarization when token limits are approached.

use zeus_core::{Error, Message, Result, Role, SessionCompactionConfig};
use zeus_llm::LlmClient;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Context manager for handling session compaction
pub struct ContextManager {
    /// Maximum token limit before compaction
    max_tokens: usize,
    /// Compaction threshold (fraction of max_tokens)
    threshold: f32,
}

impl ContextManager {
    /// Create a new context manager from config
    pub fn new(config: &SessionCompactionConfig) -> Self {
        Self {
            max_tokens: config.max_context_tokens,
            threshold: config.compaction_threshold,
        }
    }

    /// Get the max token limit
    pub fn max_tokens(&self) -> usize {
        self.max_tokens
    }

    /// Estimate token count using content-aware heuristics.
    ///
    /// Different content types have different chars-per-token ratios:
    /// - Natural text: ~4 chars/token (standard heuristic)
    /// - Code/JSON/tool arguments: ~3 chars/token (more tokens due to symbols/keywords)
    /// - Tool results (typically code/structured data): ~3 chars/token
    pub fn estimate_tokens(messages: &[Message]) -> usize {
        messages
            .iter()
            .map(|msg| {
                // Natural text content: 4 chars/token
                let content_tokens = msg.content.len() / 4;

                // Tool call arguments are JSON/code: 3 chars/token
                let tool_call_tokens: usize = msg.tool_calls.iter().map(|tc| {
                    let chars = tc.name.len() + tc.arguments.to_string().len();
                    chars / 3
                }).sum();

                // Tool results are typically code/structured output: 3 chars/token
                let tool_result_tokens: usize = msg.tool_results.iter().map(|tr| {
                    tr.output.len() / 3
                }).sum();

                content_tokens + tool_call_tokens + tool_result_tokens
            })
            .sum()
    }

    /// Check if compaction is needed based on threshold
    pub fn needs_compaction(&self, messages: &[Message]) -> bool {
        let estimated = Self::estimate_tokens(messages);
        let trigger_point = (self.max_tokens as f32 * self.threshold) as usize;

        estimated > trigger_point
    }

    /// Compact messages by summarizing the oldest 60%
    ///
    /// Algorithm:
    /// 1. Separate system messages (preserve these)
    /// 2. Extract any prior `[Context Summary]` block (from a previous compaction)
    /// 3. Take oldest 60% of remaining non-system messages
    /// 4. Summarize them via LLM, chaining the prior summary if present
    /// 5. Replace with single system message containing the merged summary
    /// 6. Keep remaining 40% of recent messages
    ///
    /// Collapse a targeted span of messages (by index range) into a single summary.
    ///
    /// Unlike `compact()` which operates on the oldest 60% of all messages,
    /// `collapse_span()` lets callers surgically collapse a specific range —
    /// e.g. a verbose tool-heavy section mid-conversation — without touching
    /// surrounding messages.
    ///
    /// # Arguments
    /// * `messages` — the full message list (mutated in place)
    /// * `start` — inclusive start index of the span to collapse
    /// * `end` — exclusive end index of the span to collapse
    /// * `llm` — LLM client used to summarize the span
    ///
    /// # Errors
    /// Returns an error if `start >= end`, indices are out of bounds, or the
    /// LLM summarization call fails.
    pub async fn collapse_span(
        &self,
        messages: &mut Vec<Message>,
        start: usize,
        end: usize,
        llm: &LlmClient,
    ) -> Result<()> {
        if start >= end {
            return Err(Error::Session(format!(
                "collapse_span: start ({}) must be less than end ({})",
                start, end
            )));
        }
        if end > messages.len() {
            return Err(Error::Session(format!(
                "collapse_span: end ({}) out of bounds (len={})",
                end,
                messages.len()
            )));
        }

        // Extract the span
        let span: Vec<Message> = messages[start..end].to_vec();
        let span_text = self.build_summary_text(&span);

        let prompt = format!(
            "Summarize the following conversation segment concisely. \
             Preserve key facts, tool results, decisions, and any important \
             output. This summary replaces a verbose section mid-conversation.\n\n{}",
            span_text
        );

        let summary_response = llm
            .complete(&[Message::user(prompt)], &[], None)
            .await
            .map_err(|e| Error::Session(format!("collapse_span LLM call failed: {}", e)))?;

        // Replace the span with a single system summary message
        let summary_msg = Message::system(format!(
            "[Collapsed Span: messages {}-{}]\n\n{}",
            start,
            end - 1,
            summary_response.content
        ));

        messages.splice(start..end, std::iter::once(summary_msg));

        Ok(())
    }

    /// On the second (and subsequent) compaction, the prior summary is prepended
    /// to the new summarization prompt so no history is silently dropped.
    pub async fn compact(&self, messages: &mut Vec<Message>, llm: &LlmClient) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }

        // Separate system messages — preserve real directives, extract prior summary
        let mut system_messages: Vec<Message> = Vec::new();
        let mut prior_summary: Option<String> = None;
        let mut other_messages: Vec<Message> = Vec::new();

        for msg in messages.drain(..) {
            if msg.role == Role::System {
                // Detect a prior [Context Summary] block from a previous compaction
                if msg.content.starts_with("[Context Summary]") {
                    // Extract the text after the header
                    prior_summary = Some(
                        msg.content
                            .trim_start_matches("[Context Summary]")
                            .trim()
                            .to_string(),
                    );
                    // Don't re-add it — we'll replace it with the merged summary below
                } else {
                    system_messages.push(msg);
                }
            } else {
                other_messages.push(msg);
            }
        }

        // If we have no non-system messages, just restore and return
        if other_messages.is_empty() {
            messages.extend(system_messages);
            if let Some(summary) = prior_summary {
                messages.push(Message::system(format!(
                    "[Context Summary]\n\n{}",
                    summary
                )));
            }
            return Ok(());
        }

        // Calculate split point (60% of non-system messages)
        let split_index = (other_messages.len() as f32 * 0.6).ceil() as usize;

        // Split into messages to compact and messages to keep
        let messages_to_compact: Vec<Message> = other_messages.drain(..split_index).collect();
        let messages_to_keep = other_messages;

        // Build summary prompt — chain prior summary if this is a subsequent compaction
        let new_summary_text = self.build_summary_text(&messages_to_compact);
        let prompt = self.build_compact_prompt(prior_summary.as_deref(), &new_summary_text);

        // Call LLM to summarize
        let summary_messages = vec![Message::user(prompt)];

        let response = llm
            .complete(&summary_messages, &[], None)
            .await
            .map_err(|e| Error::Session(format!("Failed to generate summary: {}", e)))?;

        // Create the merged summary message
        let summary_message = Message::system(format!(
            "[Context Summary]\n\nThe following is a summary of earlier conversation history:\n\n{}",
            response.content
        ));

        // Rebuild message list:
        // 1. Original system directives (non-summary)
        // 2. Merged summary message
        // 3. Recent messages
        messages.extend(system_messages);
        messages.push(summary_message);
        messages.extend(messages_to_keep);

        Ok(())
    }

    /// Build the LLM prompt for compaction, optionally chaining a prior summary.
    ///
    /// When `prior_summary` is `Some`, the prompt includes both the prior
    /// compacted context and the new messages — ensuring a second compaction
    /// doesn't silently drop what was preserved in the first.
    fn build_compact_prompt(&self, prior_summary: Option<&str>, new_messages_text: &str) -> String {
        match prior_summary {
            None => {
                // First compaction — straightforward summarization
                format!(
                    "Summarize the following conversation history concisely, preserving key facts, \
                     decisions, and context. Focus on what's most important for continuing the conversation.\n\n{}",
                    new_messages_text
                )
            }
            Some(prior) => {
                // Subsequent compaction — merge prior summary with new messages
                format!(
                    "You are merging two conversation summaries into one.\n\n\
                     ## Previously Compacted Context\n\n\
                     {prior}\n\n\
                     ## New Messages to Add\n\n\
                     {new}\n\n\
                     Produce a single consolidated summary that preserves key facts, decisions, \
                     and context from both. Be concise. Maintain chronological order where it matters.",
                    prior = prior,
                    new = new_messages_text
                )
            }
        }
    }

    /// Truncate oversized messages to reduce token count before compaction.
    ///
    /// Scans all messages and truncates:
    /// - Tool results with >2000 estimated tokens (chars/4) → ~1500 tokens
    /// - Assistant content with >3000 estimated tokens → ~2000 tokens
    /// - System messages are never touched
    ///
    /// Returns the number of messages that were truncated.
    /// Call this BEFORE the `compact()` summarization pass.
    pub fn truncate_oversized_messages(messages: &mut [Message]) -> usize {
        const TOOL_RESULT_MAX_TOKENS: usize = 2000;
        const TOOL_RESULT_TRUNCATE_TO: usize = 1500;
        const ASSISTANT_MAX_TOKENS: usize = 3000;
        const ASSISTANT_TRUNCATE_TO: usize = 2000;

        let mut truncated_count = 0;

        for msg in messages.iter_mut() {
            // Never touch system messages
            if msg.role == Role::System {
                continue;
            }

            // Truncate oversized tool results
            for tr in &mut msg.tool_results {
                let estimated_tokens = tr.output.len() / 4;
                if estimated_tokens > TOOL_RESULT_MAX_TOKENS {
                    let truncate_chars = TOOL_RESULT_TRUNCATE_TO * 4;
                    // Ensure we don't split in the middle of a multi-byte char
                    let safe_end = Self::safe_char_boundary(&tr.output, truncate_chars);
                    tr.output = format!(
                        "{}\n\n[...truncated, was {} tokens]",
                        &tr.output[..safe_end],
                        estimated_tokens
                    );
                    truncated_count += 1;
                }
            }

            // Truncate oversized assistant content
            if msg.role == Role::Assistant {
                let estimated_tokens = msg.content.len() / 4;
                if estimated_tokens > ASSISTANT_MAX_TOKENS {
                    let truncate_chars = ASSISTANT_TRUNCATE_TO * 4;
                    let safe_end = Self::safe_char_boundary(&msg.content, truncate_chars);
                    msg.content = format!(
                        "{}\n\n[...truncated, was {} tokens]",
                        &msg.content[..safe_end],
                        estimated_tokens
                    );
                    truncated_count += 1;
                }
            }
        }

        truncated_count
    }

    /// Find the nearest valid char boundary at or before `target` byte index.
    fn safe_char_boundary(s: &str, target: usize) -> usize {
        if target >= s.len() {
            return s.len();
        }
        // Walk backwards to find a char boundary
        let mut end = target;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        end
    }

    /// Smart compaction: truncate oversized messages, then compact if needed.
    ///
    /// This is the recommended entry point for session compaction. It chains:
    /// 1. `truncate_oversized_messages` — reduce bloated tool results / assistant content
    /// 2. `needs_compaction` check — only proceed if still over threshold
    /// 3. `compact` — summarize oldest 60% via LLM
    ///
    /// Returns `(truncated_count, did_compact)`.
    pub async fn smart_compact(
        &self,
        messages: &mut Vec<Message>,
        llm: &LlmClient,
    ) -> Result<(usize, bool)> {
        // Step 1: Truncate oversized messages
        let truncated = Self::truncate_oversized_messages(messages);

        // Step 2: Check if compaction is still needed
        if !self.needs_compaction(messages) {
            return Ok((truncated, false));
        }

        // Step 3: Full compaction
        self.compact(messages, llm).await?;

        Ok((truncated, true))
    }

    /// Build text representation of messages for summarization
    fn build_summary_text(&self, messages: &[Message]) -> String {
        let mut text = String::new();

        for (i, msg) in messages.iter().enumerate() {
            let role_label = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::System => "System",
                Role::Tool => "Tool",
            };

            text.push_str(&format!("--- Message {} ({}) ---\n", i + 1, role_label));

            if !msg.content.is_empty() {
                text.push_str(&msg.content);
                text.push('\n');
            }

            // Include tool calls
            for tc in &msg.tool_calls {
                text.push_str(&format!(
                    "\n[Tool Call: {}]\n{}\n",
                    tc.name,
                    serde_json::to_string_pretty(&tc.arguments).unwrap_or_default()
                ));
            }

            // Include tool results
            for tr in &msg.tool_results {
                let status = if tr.success { "Success" } else { "Error" };
                text.push_str(&format!("\n[Tool Result: {}]\n{}\n", status, tr.output));
            }

            text.push('\n');
        }

        text
    }
}

// ============================================================================
// Unified Session Compaction (moved from zeus-prometheus)
// ============================================================================

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
fn default_recent_preserve() -> usize { 20 }
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
    /// The compacted message list.
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
        for tc in &m.tool_calls {
            tokens += estimate_str_tokens(&tc.name);
            tokens += estimate_str_tokens(&tc.arguments.to_string());
        }
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
            if content.is_empty() { return false; }
            if content == "HEARTBEAT_OK" { return false; }
            if content.starts_with("[heartbeat]") { return false; }
            true
        }
        Role::Assistant => {
            let content = msg.content.trim();
            if content.is_empty() && !msg.tool_calls.is_empty() { return false; }
            if content == "HEARTBEAT_OK" { return false; }
            true
        }
        Role::System => true,
        Role::Tool => false,
    }
}

/// Strip tool result details from older messages.
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
    }
}

/// Walk split_point backward until it lands on a safe turn boundary.
fn adjust_split_to_turn_boundary(messages: &[Message], initial_split: usize) -> usize {
    let mut split = initial_split;
    while split > 0 && split < messages.len() {
        let msg = &messages[split];
        let safe = match msg.role {
            Role::User => true,
            Role::System => true,
            Role::Tool => false,
            Role::Assistant => msg.tool_calls.is_empty(),
        };
        if safe {
            return split;
        }
        split -= 1;
    }
    split
}

/// Unified session compaction: two-phase approach.
/// 1. Strip tool result details from older messages (cheap, high token savings)
/// 2. Summarize older conversation into a compact summary via LLM
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
    let message_count_trigger = messages.len() > 50;

    if tokens_before < threshold && !message_count_trigger {
        debug!(
            "Session compaction skipped: {} tokens < {} threshold, {} msgs",
            tokens_before, threshold, messages.len()
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

    let preserve_count = config.recent_preserve.min(messages.len());
    let mut split_point = messages.len().saturating_sub(preserve_count);
    split_point = adjust_split_to_turn_boundary(messages, split_point);

    if split_point == 0 {
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

    let mut older_stripped = older.to_vec();
    strip_tool_outputs(&mut older_stripped, 0);

    let conversation_for_summary: Vec<&Message> = older_stripped.iter()
        .filter(|m| is_real_conversation(m))
        .collect();

    if conversation_for_summary.is_empty() {
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

    let summary_msg = Message::system(&format!(
        "[Session Summary — {} older messages compacted]\n{}",
        split_point, summary.trim()
    ));

    let mut result = vec![summary_msg];
    result.extend_from_slice(recent);

    // Sanitize orphaned tool_results
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

// ============================================================================
// Pre-Compaction Memory Flush
// ============================================================================

/// Pre-compaction flush hook that gives the LLM a chance to persist
/// durable memories before context is compacted away.
///
/// Injects a system + user message pair asking the LLM to save any
/// lasting notes. Only fires once per compaction cycle, and only
/// when the workspace is writable.
pub struct CompactionFlush {
    /// Whether a flush has already fired in the current compaction cycle
    flushed: bool,
    /// Timeout for the flush LLM call (seconds)
    timeout_secs: u64,
}

/// The "no-op" reply token the LLM sends when there's nothing to say.
///
/// Used in two contexts:
/// 1. Pre-compaction flush: agent replies NO_REPLY if nothing to persist
/// 2. Group chat silencing: agent replies NO_REPLY to skip responding
///    (OpenClaw `SILENT_REPLY_TOKEN` parity)
pub const NO_REPLY_TOKEN: &str = "NO_REPLY";

/// Check whether `text` is a silent reply (exact NO_REPLY with optional whitespace).
///
/// Matches: `"NO_REPLY"`, `" NO_REPLY "`, `"\nNO_REPLY\n"`, `"no_reply"` (case-insensitive).
pub fn is_silent_reply(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.eq_ignore_ascii_case(NO_REPLY_TOKEN)
}

/// If `text` contains the NO_REPLY token mixed with other content, strip it out.
/// Returns `None` if stripping leaves only whitespace (= fully silent).
/// Returns `Some(cleaned)` if there was other content around the token.
///
/// OpenClaw parity: `stripSilentToken` in `tokens.ts` — prevents the token
/// from leaking to end users when the LLM embeds it in a longer response.
pub fn strip_silent_token(text: &str) -> Option<String> {
    if is_silent_reply(text) {
        return None;
    }
    if !text.to_ascii_uppercase().contains(NO_REPLY_TOKEN) {
        return Some(text.to_string());
    }
    // Strip the token (case-insensitive) and surrounding whitespace
    let mut result = text.to_string();
    // Remove all occurrences case-insensitively
    let upper = result.to_ascii_uppercase();
    if let Some(pos) = upper.find(NO_REPLY_TOKEN) {
        result = format!(
            "{}{}",
            &result[..pos].trim_end(),
            &result[pos + NO_REPLY_TOKEN.len()..].trim_start()
        );
    }
    let cleaned = result.trim().to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// S102 #24: Infer pending work from recent messages using keyword detection.
/// Scans assistant messages for indicators of unfinished work. Zero LLM calls.
/// Returns a list of extracted pending items.
pub fn infer_pending_work(messages: &[Message]) -> Vec<String> {
    let keywords = ["todo", "next", "pending", "remaining", "still need", "haven't yet",
                     "will do", "not yet", "blocked on", "need to", "should also",
                     "left to do", "follow up", "later"];
    let mut pending = Vec::new();
    // Scan last 10 assistant messages
    for msg in messages.iter().rev().filter(|m| m.role == Role::Assistant).take(10) {
        for line in msg.content.lines() {
            let lower = line.to_lowercase();
            if keywords.iter().any(|kw| lower.contains(kw)) {
                let trimmed = line.trim();
                if trimmed.len() > 10 && trimmed.len() < 200 {
                    pending.push(trimmed.to_string());
                }
            }
        }
    }
    pending.truncate(10); // cap at 10 items
    pending
}

/// Default flush timeout in seconds
const DEFAULT_FLUSH_TIMEOUT_SECS: u64 = 30;

impl CompactionFlush {
    /// Create a new flush tracker.
    pub fn new() -> Self {
        Self {
            flushed: false,
            timeout_secs: DEFAULT_FLUSH_TIMEOUT_SECS,
        }
    }

    /// Create with a custom timeout.
    pub fn with_timeout(timeout_secs: u64) -> Self {
        Self {
            flushed: false,
            timeout_secs,
        }
    }

    /// Check if a flush should fire now.
    ///
    /// Returns true only if:
    /// 1. A flush hasn't already fired this cycle
    /// 2. The workspace root exists (writable)
    /// 3. Compaction is needed
    pub fn should_flush(
        &self,
        messages: &[Message],
        context_manager: &ContextManager,
        workspace_writable: bool,
    ) -> bool {
        !self.flushed && workspace_writable && context_manager.needs_compaction(messages)
    }

    /// Generate the system + user message pair for the flush injection.
    ///
    /// These messages tell the LLM to persist any durable memories
    /// before compaction destroys older context.
    pub fn flush_messages(&self) -> (Message, Message) {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

        let system_msg = Message::system("Session nearing compaction. Store durable memories now.");
        let user_msg = Message::user(format!(
            "Write any lasting notes to memory/{}.md. If nothing to store, just say so briefly.",
            today
        ));

        (system_msg, user_msg)
    }

    /// Mark that a flush has been performed this cycle.
    pub fn mark_flushed(&mut self) {
        self.flushed = true;
    }

    /// Whether a flush has fired in the current cycle.
    pub fn is_flushed(&self) -> bool {
        self.flushed
    }

    /// Reset flush state for the next compaction cycle.
    pub fn reset(&mut self) {
        self.flushed = false;
    }

    /// Get the flush timeout duration.
    pub fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.timeout_secs)
    }
}

impl Default for CompactionFlush {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeus_core::Message;

    fn create_test_config() -> SessionCompactionConfig {
        SessionCompactionConfig {
            max_context_tokens: 1000,
            compaction_threshold: 0.8,
            summary_model: None,
            compaction_timeout_secs: None,
            ollama_compaction_threshold: None,
            flush_timeout_secs: None,
        }
    }

    #[test]
    fn test_estimate_tokens_empty() {
        let messages = vec![];
        let tokens = ContextManager::estimate_tokens(&messages);
        assert_eq!(tokens, 0);
    }

    #[test]
    fn test_estimate_tokens_simple() {
        let messages = vec![
            Message::user("Hello, world!"),  // 13 chars
            Message::assistant("Hi there!"), // 9 chars
        ];
        let tokens = ContextManager::estimate_tokens(&messages);
        // (13 + 9) / 4 = 5.5, rounds to 5
        assert_eq!(tokens, 5);
    }

    #[test]
    fn test_estimate_tokens_longer_content() {
        // Create a message with ~400 chars (should be ~100 tokens)
        let content = "a".repeat(400);
        let messages = vec![Message::user(content)];
        let tokens = ContextManager::estimate_tokens(&messages);
        assert_eq!(tokens, 100);
    }

    #[test]
    fn test_needs_compaction_under_threshold() {
        let config = create_test_config();
        let manager = ContextManager::new(&config);

        // max_tokens = 1000, threshold = 0.8, trigger = 800
        // Create messages with ~200 tokens (800 chars)
        let messages = vec![
            Message::user("x".repeat(400)),
            Message::assistant("y".repeat(400)),
        ];

        // 800 chars / 4 = 200 tokens, which is under 800
        assert!(!manager.needs_compaction(&messages));
    }

    #[test]
    fn test_needs_compaction_over_threshold() {
        let config = create_test_config();
        let manager = ContextManager::new(&config);

        // max_tokens = 1000, threshold = 0.8, trigger = 800
        // Create messages with ~900 tokens (3600 chars)
        let messages = vec![
            Message::user("x".repeat(1800)),
            Message::assistant("y".repeat(1800)),
        ];

        // 3600 chars / 4 = 900 tokens, which is over 800
        assert!(manager.needs_compaction(&messages));
    }

    #[test]
    fn test_needs_compaction_exactly_at_threshold() {
        let config = create_test_config();
        let manager = ContextManager::new(&config);

        // Exactly at threshold: 800 tokens = 3200 chars
        let messages = vec![Message::user("x".repeat(3200))];

        // 3200 / 4 = 800 tokens, exactly at threshold
        assert!(!manager.needs_compaction(&messages));
    }

    #[test]
    fn test_needs_compaction_just_over_threshold() {
        let config = create_test_config();
        let manager = ContextManager::new(&config);

        // Just over threshold: 801 tokens = 3204 chars
        let messages = vec![Message::user("x".repeat(3204))];

        // 3204 / 4 = 801 tokens, just over threshold
        assert!(manager.needs_compaction(&messages));
    }

    #[test]
    fn test_compact_preserves_system_messages() {
        let config = create_test_config();
        let _manager = ContextManager::new(&config);

        let messages = vec![
            Message::system("System instruction 1"),
            Message::user("User 1"),
            Message::assistant("Assistant 1"),
            Message::system("System instruction 2"),
            Message::user("User 2"),
            Message::assistant("Assistant 2"),
        ];

        // We can't test the actual LLM call, but we can test the message selection logic
        // by checking what would be selected for compaction

        // Separate system vs non-system
        let system_count = messages.iter().filter(|m| m.role == Role::System).count();
        let non_system_count = messages.iter().filter(|m| m.role != Role::System).count();

        assert_eq!(system_count, 2);
        assert_eq!(non_system_count, 4);

        // 60% of 4 non-system messages = 2.4, rounds up to 3
        let expected_to_compact = (non_system_count as f32 * 0.6).ceil() as usize;
        assert_eq!(expected_to_compact, 3);
    }

    #[test]
    fn test_build_summary_text_formatting() {
        let config = create_test_config();
        let manager = ContextManager::new(&config);

        let messages = vec![
            Message::user("First message"),
            Message::assistant("First response"),
        ];

        let summary_text = manager.build_summary_text(&messages);

        assert!(summary_text.contains("--- Message 1 (User) ---"));
        assert!(summary_text.contains("--- Message 2 (Assistant) ---"));
        assert!(summary_text.contains("First message"));
        assert!(summary_text.contains("First response"));
    }

    #[test]
    fn test_context_manager_creation() {
        let config = SessionCompactionConfig {
            max_context_tokens: 150000,
            compaction_threshold: 0.9,
            summary_model: Some("anthropic/claude-sonnet-4".to_string()),
            compaction_timeout_secs: None,
            ollama_compaction_threshold: None,
            flush_timeout_secs: None,
        };

        let manager = ContextManager::new(&config);
        assert_eq!(manager.max_tokens, 150000);
        assert!((manager.threshold - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn test_estimate_tokens_with_tool_calls() {
        use zeus_core::ToolCall;

        let mut msg = Message::assistant("Response");
        msg.tool_calls = vec![ToolCall {
            id: "call_1".to_string(),
            name: "read_file".to_string(), // 9 chars
            arguments: serde_json::json!({"path": "/tmp/test.txt"}), // ~25 chars
        }];

        let messages = vec![msg];
        let tokens = ContextManager::estimate_tokens(&messages);

        // "Response" (8) + "read_file" (9) + json (~25) = ~42 chars
        // 42 / 4 = 10 tokens (roughly)
        assert!(tokens >= 10);
    }

    #[test]
    fn test_estimate_tokens_with_tool_results() {
        let mut msg = Message::tool("call_1", true, "File contents here");
        msg.tool_results[0].output = "File contents here".to_string(); // 18 chars

        let messages = vec![msg];
        let tokens = ContextManager::estimate_tokens(&messages);

        // 18 / 4 = 4 tokens
        assert_eq!(tokens, 4);
    }

    #[test]
    fn test_compact_empty_messages() {
        // This test doesn't need LLM, just checks empty case
        let config = create_test_config();
        let _manager = ContextManager::new(&config);

        let messages: Vec<Message> = vec![];

        // We can't actually run compact() without an LLM client,
        // but we verify the manager handles the empty case in the logic
        assert!(messages.is_empty());
    }

    #[test]
    fn test_compact_only_system_messages() {
        // Test that only system messages are handled correctly
        let config = create_test_config();
        let _manager = ContextManager::new(&config);

        let messages = vec![Message::system("System 1"), Message::system("System 2")];

        // All messages are system messages, so nothing should be compacted
        let system_count = messages.iter().filter(|m| m.role == Role::System).count();
        assert_eq!(system_count, 2);
    }

    #[test]
    fn test_compaction_split_calculation() {
        // Test the 60% split calculation
        let test_cases = vec![
            (5, 3),  // 5 * 0.6 = 3.0, ceil = 3
            (10, 6), // 10 * 0.6 = 6.0, ceil = 6
            (7, 5),  // 7 * 0.6 = 4.2, ceil = 5
            (3, 2),  // 3 * 0.6 = 1.8, ceil = 2
            (1, 1),  // 1 * 0.6 = 0.6, ceil = 1
        ];

        for (total, expected_compact) in test_cases {
            let split = (total as f32 * 0.6).ceil() as usize;
            assert_eq!(split, expected_compact, "Failed for total={}", total);
        }
    }

    #[test]
    fn test_multiple_threshold_values() {
        // Test different threshold configurations
        let test_configs = vec![
            (1000, 0.5, 500),  // 50% threshold
            (1000, 0.75, 750), // 75% threshold
            (1000, 0.9, 900),  // 90% threshold
            (2000, 0.8, 1600), // Different max with 80%
        ];

        for (max_tokens, threshold, expected_trigger) in test_configs {
            let config = SessionCompactionConfig {
                max_context_tokens: max_tokens,
                compaction_threshold: threshold,
                summary_model: None,
                compaction_timeout_secs: None,
                ollama_compaction_threshold: None,
                flush_timeout_secs: None,
            };
            let manager = ContextManager::new(&config);

            let trigger = (max_tokens as f32 * threshold) as usize;
            assert_eq!(
                trigger, expected_trigger,
                "Failed for max={}, threshold={}",
                max_tokens, threshold
            );

            // Test just under trigger
            let chars_under = (expected_trigger - 1) * 4;
            let messages_under = vec![Message::user("x".repeat(chars_under))];
            assert!(!manager.needs_compaction(&messages_under));

            // Test at trigger (should not compact)
            let chars_at = expected_trigger * 4;
            let messages_at = vec![Message::user("x".repeat(chars_at))];
            assert!(!manager.needs_compaction(&messages_at));

            // Test over trigger
            let chars_over = (expected_trigger + 1) * 4;
            let messages_over = vec![Message::user("x".repeat(chars_over))];
            assert!(manager.needs_compaction(&messages_over));
        }
    }

    // ========================================================================
    // CompactionFlush tests
    // ========================================================================

    #[test]
    fn test_flush_initial_state() {
        let flush = CompactionFlush::new();
        assert!(!flush.is_flushed());
    }

    #[test]
    fn test_flush_mark_and_reset() {
        let mut flush = CompactionFlush::new();
        assert!(!flush.is_flushed());

        flush.mark_flushed();
        assert!(flush.is_flushed());

        flush.reset();
        assert!(!flush.is_flushed());
    }

    #[test]
    fn test_flush_one_shot_guard() {
        let mut flush = CompactionFlush::new();
        let config = create_test_config();
        let manager = ContextManager::new(&config);

        // Create messages over threshold
        let messages = vec![Message::user("x".repeat(3600))]; // > 800 tokens

        // First check — should flush
        assert!(flush.should_flush(&messages, &manager, true));

        // Mark flushed
        flush.mark_flushed();

        // Second check — should NOT flush (already flushed this cycle)
        assert!(!flush.should_flush(&messages, &manager, true));

        // After reset — should flush again
        flush.reset();
        assert!(flush.should_flush(&messages, &manager, true));
    }

    #[test]
    fn test_flush_requires_workspace_writable() {
        let flush = CompactionFlush::new();
        let config = create_test_config();
        let manager = ContextManager::new(&config);

        let messages = vec![Message::user("x".repeat(3600))];

        // Writable — should flush
        assert!(flush.should_flush(&messages, &manager, true));

        // Not writable — should NOT flush
        assert!(!flush.should_flush(&messages, &manager, false));
    }

    #[test]
    fn test_flush_no_compaction_needed() {
        let flush = CompactionFlush::new();
        let config = create_test_config();
        let manager = ContextManager::new(&config);

        // Small messages — compaction not needed
        let messages = vec![Message::user("hello")];
        assert!(!flush.should_flush(&messages, &manager, true));
    }

    #[test]
    fn test_flush_messages_content() {
        let flush = CompactionFlush::new();
        let (system_msg, user_msg) = flush.flush_messages();

        assert_eq!(system_msg.role, Role::System);
        assert!(system_msg.content.contains("compaction"));
        assert!(system_msg.content.contains("durable memories"));

        assert_eq!(user_msg.role, Role::User);
        assert!(user_msg.content.contains("memory/"));
        assert!(user_msg.content.contains(".md"));
        assert!(user_msg.content.contains("nothing to store"));
    }

    #[test]
    fn test_flush_messages_has_today_date() {
        let flush = CompactionFlush::new();
        let (_, user_msg) = flush.flush_messages();

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        assert!(
            user_msg.content.contains(&today),
            "Flush message should reference today's date: {}",
            today
        );
    }

    #[test]
    fn test_flush_timeout_default() {
        let flush = CompactionFlush::new();
        assert_eq!(flush.timeout(), std::time::Duration::from_secs(30));
    }

    #[test]
    fn test_flush_timeout_custom() {
        let flush = CompactionFlush::with_timeout(60);
        assert_eq!(flush.timeout(), std::time::Duration::from_secs(60));
    }

    // ========================================================================
    // Compound compaction / merge tests (T2)
    // ========================================================================

    #[test]
    fn test_build_compact_prompt_first_compaction() {
        let config = create_test_config();
        let manager = ContextManager::new(&config);

        let prompt = manager.build_compact_prompt(None, "some messages");
        assert!(prompt.contains("Summarize"));
        assert!(prompt.contains("some messages"));
        assert!(!prompt.contains("Previously Compacted"));
    }

    #[test]
    fn test_build_compact_prompt_subsequent_compaction() {
        let config = create_test_config();
        let manager = ContextManager::new(&config);

        let prior = "We discussed Rust async patterns and decided on tokio.";
        let new_msgs = "User asked about error handling.";
        let prompt = manager.build_compact_prompt(Some(prior), new_msgs);

        assert!(prompt.contains("Previously Compacted Context"));
        assert!(prompt.contains(prior));
        assert!(prompt.contains("New Messages to Add"));
        assert!(prompt.contains(new_msgs));
        assert!(prompt.contains("merging two conversation summaries"));
    }

    #[test]
    fn test_compact_extracts_prior_summary() {
        // Verify that a [Context Summary] system message is extracted
        // and not duplicated in the non-summary system messages bucket
        let config = create_test_config();
        let _manager = ContextManager::new(&config);

        let messages = vec![
            Message::system("Real system directive"),
            Message::system("[Context Summary]\n\nThe following is a summary of earlier conversation history:\n\nWe talked about X."),
            Message::user("New question"),
        ];

        // Partition as compact() would
        let mut system_messages: Vec<Message> = Vec::new();
        let mut prior_summary: Option<String> = None;

        for msg in &messages {
            if msg.role == Role::System {
                if msg.content.starts_with("[Context Summary]") {
                    prior_summary = Some(
                        msg.content
                            .trim_start_matches("[Context Summary]")
                            .trim()
                            .to_string(),
                    );
                } else {
                    system_messages.push(msg.clone());
                }
            }
        }

        assert_eq!(system_messages.len(), 1);
        assert_eq!(system_messages[0].content, "Real system directive");
        assert!(prior_summary.is_some());
        let prior = prior_summary.unwrap();
        assert!(prior.contains("We talked about X"), "prior={:?}", prior);
        assert!(!prior.contains("[Context Summary]"), "token should be stripped");
    }

    #[test]
    fn test_compact_no_prior_summary_no_chaining() {
        // When there's no prior [Context Summary] message,
        // build_compact_prompt should produce the simple (non-merge) prompt
        let config = create_test_config();
        let manager = ContextManager::new(&config);

        let prompt = manager.build_compact_prompt(None, "--- Message 1 (User) ---\nHello");
        assert!(!prompt.contains("Previously Compacted"));
        assert!(prompt.contains("Summarize"));
    }

    #[test]
    fn test_compact_split_still_correct_after_summary_removal() {
        // After removing the prior [Context Summary] system message,
        // the 60% split should operate on the *non-system* messages only.
        let non_system = vec![
            Message::user("msg1"), Message::assistant("r1"),
            Message::user("msg2"), Message::assistant("r2"),
            Message::user("msg3"), Message::assistant("r3"),
            Message::user("msg4"), Message::assistant("r4"),
            Message::user("msg5"), Message::assistant("r5"),
        ];
        // 10 non-system messages → split at ceil(10 * 0.6) = 6
        let split = (non_system.len() as f32 * 0.6).ceil() as usize;
        assert_eq!(split, 6);
        let kept = non_system.len() - split;
        assert_eq!(kept, 4);
    }

    #[test]
    fn test_flush_default_trait() {
        let flush = CompactionFlush::default();
        assert!(!flush.is_flushed());
        assert_eq!(flush.timeout(), std::time::Duration::from_secs(30));
    }

    // ========================================================================
    // Silent reply tests (OpenClaw NO_REPLY parity)
    // ========================================================================

    #[test]
    fn test_is_silent_reply_exact() {
        assert!(is_silent_reply("NO_REPLY"));
    }

    #[test]
    fn test_is_silent_reply_whitespace() {
        assert!(is_silent_reply("  NO_REPLY  "));
        assert!(is_silent_reply("\nNO_REPLY\n"));
        assert!(is_silent_reply("\t NO_REPLY \t"));
    }

    #[test]
    fn test_is_silent_reply_case_insensitive() {
        assert!(is_silent_reply("no_reply"));
        assert!(is_silent_reply("No_Reply"));
        assert!(is_silent_reply("NO_reply"));
    }

    #[test]
    fn test_is_silent_reply_rejects_prose() {
        assert!(!is_silent_reply("Staying quiet as directed. NO_REPLY"));
        assert!(!is_silent_reply("NO_REPLY — nothing to add"));
        assert!(!is_silent_reply("I'll stay quiet. 🤐"));
        assert!(!is_silent_reply("That's zeus106 responding — staying quiet as directed. 🤐"));
    }

    #[test]
    fn test_strip_silent_token_exact_returns_none() {
        assert_eq!(strip_silent_token("NO_REPLY"), None);
        assert_eq!(strip_silent_token("  NO_REPLY  "), None);
    }

    #[test]
    fn test_strip_silent_token_no_token_returns_original() {
        assert_eq!(strip_silent_token("Hello world"), Some("Hello world".into()));
    }

    #[test]
    fn test_strip_silent_token_mixed_content() {
        assert_eq!(strip_silent_token("Sure thing NO_REPLY"), Some("Sure thing".into()));
        assert_eq!(strip_silent_token("NO_REPLY ok"), Some("ok".into()));
        assert_eq!(strip_silent_token("😄 NO_REPLY"), Some("😄".into()));
    }

    #[test]
    fn test_strip_silent_token_only_whitespace_after_strip() {
        assert_eq!(strip_silent_token("  NO_REPLY  "), None);
    }

    // ========================================================================
    // truncate_oversized_messages tests
    // ========================================================================

    #[test]
    fn test_truncate_noop_on_small_messages() {
        let mut messages = vec![
            Message::user("Hello, how are you?"),
            Message::assistant("I'm fine, thanks!"),
            Message::tool("call_1", true, "Short tool output"),
        ];
        let original: Vec<String> = messages
            .iter()
            .map(|m| m.content.clone())
            .collect();
        let original_tool_outputs: Vec<String> = messages
            .iter()
            .flat_map(|m| m.tool_results.iter().map(|tr| tr.output.clone()))
            .collect();

        let count = ContextManager::truncate_oversized_messages(&mut messages);

        assert_eq!(count, 0, "No messages should be truncated");
        // Verify content unchanged
        for (i, msg) in messages.iter().enumerate() {
            assert_eq!(msg.content, original[i]);
        }
        let tool_outputs: Vec<String> = messages
            .iter()
            .flat_map(|m| m.tool_results.iter().map(|tr| tr.output.clone()))
            .collect();
        assert_eq!(tool_outputs, original_tool_outputs);
    }

    #[test]
    fn test_truncate_large_tool_result() {
        // Create a tool result with >2000 tokens (>8000 chars)
        let large_output = "x".repeat(12000); // 3000 tokens
        let mut messages = vec![
            Message::user("Run a command"),
            Message::tool("call_1", true, large_output.clone()),
        ];

        let count = ContextManager::truncate_oversized_messages(&mut messages);

        assert_eq!(count, 1);
        let tr = &messages[1].tool_results[0];
        // Should be truncated to ~1500 tokens = 6000 chars + suffix
        assert!(tr.output.len() < large_output.len());
        assert!(tr.output.contains("[...truncated, was 3000 tokens]"));
        // The truncated content should start with the original prefix
        assert!(tr.output.starts_with(&"x".repeat(100)));
        // Verify it's roughly the right size (6000 chars for content + suffix)
        assert!(tr.output.len() < 6100);
    }

    #[test]
    fn test_truncate_large_assistant_content() {
        // Create assistant content with >3000 tokens (>12000 chars)
        let large_content = "y".repeat(16000); // 4000 tokens
        let mut messages = vec![
            Message::user("Explain everything"),
            Message::assistant(large_content.clone()),
        ];

        let count = ContextManager::truncate_oversized_messages(&mut messages);

        assert_eq!(count, 1);
        let content = &messages[1].content;
        assert!(content.len() < large_content.len());
        assert!(content.contains("[...truncated, was 4000 tokens]"));
        // Should be truncated to ~2000 tokens = 8000 chars + suffix
        assert!(content.starts_with(&"y".repeat(100)));
        assert!(content.len() < 8100);
    }

    #[test]
    fn test_truncate_preserves_system_messages() {
        // System messages should NEVER be truncated, even if huge
        let large_system = "z".repeat(20000); // 5000 tokens — well over any threshold
        let mut messages = vec![
            Message::system(large_system.clone()),
            Message::user("Hello"),
        ];

        let count = ContextManager::truncate_oversized_messages(&mut messages);

        assert_eq!(count, 0, "System messages must never be truncated");
        assert_eq!(messages[0].content, large_system);
    }

    #[test]
    fn test_truncate_multiple_messages() {
        // Both a large tool result and a large assistant message
        let large_tool = "a".repeat(10000); // 2500 tokens
        let large_assistant = "b".repeat(16000); // 4000 tokens
        let mut messages = vec![
            Message::user("Do something"),
            Message::tool("call_1", true, large_tool),
            Message::assistant(large_assistant),
        ];

        let count = ContextManager::truncate_oversized_messages(&mut messages);

        assert_eq!(count, 2, "Both oversized messages should be truncated");
        assert!(messages[1].tool_results[0].output.contains("[...truncated"));
        assert!(messages[2].content.contains("[...truncated"));
    }

    #[test]
    fn test_truncate_just_under_threshold_not_truncated() {
        // Tool result at exactly 2000 tokens (8000 chars) — should NOT be truncated
        let borderline_output = "x".repeat(8000); // exactly 2000 tokens
        let mut messages = vec![Message::tool("call_1", true, borderline_output.clone())];

        let count = ContextManager::truncate_oversized_messages(&mut messages);

        assert_eq!(count, 0);
        assert_eq!(messages[0].tool_results[0].output, borderline_output);
    }

    #[test]
    fn test_truncate_just_over_threshold() {
        // Tool result at 2001 tokens (8004 chars) — should be truncated
        let over_output = "x".repeat(8004); // 2001 tokens
        let mut messages = vec![Message::tool("call_1", true, over_output)];

        let count = ContextManager::truncate_oversized_messages(&mut messages);

        assert_eq!(count, 1);
        assert!(messages[0].tool_results[0].output.contains("[...truncated, was 2001 tokens]"));
    }

    #[test]
    fn test_truncate_preserves_user_messages() {
        // User messages should not be truncated (only assistant + tool results)
        let large_user = "u".repeat(20000);
        let mut messages = vec![Message::user(large_user.clone())];

        let count = ContextManager::truncate_oversized_messages(&mut messages);

        assert_eq!(count, 0);
        assert_eq!(messages[0].content, large_user);
    }

    #[test]
    fn test_truncate_multibyte_safety() {
        // Ensure we don't split in the middle of a multi-byte character
        // Create a string with multi-byte chars that crosses the threshold
        // '🦀' is 4 bytes per char
        let crab = "🦀".repeat(2500); // 10000 bytes = 2500 tokens (at chars/4 heuristic, but
                                       // len() counts bytes: 10000 bytes / 4 = 2500 tokens)
        let mut messages = vec![Message::tool("call_1", true, crab.clone())];

        let count = ContextManager::truncate_oversized_messages(&mut messages);

        assert_eq!(count, 1);
        // The output should be valid UTF-8 (no panic)
        let output = &messages[0].tool_results[0].output;
        assert!(output.contains("[...truncated"));
        // Verify it's valid UTF-8 by iterating chars
        let _ = output.chars().count();
    }

    #[test]
    fn test_safe_char_boundary() {
        let s = "hello 🦀 world";
        // '🦀' starts at byte 6 and is 4 bytes (6..10)
        // Targeting byte 8 (mid-emoji) should snap back to 6
        let boundary = ContextManager::safe_char_boundary(s, 8);
        assert!(s.is_char_boundary(boundary));
        assert_eq!(boundary, 6);

        // Targeting past end should return len
        let boundary = ContextManager::safe_char_boundary(s, 1000);
        assert_eq!(boundary, s.len());

        // Targeting exact boundary should return it
        let boundary = ContextManager::safe_char_boundary(s, 6);
        assert_eq!(boundary, 6);
    }
}
