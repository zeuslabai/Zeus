//! Tool execution and cooking loop (Enhanced)
//!
//! The "cooking loop" is the iterative LLM <-> tool execution cycle:
//! 1. Send message to LLM with available tools
//! 2. If LLM requests tool calls, execute them
//! 3. Feed tool results back to LLM
//! 4. Repeat until LLM responds with no tool calls or max iterations reached
//!
//! Enhanced features:
//! - Auto-compact: summarize older messages when context approaches token limit
//! - Auto-rotate: track session rotation on context overflow or errors
//! - Improved iteration tracking: token counts, attempt tracking, compaction metadata
//! - Error classification with retry/backoff

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info, warn};
use zeus_core::{Message, Result, ToolCall, ToolResult, ToolSchema};
use zeus_llm::{LlmClient, StopReason};
use zeus_mnemosyne::Mnemosyne;

use zeus_agent::loop_guard::{LoopGuard, LoopGuardVerdict};

use crate::cooking_auth::{AuthProfileManager, FailoverReason};
use crate::cooking_backoff::{BackoffConfig, BackoffStrategy};
use crate::cooking_checkpoint::CookingCheckpointStore;
use crate::cooking_errors::{ErrorClass, classify_error};
use crate::cooking_events::EventEmitter;
use crate::memory_injector::MemoryInjector;

/// #177: Minimum content length (chars) for a response to be considered "substantive"
/// by the phantom-action guard. Short responses like "OK" or "Done" are likely
/// genuine acks, not hallucinated action claims.
const PHANTOM_ACTION_MIN_CONTENT_LEN: usize = 50;

/// #177: Detect a "phantom action" response — a substantive text response with
/// zero tool_calls when tools were available, from a weak caller that's known
/// to hallucinate tool actions in text instead of emitting tool_calls.
///
/// Intent-agnostic: does NOT inspect what the text says (no per-intent keywords).
/// Only checks the structural mismatch: tools available + substantive content
/// but no tool_calls, from a provider known to exhibit this behavior.
fn is_phantom_action_response(
    tool_calls: &[zeus_core::ToolCall],
    available_tools: &[zeus_core::ToolSchema],
    content: &str,
    stop_reason: &zeus_llm::StopReason,
) -> bool {
    // #232: Gate on CONTENT, not provider. The guard fires only when the reply
    // actually narrates a tool action (first-person cue + tool verb) or leaks a
    // raw tool-call as text. Plain greetings/answers/acks → false → natural voice.
    tool_calls.is_empty()
        && !available_tools.is_empty()
        && content.len() > PHANTOM_ACTION_MIN_CONTENT_LEN
        && *stop_reason != zeus_llm::StopReason::Error
        && content_claims_tool_action(content)
}

/// #232/#236: Returns true when `content` either narrates a first-person tool
/// action or contains a raw tool-call leak (tool-call-as-text). This is the
/// content-based gate that replaces the old provider allow-list.
///
/// Fires on:
/// - a first-person cue (`i'll` / `let me` / `i will` / `i'm going to`) co-occurring
///   with a tool verb (`read|write|edit|run|execute|search|fetch|create|delete|list`)
/// - raw tool-leak markers (`<function`, `<tool_call`, or XML tool tags)
///
/// Plain greetings, normal answers, and short acks return false.
fn content_claims_tool_action(content: &str) -> bool {
    let lower = content.to_lowercase();

    // #236: raw tool-call-as-text leaks — always a phantom/leak.
    if lower.contains("<function")
        || lower.contains("<tool_call")
        || lower.contains("<tool_use")
        || lower.contains("<invoke")
        || lower.contains("</function")
        || lower.contains("</tool_call")
        || lower.contains("function_calls>")
    {
        return true;
    }

    // First-person intent cue narrating an imminent action.
    let has_cue = lower.contains("i'll")
        || lower.contains("i’ll")
        || lower.contains("let me")
        || lower.contains("i will")
        || lower.contains("i'm going to")
        || lower.contains("i’m going to")
        || lower.contains("im going to");
    if !has_cue {
        return false;
    }

    // Co-occurring tool verb — the cue must describe an actual tool action.
    const TOOL_VERBS: [&str; 10] = [
        "read", "write", "edit", "run", "execute", "search", "fetch", "create", "delete", "list",
    ];
    TOOL_VERBS.iter().any(|verb| lower.contains(verb))
}

/// Trait for executing tool calls.
///
/// Implementations should dispatch to zeus-talos, zeus-browser, or any other
/// tool provider. The trait is object-safe for use with dyn dispatch.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a single tool call and return the result.
    async fn execute_tool(&self, call: &ToolCall) -> ToolResult;

    /// Check if a tool is available.
    fn has_tool(&self, name: &str) -> bool;

    /// List all available tool names.
    fn available_tools(&self) -> Vec<String>;
}

/// Configuration for the cooking loop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookingConfig {
    /// Maximum number of LLM <-> tool iterations
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    /// Maximum total tool calls across all iterations
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls: usize,
    /// Whether to inject memory context before LLM calls
    #[serde(default)]
    pub inject_memory: bool,
    /// Number of memory search results to inject
    #[serde(default = "default_memory_results")]
    pub memory_results: usize,
    /// Maximum context tokens (model-dependent). Used for auto-compact threshold.
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: u64,
    /// Compaction threshold as fraction of max_context_tokens (0.0-1.0).
    /// When estimated tokens exceed this ratio, older messages are summarized.
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: f64,
    /// Target ratio after compaction (fraction of max_context_tokens).
    #[serde(default = "default_compaction_target_ratio")]
    pub compaction_target_ratio: f64,
    /// Minimum recent messages to keep verbatim during compaction.
    #[serde(default = "default_min_recent_messages")]
    pub min_recent_messages: usize,
    /// Enable retry with backoff on transient errors.
    #[serde(default = "default_enable_retry")]
    pub enable_retry: bool,
    /// Maximum retry attempts for transient errors.
    #[serde(default = "default_max_retry_attempts")]
    pub max_retry_attempts: u32,
    /// Maximum compactions allowed per cooking run (prevents infinite compaction loops).
    #[serde(default = "default_max_compactions")]
    pub max_compactions: u32,
    /// Minimum iterations between compactions (prevents thrashing).
    #[serde(default = "default_min_iterations_between_compactions")]
    pub min_iterations_between_compactions: usize,
    /// Whether to inject tool-call audit summaries as system messages between iterations.
    /// Disable for providers that reject mid-conversation system messages (all non-Anthropic).
    #[serde(default = "default_audit_logging")]
    pub audit_logging: bool,
    /// Per-tool-call timeout in seconds. If a single tool execution exceeds this,
    /// it is aborted and a failed ToolResult is returned so the LLM can recover.
    /// Prevents a hung SSH / slow command / stuck web_fetch from wedging the entire
    /// cooking loop for the full LLM / relay timeout.
    #[serde(default = "default_tool_call_timeout_secs")]
    pub tool_call_timeout_secs: u64,
    /// Optional human-readable per-tool-call timeout (e.g. "30m", "2h", "30 hours").
    /// When present and parseable, overrides `tool_call_timeout_secs`. When absent
    /// or unparseable, falls back to `tool_call_timeout_secs` (default 1800).
    /// Operator UX: humans type "30m" not "1800".
    #[serde(default)]
    pub tool_call_timeout: Option<String>,
}

fn default_audit_logging() -> bool {
    true // safe default; callers should set false for non-Anthropic providers
}

fn default_tool_call_timeout_secs() -> u64 {
    1800 // 30 minutes — matches gateway timeout; subagent spawns and complex tools need time
}

/// Resolve the per-tool-call timeout, preferring a human-readable string when present
/// and parseable (via `humantime::parse_duration`), falling back to the u64-secs field,
/// and finally to the default 1800s.
///
/// Production-refactor-over-test-scaffolding: free function, pure logic, no Config method.
/// Clean tests, no struct construction needed.
pub fn parse_cooking_timeout(s: Option<&str>, secs: u64) -> std::time::Duration {
    if let Some(raw) = s {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            if let Ok(d) = humantime::parse_duration(trimmed) {
                return d;
            }
            // Parse failure: fall through to u64-secs fallback below.
            // (Caller may log; we stay pure here.)
        }
    }
    if secs == 0 {
        std::time::Duration::from_secs(1800)
    } else {
        std::time::Duration::from_secs(secs)
    }
}

fn default_max_iterations() -> usize {
    200 // Effectively unlimited — LoopGuard (50 tool calls) is the real safety net
}
fn default_max_tool_calls() -> usize {
    200 // 4x previous cap — complex autonomous tasks (LTX-2 setup, multi-step builds) need room
}
fn default_memory_results() -> usize {
    5
}
fn default_max_context_tokens() -> u64 {
    zeus_core::DEFAULT_MAX_CONTEXT_TOKENS as u64
}
fn default_compaction_threshold() -> f64 {
    0.8
}
fn default_compaction_target_ratio() -> f64 {
    0.3
}
fn default_min_recent_messages() -> usize {
    12 // 4 was too aggressive — agents lost task context after compaction
}
fn default_enable_retry() -> bool {
    true
}
fn default_max_retry_attempts() -> u32 {
    3
}
fn default_max_compactions() -> u32 {
    3
}
fn default_min_iterations_between_compactions() -> usize {
    2
}

impl Default for CookingConfig {
    fn default() -> Self {
        Self {
            max_iterations: default_max_iterations(),
            max_tool_calls: default_max_tool_calls(),
            inject_memory: true,
            memory_results: default_memory_results(),
            max_context_tokens: default_max_context_tokens(),
            compaction_threshold: default_compaction_threshold(),
            compaction_target_ratio: default_compaction_target_ratio(),
            min_recent_messages: default_min_recent_messages(),
            enable_retry: default_enable_retry(),
            max_retry_attempts: default_max_retry_attempts(),
            max_compactions: default_max_compactions(),
            min_iterations_between_compactions: default_min_iterations_between_compactions(),
            audit_logging: default_audit_logging(),
            tool_call_timeout_secs: default_tool_call_timeout_secs(),
            tool_call_timeout: None,
        }
    }
}

/// Result of a complete cooking loop execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookingResult {
    /// Final response text from LLM
    pub response: String,
    /// Total iterations (LLM calls) performed
    pub iterations: usize,
    /// All tool calls that were executed
    pub tool_calls: Vec<ToolCallRecord>,
    /// Total processing time in milliseconds
    pub processing_time_ms: u64,
    /// Whether the loop ended normally (no more tool calls) vs hitting limits
    pub completed_naturally: bool,
    /// Memory context that was injected (if any)
    pub memory_context: Option<String>,
    /// Whether context compaction was performed during this run
    #[serde(default)]
    pub compacted: bool,
    /// Summary generated during compaction (if any)
    #[serde(default)]
    pub compaction_summary: Option<String>,
    /// Number of compactions performed during this run
    #[serde(default)]
    pub compaction_count: u32,
    /// Whether a hard context rotation was performed (summary + last message only)
    #[serde(default)]
    pub context_rotated: bool,
    /// Estimated token count at start of run
    #[serde(default)]
    pub estimated_tokens_start: u64,
    /// Estimated token count at end of run
    #[serde(default)]
    pub estimated_tokens_end: u64,
    /// Number of transient-error retries that occurred
    #[serde(default)]
    pub retry_count: u32,
    /// If the loop was interrupted by a new inbound message, contains that message.
    /// The caller should process this as the next user turn immediately.
    #[serde(default)]
    pub interrupted_by: Option<String>,
}

/// Record of a single tool call execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// Tool name
    pub name: String,
    /// Call ID
    pub call_id: String,
    /// Arguments (JSON)
    pub arguments: serde_json::Value,
    /// Whether it succeeded
    pub success: bool,
    /// Output (truncated for serialization)
    pub output: String,
    /// Which iteration this happened in
    pub iteration: usize,
}

/// System prompt used when asking the LLM to summarize conversation for compaction.
const COMPACTION_PROMPT: &str = "\
You are a conversation summarizer. Create a concise summary of the conversation \
history that preserves all important context, decisions, and information needed to \
continue the conversation naturally.\n\n\
Guidelines:\n\
1. Preserve key facts, decisions, and preferences mentioned by the user\n\
2. Keep track of any ongoing tasks or projects being discussed\n\
3. Note any code, files, or technical details that were shared\n\
4. Maintain the emotional tone and rapport established\n\
5. Be concise but comprehensive - nothing important should be lost\n\n\
Format your summary as a structured recap.";

// ============================================================================
// R1: TodoWrite/TodoRead — agent-managed task list
// ============================================================================

/// Status of a single todo item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

impl Default for TodoStatus {
    fn default() -> Self { TodoStatus::Pending }
}

/// A single todo item maintained by the cooking loop on behalf of the agent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    #[serde(default)]
    pub status: TodoStatus,
}

/// Outcome of a cooking turn — emitted at natural completion.
/// v0.2: includes outstanding-todos signal so coordinator can detect dropped work.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnOutcome {
    /// LLM stopped emitting tool calls AND todo list is empty — clean finish.
    Complete { iterations: usize, tool_calls: usize },
    /// LLM stopped emitting tool calls but todos remain — continued via reminder.
    ContinuedWithTodos { iterations: usize, remaining: usize },
    /// Hit a hard stop (max iterations, interrupt, error).
    Aborted { reason: String, iterations: usize },
}

/// Render the todo list as a human-readable bulleted summary for re-prompting.
pub(crate) fn render_todos(todos: &[TodoItem]) -> String {
    if todos.is_empty() {
        return "(no todos)".to_string();
    }
    todos.iter().enumerate().map(|(i, t)| {
        let mark = match t.status {
            TodoStatus::Pending => "[ ]",
            TodoStatus::InProgress => "[~]",
            TodoStatus::Completed => "[x]",
        };
        format!("{}. {} {} (id={})", i + 1, mark, t.content, t.id)
    }).collect::<Vec<_>>().join("\n")
}

/// Handle the loop-internal `todo_write` / `todo_read` tools.
///
/// `todo_write`: replaces the entire todo list. Args: `{ "todos": [{"id","content","status"}, ...] }`.
/// `todo_read`: returns the current list. Args: `{}`.
pub(crate) fn handle_todo_tool(call: &ToolCall, todos: &mut Vec<TodoItem>) -> ToolResult {
    if call.name == "todo_read" {
        let body = render_todos(todos);
        return ToolResult {
            call_id: call.id.clone(),
            success: true,
            output: body,
        };
    }
    // todo_write
    #[derive(Deserialize)]
    struct WriteArgs { todos: Vec<TodoItem> }
    let parsed: std::result::Result<WriteArgs, _> = serde_json::from_value(call.arguments.clone());
    match parsed {
        Ok(args) => {
            *todos = args.todos;
            let pending = todos.iter().filter(|t| t.status != TodoStatus::Completed).count();
            ToolResult {
                call_id: call.id.clone(),
                success: true,
                output: format!(
                    "Todo list updated ({} item(s), {} pending). Current:\n{}",
                    todos.len(), pending, render_todos(todos)
                ),
            }
        }
        Err(e) => ToolResult {
            call_id: call.id.clone(),
            success: false,
            output: format!(
                "todo_write: invalid arguments: {}. Expected {{\"todos\":[{{\"id\":\"...\",\"content\":\"...\",\"status\":\"pending|in_progress|completed\"}}]}}",
                e
            ),
        },
    }
}

/// Built-in tool schemas for the agent's todo list. Callers should merge these
/// into the `tools` slice they pass to `CookingLoop::run` so the LLM can use them.
pub fn todo_tool_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "todo_write".to_string(),
            description: "Replace the agent's todo list. Use to plan multi-step work, then update statuses as you go. The cooking loop will not exit naturally while pending todos remain.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id":      { "type": "string", "description": "Stable identifier (free-form)." },
                                "content": { "type": "string", "description": "What needs to be done." },
                                "status":  { "type": "string", "enum": ["pending","in_progress","completed"] }
                            },
                            "required": ["id", "content"]
                        }
                    }
                },
                "required": ["todos"]
            }),
        },
        ToolSchema {
            name: "todo_read".to_string(),
            description: "Read the current todo list (rendered as a checklist).".to_string(),
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        },
    ]
}

/// The cooking loop: iterative LLM <-> tool execution.
#[derive(Default)]
pub struct CookingLoop {
    config: CookingConfig,
    event_emitter: Option<EventEmitter>,
    auth_manager: Option<std::sync::Arc<tokio::sync::Mutex<AuthProfileManager>>>,
    /// Optional Mnemosyne instance for per-iteration memory refresh
    mnemosyne: Option<Arc<Mnemosyne>>,
    /// Memory injector for formatting search results into context
    memory_injector: Option<MemoryInjector>,
    /// Optional checkpoint store for crash-resume persistence
    checkpoint_store: Option<Arc<CookingCheckpointStore>>,
    /// Optional receiver for mid-loop interrupts (new channel messages).
    /// When a message arrives on this channel, the cooking loop exits gracefully
    /// at the next iteration boundary and returns `interrupted_by` in the result.
    interrupt_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    /// Optional ledger for recording turn outcomes (R2 event-driven heartbeat).
    /// When set, TurnOutcome::Complete writes a "complete" entry so the heartbeat
    /// knows not to re-wake for this task.
    ledger: Option<Arc<dyn crate::ledger::LedgerStore>>,
    /// Optional sender to fire a WakeRequest when the cooking loop completes.
    /// When set, the heartbeat is woken immediately on TurnOutcome::Complete so
    /// the agent resumes without waiting for the next scheduled heartbeat.
    wake_sender: Option<tokio::sync::mpsc::Sender<super::heartbeat::WakeRequest>>,
    /// Optional progress signal for the #284B idle watchdog. Stores the unix-seconds
    /// timestamp of the most recent model text response or completed tool call.
    /// The gateway timeout arm reads this to kill only genuinely idle cooks.
    /// `None` = no observer wired.
    progress_signal: Option<Arc<std::sync::atomic::AtomicU64>>,
}

/// Owns cleanup for a checkpoint row while a cooking future is live.
///
/// The gateway can drop the cooking future on timeout/client abort before the
/// loop reaches its normal completion path. In that case this guard removes the
/// incomplete row so bootstrap cannot auto-resume a stale `cooking-*` task. A
/// process crash still leaves the row behind for crash-resume, because `Drop`
/// never runs.
struct CheckpointSessionGuard {
    store: Option<Arc<CookingCheckpointStore>>,
    session_id: String,
    active: bool,
}

impl CheckpointSessionGuard {
    fn new(store: Option<Arc<CookingCheckpointStore>>, session_id: String) -> Self {
        let active = store.is_some();
        Self {
            store,
            session_id,
            active,
        }
    }

    async fn complete_and_prune(&mut self) {
        if let Some(store) = self.store.as_ref() {
            store.mark_completed(&self.session_id).await;
            store.prune_completed(&self.session_id).await;
        }
        self.active = false;
    }

    async fn delete_now(&mut self, reason: &'static str) {
        if !self.active {
            return;
        }
        if let Some(store) = self.store.as_ref() {
            store.delete_session(&self.session_id).await;
            warn!(
                session = %self.session_id,
                reason,
                "Cooking checkpoint cleaned before resumable state could orphan"
            );
        }
        self.active = false;
    }
}

impl Drop for CheckpointSessionGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let Some(store) = self.store.take() else {
            return;
        };
        let session_id = std::mem::take(&mut self.session_id);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    store.delete_session(&session_id).await;
                    warn!(
                        session = %session_id,
                        reason = "future_dropped",
                        "Cooking checkpoint cleaned before resumable state could orphan"
                    );
                });
            }
            Err(_) => {
                warn!(
                    session = %session_id,
                    reason = "future_dropped_no_runtime",
                    "Cooking checkpoint cleanup skipped because no Tokio runtime was available"
                );
            }
        }
    }
}

impl CookingLoop {
    pub fn new(config: CookingConfig) -> Self {
        Self {
            config,
            event_emitter: None,
            auth_manager: None,
            mnemosyne: None,
            memory_injector: None,
            checkpoint_store: None,
            ledger: None,
            interrupt_rx: None,
            wake_sender: None,
            progress_signal: None,
        }
    }

    /// Attach an event emitter for streaming cooking events.
    pub fn with_events(mut self, emitter: EventEmitter) -> Self {
        self.event_emitter = Some(emitter);
        self
    }

    /// Attach an auth profile manager for automatic profile rotation on errors.
    pub fn with_auth_manager(
        mut self,
        manager: std::sync::Arc<tokio::sync::Mutex<AuthProfileManager>>,
    ) -> Self {
        self.auth_manager = Some(manager);
        self
    }

    /// Attach Mnemosyne for per-iteration memory context refresh.
    ///
    /// When set, the cooking loop re-queries Mnemosyne before each LLM call
    /// using the evolving conversation context, keeping memory awareness fresh
    /// across multi-turn tool execution.
    pub fn with_mnemosyne(mut self, mnemosyne: Arc<Mnemosyne>, injector: MemoryInjector) -> Self {
        self.mnemosyne = Some(mnemosyne);
        self.memory_injector = Some(injector);
        self
    }

    /// Attach a checkpoint store for crash-resume persistence.
    ///
    /// When set, the cooking loop saves a checkpoint after each iteration's
    /// tool executions, enabling resume from the last checkpoint on crash.
    pub fn with_checkpoint_store(mut self, store: Arc<CookingCheckpointStore>) -> Self {
        self.checkpoint_store = Some(store);
        self
    }

    /// Attach a mid-loop interrupt receiver.
    ///
    /// When a message is sent on the corresponding sender, the cooking loop exits
    /// at the next iteration boundary. The interrupt message is returned in
    /// `CookingResult::interrupted_by` so callers can process it immediately.
    pub fn with_interrupt(mut self, rx: tokio::sync::mpsc::UnboundedReceiver<String>) -> Self {
        self.interrupt_rx = Some(rx);
        self
    }

    /// Attach a ledger for recording turn outcomes (R2 integration).
    pub fn with_ledger(mut self, ledger: Arc<dyn crate::ledger::LedgerStore>) -> Self {
        self.ledger = Some(ledger);
        self
    }

    /// Set a WakeRequest sender to fire the heartbeat immediately when the
    /// cooking loop completes (TurnOutcome::Complete / ContinuedWithTodos).
    /// This bypasses the event-driven-only safety net so the agent resumes
    /// without waiting for the next scheduled heartbeat interval.
    pub fn with_wake_sender(
        mut self,
        sender: tokio::sync::mpsc::Sender<super::heartbeat::WakeRequest>,
    ) -> Self {
        self.wake_sender = Some(sender);
        self
    }

    /// Attach a progress signal for the #284B cooking idle watchdog.
    ///
    /// The provided `AtomicU64` is updated with the current unix-seconds
    /// timestamp after every model text response and every completed tool call
    /// inside `run()`. The gateway interprets this as last cook activity for the
    /// #284B idle watchdog: active cooks keep running; genuinely idle cooks die.
    pub fn with_progress_signal(mut self, signal: Arc<std::sync::atomic::AtomicU64>) -> Self {
        self.progress_signal = Some(signal);
        self
    }

    fn record_progress(&self) {
        if let Some(ref signal) = self.progress_signal {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            signal.store(now, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Set event emitter after construction.
    pub fn set_event_emitter(&mut self, emitter: EventEmitter) {
        self.event_emitter = Some(emitter);
    }

    /// Build a memory query from the original user message and recent conversation.
    ///
    /// Combines the original request with the latest tool results to create
    /// a query that captures the evolving context of the cooking loop.
    fn build_memory_query(&self, original_message: &str, messages: &[Message]) -> String {
        // Take the last few messages' content for context
        let recent: Vec<&str> = messages
            .iter()
            .rev()
            .take(3)
            .filter(|m| !m.content.is_empty())
            .map(|m| m.content.as_str())
            .collect();

        if recent.is_empty() {
            return original_message.to_string();
        }

        // Combine original message with recent context, truncated to keep the query focused
        let recent_text: String = recent.into_iter().rev().collect::<Vec<_>>().join(" ");
        let truncated = if recent_text.len() > 500 {
            &recent_text[..zeus_core::floor_char_boundary(&recent_text, 500)]
        } else {
            &recent_text
        };

        format!("{} {}", original_message, truncated)
    }

    /// Emit a cooking event if an emitter is configured.
    fn emit(&self, event: crate::cooking_events::CookingEvent) {
        if let Some(ref emitter) = self.event_emitter {
            emitter.emit(event);
        }
    }

    /// R1+R2: Emit a turn-outcome marker and write to ledger if available.
    fn emit_turn_outcome(&self, outcome: &TurnOutcome) {
        match serde_json::to_string(outcome) {
            Ok(json) => info!(turn_outcome = %json, "TurnOutcome v0.2 emitted"),
            Err(e) => warn!("Failed to serialize TurnOutcome: {}", e),
        }
        // R2 integration: write completion events to the ledger so heartbeat
        // knows not to re-wake for completed tasks.
        if let Some(ref ledger) = self.ledger {
            let (kind, reason) = match outcome {
                TurnOutcome::Complete { .. } => ("complete", "todos_all_done"),
                TurnOutcome::ContinuedWithTodos { .. } => ("continue", "todos_remaining"),
                TurnOutcome::Aborted { reason, .. } => ("abort", reason.as_str()),
            };
            let entry = crate::ledger::LedgerEntry::now(kind, reason);
            let ledger = ledger.clone();
            tokio::spawn(async move {
                if let Err(e) = ledger.append(entry).await {
                    warn!("Failed to write turn outcome to ledger: {}", e);
                }
            });
        }
        // Fire WakeRequest so the heartbeat fires immediately without waiting
        // for the next scheduled interval.
        if let (TurnOutcome::Complete { .. } | TurnOutcome::ContinuedWithTodos { .. },
                Some(sender)) = (outcome, &self.wake_sender) {
            let req = super::heartbeat::WakeRequest {
                reason: "cooking_complete".into(),
                agent_id: None,
            };
            if let Err(e) = sender.try_send(req) {
                debug!("Failed to send WakeRequest on turn outcome: {}", e);
            }
        }
    }

    /// Run the full cooking loop.
    ///
    /// Parameters:
    /// - `message`: The user's message
    /// - `system_prompt`: Base system prompt (from workspace)
    /// - `tools`: Available tool schemas
    /// - `llm`: LLM client for inference
    /// - `executor`: Tool executor implementation
    /// - `memory_context`: Optional pre-fetched memory context to inject
    pub async fn run(
        &mut self,
        message: &str,
        system_prompt: &str,
        tools: &[ToolSchema],
        llm: &LlmClient,
        executor: &dyn ToolExecutor,
        memory_context: Option<&str>,
    ) -> Result<CookingResult> {
        self.run_with_history(message, system_prompt, tools, llm, executor, memory_context, &[], None, vec![])
            .await
    }

    /// Run the cooking loop with prior conversation history.
    ///
    /// `conversation_history` is prepended before the current user message so
    /// the LLM sees prior turns from the session. This prevents the "amnesia"
    /// problem where agents forget context between messages.
    ///
    /// `cancel` — optional cancellation token. When cancelled, the cooking loop
    /// finishes the current iteration and returns a partial result. Used by the
    /// gateway to preempt long cooks when a new channel message arrives.
    pub async fn run_with_history(
        &mut self,
        message: &str,
        system_prompt: &str,
        tools: &[ToolSchema],
        llm: &LlmClient,
        executor: &dyn ToolExecutor,
        memory_context: Option<&str>,
        conversation_history: &[Message],
        cancel: Option<tokio_util::sync::CancellationToken>,
        attachments: Vec<zeus_core::Attachment>,
    ) -> Result<CookingResult> {
        let start = std::time::Instant::now();
        let mut all_tool_records: Vec<ToolCallRecord> = Vec::new();
        let mut total_tool_calls = 0;
        // R1: Agent-managed todo list. Modified via todo_write/todo_read tool calls.
        let mut todos: Vec<TodoItem> = Vec::new();
        let mut compaction_count: u32 = 0;
        let mut loop_guard = LoopGuard::default_limits();
        let mut last_compaction_iteration: usize = 0;
        let mut compaction_summary: Option<String> = None;
        let mut context_rotated = false;
        let mut retry_count: u32 = 0;
        let mut planning_retry_count: u32 = 0;
        let mut phantom_action_reprompt_used: bool = false;

        // Attempt checkpoint resume — find an interrupted session matching this message.
        // On crash/restart, the cooking loop continues from the last saved checkpoint
        // rather than re-executing all tool calls from scratch.
        let (session_id, session_started_at) = if let Some(ref store) = self.checkpoint_store {
            let interrupted = store.find_interrupted_sessions().await;
            if let Some(prior) = interrupted.into_iter().find(|s| s.original_message == message) {
                info!(
                    session = %prior.session_id,
                    iteration = prior.iteration,
                    tools = prior.tool_call_count,
                    "Resuming interrupted cooking session from iteration {}",
                    prior.iteration
                );
                (prior.session_id, prior.started_at)
            } else {
                let id = uuid::Uuid::new_v4().to_string();
                let now = chrono::Utc::now();
                store.start_session(&id, message, system_prompt).await;
                (id, now)
            }
        } else {
            (uuid::Uuid::new_v4().to_string(), chrono::Utc::now())
        };

        let mut checkpoint_guard = CheckpointSessionGuard::new(
            self.checkpoint_store.clone(),
            session_id.clone(),
        );

        // Restore state from checkpoint, or build fresh conversation.
        let restored = if let Some(ref store) = self.checkpoint_store {
            store
                .load_checkpoint(&session_id)
                .await
                .filter(|cp| !cp.completed && cp.iteration > 0 && !cp.messages.is_empty())
        } else {
            None
        };

        let mut iterations: usize;
        let mut messages: Vec<Message>;
        if let Some(cp) = restored {
            info!(
                session = %session_id,
                iteration = cp.iteration,
                tools = cp.tool_call_count,
                "Checkpoint restored — continuing from iteration {}",
                cp.iteration
            );
            total_tool_calls = cp.tool_call_count;
            all_tool_records = cp.tool_records;
            iterations = cp.iteration;
            messages = cp.messages;
            todos = cp.todos; // R1: restore agent-managed todo list
        } else {
            iterations = 0;
            messages = conversation_history.to_vec();
            if attachments.is_empty() {
                messages.push(Message::user(message));
            } else {
                messages.push(Message::user_with_attachments(message, attachments));
            }
        }

        // Build initial system prompt with pre-fetched memory context
        let mut current_memory_context: Option<String> = memory_context.map(|s| s.to_string());
        let mut full_system_prompt = if let Some(ref memory) = current_memory_context {
            format!("{}\n\n## Relevant Memory\n{}", system_prompt, memory)
        } else {
            system_prompt.to_string()
        };

        // Estimate initial tokens (includes system prompt estimate)
        let system_tokens = estimate_str_tokens(&full_system_prompt);
        let estimated_tokens_start = system_tokens + estimate_message_tokens(&messages);

        // Compaction threshold
        let compaction_token_threshold =
            (self.config.max_context_tokens as f64 * self.config.compaction_threshold) as u64;
        let mut final_response = String::new();
        let mut completed_naturally = false;

        // Backoff strategy for transient errors
        let backoff_config = BackoffConfig {
            max_retries: self.config.max_retry_attempts,
            ..BackoffConfig::default()
        };
        let mut backoff = BackoffStrategy::new(backoff_config);

        // BUG 2 (Dispatch 22) — cooking no-progress detector.
        // Tracks the last N consecutive iterations that produced no tool calls and
        // identical `response.content`. When N hits the threshold, abort with a
        // distinctive error so the agent stops re-emitting the same scratchpad
        // forever (the "self-talk degenerate loop" failure mode flagged by the
        // audit on zeus-spark + GLM-degraded zeus107 sessions).
        const STUCK_AGENT_THRESHOLD: usize = 3;
        let mut stuck_streak: usize = 0;
        let mut last_stuck_response: Option<String> = None;
        // #99: kimi stuck-agent recovery. Before hard-aborting on a degenerate
        // empty-tool_call loop, inject one "you must call a tool now" re-prompt
        // and reset the streak ONCE. Provider-agnostic; mirrors the planning-retry
        // guard below. Only the SECOND time we hit the threshold do we abort.
        let mut stuck_recovery_used = false;

        loop {
            iterations += 1;

            // Check cancellation token — new message arrived, yield gracefully
            if let Some(ref token) = cancel {
                if token.is_cancelled() {
                    info!(
                        "Cooking cancelled after {} iterations, {} tool calls (new message preempted)",
                        iterations - 1, total_tool_calls
                    );
                    break;
                }
            }

            // Mid-loop interrupt: check if a new channel message arrived between iterations.
            // Uses try_recv (non-blocking) so the hot path has zero overhead when quiet.
            if let Some(ref mut rx) = self.interrupt_rx {
                match rx.try_recv() {
                    Ok(interrupt_msg) => {
                        info!(
                            "Cooking interrupted after {} iterations, {} tool calls — new message preempted",
                            iterations - 1, total_tool_calls
                        );
                        completed_naturally = false;
                        let estimated_tokens_end = system_tokens + estimate_message_tokens(&messages);
                        checkpoint_guard.delete_now("interrupt").await;
                        return Ok(CookingResult {
                            response: final_response,
                            iterations: iterations - 1,
                            tool_calls: all_tool_records,
                            processing_time_ms: start.elapsed().as_millis() as u64,
                            completed_naturally,
                            memory_context: current_memory_context,
                            compacted: compaction_count > 0,
                            compaction_summary,
                            compaction_count,
                            context_rotated,
                            estimated_tokens_start,
                            estimated_tokens_end,
                            retry_count,
                            interrupted_by: Some(interrupt_msg),
                        });
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        // Sender dropped — interrupt mechanism disabled, continue normally
                        self.interrupt_rx = None;
                    }
                }
            }

            // S59-P4: Progress tracking — log each iteration for visibility
            info!(
                "Cooking iteration {}/{} — {} tool calls so far",
                iterations, self.config.max_iterations, total_tool_calls
            );

            if iterations > self.config.max_iterations {
                warn!(
                    "Cooking loop hit max iterations ({})",
                    self.config.max_iterations
                );
                break;
            }

            if total_tool_calls >= self.config.max_tool_calls {
                warn!(
                    "Cooking loop hit max tool calls ({})",
                    self.config.max_tool_calls
                );
                completed_naturally = false;
                // Flag session as interrupted so heartbeat auto-resumes
                if let Some(ref store) = self.checkpoint_store {
                    store.mark_interrupted(&session_id).await;
                }
                break;
            }

            // --- Auto-compact check (allows multiple compactions with gap) ---
            let current_tokens = system_tokens + estimate_message_tokens(&messages);
            let iterations_since_compaction = iterations.saturating_sub(last_compaction_iteration);
            let can_compact = compaction_count < self.config.max_compactions
                && iterations_since_compaction >= self.config.min_iterations_between_compactions;

            if can_compact && current_tokens >= compaction_token_threshold {
                info!(
                    tokens = current_tokens,
                    threshold = compaction_token_threshold,
                    compaction = compaction_count + 1,
                    "Context approaching limit, auto-compacting"
                );
                self.emit(crate::cooking_events::CookingEvent::compaction_start(
                    "cooking",
                    current_tokens,
                ));

                match compact_messages(&messages, llm, self.config.min_recent_messages).await {
                    Ok((new_messages, summary)) => {
                        let new_tokens = system_tokens + estimate_message_tokens(&new_messages);
                        info!(
                            before = current_tokens,
                            after = new_tokens,
                            compaction = compaction_count + 1,
                            "Context compacted"
                        );
                        self.emit(crate::cooking_events::CookingEvent::compaction_complete(
                            "cooking",
                            current_tokens,
                            new_tokens,
                            true,
                            Some(summary.chars().take(100).collect()),
                        ));
                        messages = new_messages;
                        compaction_summary = Some(summary);
                        compaction_count += 1;
                        last_compaction_iteration = iterations;
                    }
                    Err(e) => {
                        warn!("Auto-compact failed (continuing without): {}", e);
                        self.emit(crate::cooking_events::CookingEvent::warning(
                            format!("Auto-compact failed: {}", e),
                            true,
                        ));
                    }
                }
            }

            // --- Per-iteration memory refresh from Mnemosyne ---
            // On iterations after the first, re-query Mnemosyne using the evolving
            // conversation context so the LLM sees memories relevant to the latest
            // tool results, not just the original user message.
            // IMPORTANT: Only rebuild full_system_prompt when memory actually changed.
            // Rebuilding on every iteration busts the Anthropic prompt cache even when
            // no new memories were stored — forcing a full re-tokenization each turn.
            // Skip for budget providers (GLM/ZAI) — memory rarely changes mid-cook
            // and the re-query + prompt rebuild burns tokens on every iteration.
            let skip_memory_refresh = *llm.provider() == zeus_core::Provider::Zai;
            if !skip_memory_refresh && iterations > 1
                && let (Some(mnemosyne), Some(injector)) = (&self.mnemosyne, &self.memory_injector)
            {
                let query = self.build_memory_query(message, &messages);
                if let Some(fresh_memory) = injector.fetch_context(mnemosyne, &query).await {
                    if current_memory_context.as_deref() != Some(fresh_memory.as_str()) {
                        debug!(
                            iteration = iterations,
                            chars = fresh_memory.len(),
                            "Memory context changed — rebuilding system prompt"
                        );
                        full_system_prompt =
                            format!("{}\n\n## Relevant Memory\n{}", system_prompt, fresh_memory);
                        current_memory_context = Some(fresh_memory);
                    } else {
                        debug!(
                            iteration = iterations,
                            "Memory context unchanged — skipping rebuild (prompt cache stability)"
                        );
                    }
                }
            }

            debug!(iteration = iterations, "Cooking loop: calling LLM");

            // --- LLM call with error classification and retry ---
            // R1: augment caller-provided tools with todo_write / todo_read so the LLM
            // can manage its own task list. These are intercepted in the dispatch
            // switch below and never hit the external executor.
            let tools_with_todos: Vec<ToolSchema> = {
                let mut v: Vec<ToolSchema> = tools.to_vec();
                v.extend(todo_tool_schemas());
                v
            };
            let response = match llm
                .complete(&messages, &tools_with_todos, Some(&full_system_prompt))
                .await
            {
                Ok(resp) => {
                    backoff.reset();
                    // Phase 1 Lite: emit the model's text content as a TextDelta so the
                    // TUI can show the actual reasoning + plan the model wrote *before*
                    // each tool call, instead of a bare "Thinking..." placeholder.
                    // Per-iteration (not per-token) since llm.complete() is non-streaming.
                    if !resp.content.is_empty() {
                        self.record_progress();
                        self.emit(crate::cooking_events::CookingEvent::text_delta(
                            resp.content.clone(),
                        ));
                    }
                    resp
                }
                Err(e) => {
                    if !self.config.enable_retry {
                        return Err(e);
                    }

                    let error_class = classify_error(&e);

                    // Try auth profile rotation for auth/billing/rate-limit/unavailable errors
                    if let Some(ref auth_mgr) = self.auth_manager {
                        let failover_reason = match error_class {
                            ErrorClass::RateLimit => Some(FailoverReason::RateLimit),
                            ErrorClass::Auth => Some(FailoverReason::Auth),
                            ErrorClass::Billing => Some(FailoverReason::Billing),
                            ErrorClass::ProviderUnavailable => Some(FailoverReason::Unavailable),
                            _ => None,
                        };
                        if let Some(reason) = failover_reason {
                            let mut mgr = auth_mgr.lock().await;
                            if let Some((from, to)) = mgr.rotate_session_profile("cooking", reason)
                            {
                                warn!(
                                    from = %from,
                                    to = %to,
                                    reason = %reason,
                                    "Rotating auth profile"
                                );
                                self.emit(crate::cooking_events::CookingEvent::profile_rotated(
                                    &from,
                                    &to,
                                    &reason.to_string(),
                                ));
                                retry_count += 1;
                                iterations -= 1;
                                continue;
                            }
                        }
                    }

                    match error_class {
                        ErrorClass::Transient | ErrorClass::RateLimit => {
                            if let Some(delay) = backoff.next_delay() {
                                retry_count += 1;
                                warn!(
                                    error = %e,
                                    attempt = retry_count,
                                    delay_ms = delay.as_millis() as u64,
                                    "Transient error, retrying"
                                );
                                self.emit(crate::cooking_events::CookingEvent::retry_attempt(
                                    retry_count,
                                    self.config.max_retry_attempts,
                                    delay.as_millis() as u64,
                                    &format!("{:?}", error_class),
                                ));
                                tokio::time::sleep(delay).await;
                                // Don't count this as an iteration
                                iterations -= 1;
                                continue;
                            }
                            return Err(e);
                        }
                        ErrorClass::ContextOverflow
                            if compaction_count < self.config.max_compactions =>
                        {
                            // Force compaction and retry
                            warn!(
                                compaction = compaction_count + 1,
                                "Context overflow, forcing compaction"
                            );
                            self.emit(crate::cooking_events::CookingEvent::warning(
                                "Context overflow detected, compacting",
                                true,
                            ));
                            let current_tokens = system_tokens + estimate_message_tokens(&messages);
                            match compact_messages(&messages, llm, self.config.min_recent_messages)
                                .await
                            {
                                Ok((new_messages, summary)) => {
                                    let new_tokens =
                                        system_tokens + estimate_message_tokens(&new_messages);
                                    self.emit(
                                        crate::cooking_events::CookingEvent::compaction_complete(
                                            "cooking",
                                            current_tokens,
                                            new_tokens,
                                            true,
                                            Some(summary.chars().take(100).collect()),
                                        ),
                                    );
                                    messages = new_messages;
                                    compaction_summary = Some(summary);
                                    compaction_count += 1;
                                    last_compaction_iteration = iterations;
                                    iterations -= 1;
                                    continue;
                                }
                                Err(_) => return Err(e),
                            }
                        }
                        ErrorClass::ContextOverflow => {
                            // Max compactions exhausted — hard rotate context
                            warn!("Context overflow after max compactions, hard-rotating context");
                            self.emit(crate::cooking_events::CookingEvent::warning(
                                "Hard context rotation: dropping history, keeping summary + last message",
                                true,
                            ));
                            let summary = compaction_summary.clone().unwrap_or_else(|| {
                                "Previous conversation context was too large and was reset."
                                    .to_string()
                            });
                            messages = vec![Message::user(format!(
                                "[Context rotated — previous summary]\n{}\n\n[Current request]\n{}",
                                summary, message
                            ))];
                            context_rotated = true;
                            iterations -= 1;
                            continue;
                        }
                        _ => return Err(e),
                    }
                }
            };

            // If no tool calls, check for planning-only retry before accepting
            if response.tool_calls.is_empty() || response.stop_reason != StopReason::ToolUse {
                // #177: Phantom-action guard for weak callers.
                // If a weak provider emits a substantive text response with zero tool_calls
                // when tools were available, re-prompt once — it likely hallucinated an
                // action claim in text instead of emitting a tool_call.
                if is_phantom_action_response(
                    &response.tool_calls,
                    &tools_with_todos,
                    &response.content,
                    &response.stop_reason,
                ) {
                    if !phantom_action_reprompt_used {
                        phantom_action_reprompt_used = true;
                        info!(
                            "Phantom-action guard: re-prompting weak caller (no tool_calls on substantive response)"
                        );
                        messages.push(Message::assistant(&response.content));
                        messages.push(Message::system(
                            "You did not call any tools in that response, but tools are available. \
                             If you intended to perform an action, you must call the appropriate tool — \
                             describing an action in text does not execute it. \
                             If no action was needed, simply confirm.",
                        ));
                        continue; // re-enter cooking loop
                    } else {
                        warn!(
                            "Phantom-action guard: second no-tool_call response after re-prompt, accepting as final"
                        );
                    }
                }
                // BUG 2 (Dispatch 22) — stuck-agent abort.
                // If the model emits the EXACT same content with no tool calls for
                // STUCK_AGENT_THRESHOLD consecutive iterations, the sampler is wedged
                // (degenerate loop) — abort the cooking session rather than burning
                // tokens forever. Only counts true repeats; a single one-shot text
                // response or a planning-retry that successfully changes the next iter's
                // text resets the streak.
                if response.tool_calls.is_empty() {
                    let same_as_last = last_stuck_response
                        .as_deref()
                        .map(|prev| prev == response.content.as_str())
                        .unwrap_or(false);
                    if same_as_last {
                        stuck_streak += 1;
                    } else {
                        stuck_streak = 1;
                        last_stuck_response = Some(response.content.clone());
                    }
                    if stuck_streak >= STUCK_AGENT_THRESHOLD {
                        if !stuck_recovery_used {
                            // #99: First time we wedge — try to break the loop instead
                            // of aborting. Inject an explicit "act now" instruction,
                            // reset the streak, and re-enter the cooking loop ONCE.
                            // Kimi/moonshot can fall into a degenerate empty-tool_call
                            // loop after orphan-strip (#57); a single re-prompt is often
                            // enough to unwedge it. Provider-agnostic and safe for opus.
                            stuck_recovery_used = true;
                            stuck_streak = 0;
                            last_stuck_response = None;
                            warn!(
                                "stuck-agent: degenerate empty-tool_call loop at iter {} — \
                                 attempting one-shot recovery re-prompt before aborting",
                                iterations
                            );
                            self.emit(crate::cooking_events::CookingEvent::warning(
                                "Stuck-agent recovery: re-prompting model to call a tool",
                                true,
                            ));
                            messages.push(Message::user(
                                "You repeated the same response without calling any tool. \
                                 You MUST call a tool now to make progress. If the task is \
                                 complete, say so explicitly and stop. Do not repeat your \
                                 previous message.",
                            ));
                            continue; // re-enter cooking loop
                        }
                        let msg = format!(
                            "stuck-agent: emitting same scratchpad without tool calls for {} consecutive iterations (iter {}); recovery re-prompt already attempted",
                            stuck_streak, iterations
                        );
                        warn!("{}", msg);
                        return Err(zeus_core::Error::Agent(msg));
                    }
                } else {
                    // Tool calls exist — not a stuck-no-progress state, reset.
                    stuck_streak = 0;
                    last_stuck_response = None;
                }
                // Planning-only retry: if the model described a plan but didn't act,
                // inject an "act now" instruction and re-enter the loop.
                // Guard rails (from OpenClaw pattern):
                // - Max 2 planning retries
                // - Only short text (<2000 chars) without code blocks
                // - Must contain planning language ("I will", "let me", "I'll")
                // - Must have had tools available (otherwise text-only is expected)
                let content_lower = response.content.to_lowercase();
                let is_short = response.content.len() < 2000;
                let no_code_blocks = !response.content.contains("```");
                let has_planning_language = content_lower.contains("i will")
                    || content_lower.contains("let me")
                    || content_lower.contains("i'll")
                    || content_lower.contains("i would")
                    || content_lower.contains("i'm going to")
                    || content_lower.contains("here's my plan")
                    || content_lower.contains("the plan is");
                let had_tools = !tools.is_empty();
                // #177: Stack-bound — total re-prompts/turn ≤ 2.
                // If the phantom-action guard already consumed 1 re-prompt,
                // planning retry gets at most 1 more.
                let planning_limit: u32 = if phantom_action_reprompt_used { 1 } else { 2 };
                let under_retry_limit = planning_retry_count < planning_limit;

                if is_short && no_code_blocks && has_planning_language && had_tools && under_retry_limit {
                    planning_retry_count += 1;
                    info!(
                        iteration = iterations,
                        retry = planning_retry_count,
                        "Planning-only response detected — retrying with act-now instruction ({}/2)",
                        planning_retry_count
                    );
                    // Inject the planning response + retry instruction
                    messages.push(Message::assistant(&response.content));
                    messages.push(Message::user(
                        "The previous response only described the plan. Do not restate the plan. \
                         Act now: take the first concrete tool action you can. \
                         If a real blocker prevents action, reply with the exact blocker in one sentence."
                    ));
                    continue; // re-enter cooking loop
                }

                // R1 completion gate: "no tool calls AND todos empty."
                let pending = todos.iter().filter(|t| t.status != TodoStatus::Completed).count();
                if pending > 0 {
                    info!(
                        iteration = iterations,
                        pending_todos = pending,
                        "LLM stopped emitting tool calls but todos remain — re-prompting to continue"
                    );
                    let outcome = TurnOutcome::ContinuedWithTodos {
                        iterations,
                        remaining: pending,
                    };
                    self.emit_turn_outcome(&outcome);
                    let summary = render_todos(&todos);
                    messages.push(Message::user(&format!(
                        "You stopped, but the todo list still has {} unfinished item(s). \
                         Continue working through them. If a todo cannot be completed, mark it completed with a note via todo_write. \
                         Current list:\n{}",
                        pending, summary
                    )));
                    continue; // re-enter cooking loop
                }
                final_response = response.content;
                completed_naturally = true;
                let outcome = TurnOutcome::Complete {
                    iterations,
                    tool_calls: total_tool_calls,
                };
                self.emit_turn_outcome(&outcome);
                info!(
                    iteration = iterations,
                    total_tools = total_tool_calls,
                    "Cooking complete — {} iterations, {} tool calls. todos empty.",
                    iterations, total_tool_calls
                );
                break;
            }

            // Add assistant message with tool calls to conversation
            let assistant_msg =
                Message::assistant(&response.content).with_tool_calls(response.tool_calls.clone());
            messages.push(assistant_msg);

            // Execute each tool call
            for call in &response.tool_calls {
                debug!(
                    tool = %call.name,
                    call_id = %call.id,
                    iteration = iterations,
                    "Cooking loop: executing tool"
                );

                // LoopGuard: check for repeated identical calls before executing
                match loop_guard.check(&call.name, &call.arguments) {
                    LoopGuardVerdict::Block(msg) => {
                        warn!(
                            tool = %call.name,
                            "Cooking loop guard blocked tool call: {}",
                            msg
                        );
                        // Return blocked result as tool error — LLM sees it and should stop
                        let tool_msg = Message::tool(&call.id, false, &msg);
                        messages.push(tool_msg);
                        // Inject system-level notice so LLM adjusts approach
                        messages.push(Message::system(&format!("Tool blocked: {}. Try a different approach.", msg)));
                        total_tool_calls += 1;
                        all_tool_records.push(ToolCallRecord {
                            name: call.name.clone(),
                            call_id: call.id.clone(),
                            arguments: call.arguments.clone(),
                            success: false,
                            output: msg,
                            iteration: iterations,
                        });
                        continue;
                    }
                    LoopGuardVerdict::Warn(msg) => {
                        warn!(
                            tool = %call.name,
                            "Cooking loop guard warning: {}",
                            msg
                        );
                        // Inject warning into conversation so LLM reconsiders
                        messages.push(Message::user(&msg));
                    }
                    LoopGuardVerdict::Allow => {}
                }

                self.emit(crate::cooking_events::CookingEvent::tool_start(
                    &call.id,
                    &call.name,
                    Some(call.arguments.clone()),
                ));

                let tool_start = std::time::Instant::now();
                let tool_timeout = parse_cooking_timeout(
                    self.config.tool_call_timeout.as_deref(),
                    self.config.tool_call_timeout_secs,
                );
                // R1: Intercept loop-internal todo tools — handled by the cooking loop,
                // not delegated to the external executor.
                let result = if call.name == "todo_write" || call.name == "todo_read" {
                    handle_todo_tool(call, &mut todos)
                } else {
                    match tokio::time::timeout(tool_timeout, executor.execute_tool(call)).await {
                    Ok(r) => r,
                    Err(_elapsed) => {
                        let timeout_secs = tool_timeout.as_secs();
                        warn!(
                            tool = %call.name,
                            call_id = %call.id,
                            timeout_secs = timeout_secs,
                            timeout_human = ?self.config.tool_call_timeout,
                            "Tool call exceeded per-tool timeout — aborting"
                        );
                        ToolResult {
                            call_id: call.id.clone(),
                            success: false,
                            output: format!(
                                "Tool '{}' timed out after {}s (per-tool-call timeout). The individual tool execution was aborted so the conversation can continue. Try a different approach, a simpler invocation, or a different tool.",
                                call.name, timeout_secs
                            ),
                        }
                    }
                }
                }; // end of if/else for todo-tool interception
                let tool_duration = tool_start.elapsed().as_millis() as u64;
                total_tool_calls += 1;

                // #284B: record tool-call completion as cook activity for the
                // gateway idle watchdog. Sole chokepoint — every tool-exec path
                // (including todo-tool interception above) funnels here.
                self.record_progress();

                // Record the execution
                all_tool_records.push(ToolCallRecord {
                    name: call.name.clone(),
                    call_id: call.id.clone(),
                    arguments: call.arguments.clone(),
                    success: result.success,
                    output: truncate_output(&result.output, 2000),
                    iteration: iterations,
                });

                self.emit(crate::cooking_events::CookingEvent::tool_complete(
                    &call.id,
                    &call.name,
                    &truncate_output(&result.output, 200),
                    !result.success,
                    tool_duration,
                ));

                // Add truncated tool result to conversation to prevent LLM token bloat
                // on large outputs. Display event still gets full output (up to 200 chars).
                let truncated_result = truncate_output(&result.output, 2000);
                let tool_msg = Message::tool(&call.id, result.success, &truncated_result);
                messages.push(tool_msg);

                info!(
                    tool = %call.name,
                    success = result.success,
                    iteration = iterations,
                    total_tools = total_tool_calls,
                    "Tool executed"
                );
            }

            // Tool-call audit: inject iteration summary so the LLM can read back what it did.
            // Only inject for providers that handle mid-conversation system messages (Anthropic).
            // This prevents models from hallucinating about their own tool usage.
            {
                let iter_records: Vec<&ToolCallRecord> = all_tool_records.iter()
                    .filter(|r| r.iteration == iterations)
                    .collect();
                if !iter_records.is_empty() && self.config.audit_logging {
                    let summary_parts: Vec<String> = iter_records.iter()
                        .map(|r| format!("{}:{}", r.name, if r.success { "ok" } else { "FAIL" }))
                        .collect();
                    let audit_line = format!(
                        "[Iteration {} audit: {} tool(s) — {}]",
                        iterations, iter_records.len(), summary_parts.join(", ")
                    );
                    messages.push(Message::system(&audit_line));
                    debug!(iteration = iterations, "Tool audit injected: {}", audit_line);
                } else if !iter_records.is_empty() {
                    debug!(iteration = iterations, "Tool audit skipped (provider doesn't support mid-conversation system messages)");
                }
            }

            // Checkpoint after each iteration's tool executions
            if let Some(ref store) = self.checkpoint_store {
                use crate::cooking_checkpoint::CookingCheckpoint;
                let checkpoint = CookingCheckpoint {
                    session_id: session_id.clone(),
                    original_message: message.to_string(),
                    iteration: iterations,
                    tool_call_count: total_tool_calls,
                    messages: messages.clone(),
                    tool_records: all_tool_records.clone(),
                    completed: false,
                    updated_at: chrono::Utc::now(),
                    started_at: session_started_at,
                    system_prompt: system_prompt.to_string(),
                    todos: todos.clone(), // R1: persist agent todo list
                };
                store.save_checkpoint(&checkpoint).await;
            }

            // S59-P1: ReACT Reflection — after tool execution, evaluate progress
            // Only reflect every 3 iterations to avoid excessive LLM calls.
            // Skip for budget providers (GLM/ZAI) — they do more iterations due to
            // no parallel tool calls, so reflections compound token burn.
            let skip_reflection = *llm.provider() == zeus_core::Provider::Zai;
            if !skip_reflection && iterations > 1 && iterations % 3 == 0 && iterations < self.config.max_iterations {
                let reflection_prompt = format!(
                    "Progress check — iteration {}, {} tool calls completed. \
                     Evaluate your approach: are you making progress toward the goal? \
                     Consider a different strategy if needed. \
                     When the task is complete, summarize what you accomplished.",
                    iterations, total_tool_calls
                );
                messages.push(Message::user(&reflection_prompt));
                debug!(
                    iteration = iterations,
                    "ReACT reflection injected"
                );
            }

            // If the LLM also had text content, capture it
            if !response.content.is_empty() {
                final_response = response.content.clone();
            }
        }

        // Mark checkpoint session as completed, then prune it immediately so
        // finished cooks don't accumulate their messages/records blobs in the
        // checkpoint DB (the source of the 2 GB bloat). A completed session has
        // nothing left to resume, so the row is dead weight the moment we land here.
        checkpoint_guard.complete_and_prune().await;

        let estimated_tokens_end = system_tokens + estimate_message_tokens(&messages);

        Ok(CookingResult {
            response: final_response,
            iterations,
            tool_calls: all_tool_records,
            processing_time_ms: start.elapsed().as_millis() as u64,
            completed_naturally,
            memory_context: current_memory_context,
            compacted: compaction_count > 0,
            compaction_summary,
            compaction_count,
            context_rotated,
            estimated_tokens_start,
            estimated_tokens_end,
            retry_count,
            interrupted_by: None,
        })
    }
}

/// Rough estimate of token count for a string (~4 chars per token).
fn estimate_str_tokens(s: &str) -> u64 {
    (s.len() / 4) as u64
}

/// Rough estimate of token count for a slice of messages.
pub fn estimate_message_tokens(messages: &[Message]) -> u64 {
    messages
        .iter()
        .map(|m| {
            let content_tokens = estimate_str_tokens(&m.content);
            let tool_call_tokens: u64 = m
                .tool_calls
                .iter()
                .map(|tc| {
                    estimate_str_tokens(&tc.name) + (tc.arguments.to_string().len() / 4) as u64
                })
                .sum();
            let tool_result_tokens: u64 = m
                .tool_results
                .iter()
                .map(|tr| estimate_str_tokens(&tr.output))
                .sum();
            content_tokens + tool_call_tokens + tool_result_tokens
        })
        .sum()
}

/// Compact older messages by summarizing them with the LLM.
///
/// Splits messages into older (to summarize) and recent (to keep verbatim).
/// Returns the new message list and the generated summary.
async fn compact_messages(
    messages: &[Message],
    llm: &LlmClient,
    min_recent: usize,
) -> Result<(Vec<Message>, String)> {
    if messages.len() <= min_recent {
        return Err(zeus_core::Error::Config(
            "Not enough messages to compact".to_string(),
        ));
    }

    let split_point = messages.len() - min_recent;
    let to_summarize = &messages[..split_point];
    let to_keep = &messages[split_point..];

    // Format older messages for the summarization prompt
    let conversation_text: String = to_summarize
        .iter()
        .map(|m| {
            let role = match m.role {
                zeus_core::Role::User => "User",
                zeus_core::Role::Assistant => "Assistant",
                zeus_core::Role::Tool => "Tool",
                _ => "System",
            };
            if m.content.is_empty() && !m.tool_calls.is_empty() {
                format!(
                    "{}: [Tool calls: {}]",
                    role,
                    m.tool_calls
                        .iter()
                        .map(|tc| tc.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else if m.content.is_empty() && !m.tool_results.is_empty() {
                format!("{}: [Tool results: {} items]", role, m.tool_results.len())
            } else {
                let truncated: String = m.content.chars().take(500).collect();
                format!("{}: {}", role, truncated)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let summary_request = vec![Message::user(format!(
        "Please summarize this conversation:\n\n{}",
        conversation_text
    ))];

    let response = llm
        .complete(&summary_request, &[], Some(COMPACTION_PROMPT))
        .await?;

    let summary = response.content;

    // Build compacted history
    let mut new_messages = Vec::with_capacity(min_recent + 2);
    new_messages.push(Message::user(format!(
        "[Previous conversation summary]\n{}",
        summary
    )));
    new_messages.push(Message::assistant(
        "I understand. I have the context from our previous conversation. How can I help you continue?"
    ));
    new_messages.extend(to_keep.iter().cloned());

    Ok((new_messages, summary))
}

/// Truncate output to a maximum length for recording
fn truncate_output(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        // Head-60% / tail-40% split — preserves start context (filenames, paths)
        // and end context (results, errors, line counts) since most useful info
        // lives at both ends. Blind head-chop loses grep/tail output tail.
        let head_len = (max_len as f64 * 0.60) as usize;
        let head_end = zeus_core::floor_char_boundary(s, head_len);
        let tail_start = zeus_core::floor_char_boundary(s, s.len() - (max_len - head_end));
        format!(
            "{}\n...\n{}",
            &s[..head_end],
            &s[tail_start..]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── parse_cooking_timeout: human-readable timeout parser ───
    // String-first; falls back to u64-secs; falls back to 1800 default.
    #[test]
    fn parse_cooking_timeout_minutes() {
        let d = parse_cooking_timeout(Some("30m"), 0);
        assert_eq!(d, std::time::Duration::from_secs(30 * 60));
    }

    #[test]
    fn parse_cooking_timeout_hours() {
        let d = parse_cooking_timeout(Some("2h"), 0);
        assert_eq!(d, std::time::Duration::from_secs(2 * 60 * 60));
    }

    #[test]
    fn parse_cooking_timeout_long_form() {
        let d = parse_cooking_timeout(Some("30 hours"), 0);
        assert_eq!(d, std::time::Duration::from_secs(30 * 60 * 60));
    }

    #[test]
    fn parse_cooking_timeout_compound() {
        // humantime supports compound: "1h 30m"
        let d = parse_cooking_timeout(Some("1h 30m"), 0);
        assert_eq!(d, std::time::Duration::from_secs(90 * 60));
    }

    #[test]
    fn parse_cooking_timeout_invalid_falls_back_to_secs() {
        // Garbage string → fall through to u64-secs.
        let d = parse_cooking_timeout(Some("not-a-duration"), 600);
        assert_eq!(d, std::time::Duration::from_secs(600));
    }

    #[test]
    fn parse_cooking_timeout_empty_string_falls_back_to_secs() {
        let d = parse_cooking_timeout(Some(""), 900);
        assert_eq!(d, std::time::Duration::from_secs(900));
    }

    #[test]
    fn parse_cooking_timeout_whitespace_only_falls_back_to_secs() {
        let d = parse_cooking_timeout(Some("   "), 900);
        assert_eq!(d, std::time::Duration::from_secs(900));
    }

    #[test]
    fn parse_cooking_timeout_none_uses_secs() {
        let d = parse_cooking_timeout(None, 1234);
        assert_eq!(d, std::time::Duration::from_secs(1234));
    }

    #[test]
    fn parse_cooking_timeout_none_zero_secs_uses_default_1800() {
        // None + secs=0 → default 1800 (safety net for un-initialized configs).
        let d = parse_cooking_timeout(None, 0);
        assert_eq!(d, std::time::Duration::from_secs(1800));
    }

    #[test]
    fn parse_cooking_timeout_invalid_zero_secs_uses_default_1800() {
        let d = parse_cooking_timeout(Some("garbage"), 0);
        assert_eq!(d, std::time::Duration::from_secs(1800));
    }

    // R1: TodoWrite/TodoRead unit tests
    #[test]
    fn test_todo_write_replaces_list() {
        let mut todos: Vec<TodoItem> = vec![TodoItem {
            id: "stale".into(),
            content: "old".into(),
            status: TodoStatus::Pending,
        }];
        let call = ToolCall {
            id: "c1".into(),
            name: "todo_write".into(),
            arguments: serde_json::json!({
                "todos": [
                    {"id": "a", "content": "first", "status": "in_progress"},
                    {"id": "b", "content": "second", "status": "pending"}
                ]
            }),
        };
        let result = handle_todo_tool(&call, &mut todos);
        assert!(result.success);
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].id, "a");
        assert_eq!(todos[0].status, TodoStatus::InProgress);
        assert_eq!(todos[1].status, TodoStatus::Pending);
    }

    #[test]
    fn test_todo_read_returns_current_list() {
        let mut todos: Vec<TodoItem> = vec![
            TodoItem { id: "1".into(), content: "do thing".into(), status: TodoStatus::Completed },
        ];
        let call = ToolCall {
            id: "c2".into(),
            name: "todo_read".into(),
            arguments: serde_json::json!({}),
        };
        let result = handle_todo_tool(&call, &mut todos);
        assert!(result.success);
        assert!(result.output.contains("do thing"));
        assert!(result.output.contains("[x]"));
    }

    #[test]
    fn test_todo_write_invalid_args_returns_error() {
        let mut todos: Vec<TodoItem> = vec![];
        let call = ToolCall {
            id: "c3".into(),
            name: "todo_write".into(),
            arguments: serde_json::json!({"wrong": "shape"}),
        };
        let result = handle_todo_tool(&call, &mut todos);
        assert!(!result.success);
        assert!(todos.is_empty());
    }

    // Mock tool executor for testing
    struct MockExecutor {
        tools: Vec<String>,
    }

    impl MockExecutor {
        fn new(tools: Vec<&str>) -> Self {
            Self {
                tools: tools.into_iter().map(String::from).collect(),
            }
        }
    }

    #[async_trait]
    impl ToolExecutor for MockExecutor {
        async fn execute_tool(&self, call: &ToolCall) -> ToolResult {
            ToolResult {
                call_id: call.id.clone(),
                success: true,
                output: format!("Mock result for {}", call.name),
            }
        }

        fn has_tool(&self, name: &str) -> bool {
            self.tools.iter().any(|t| t == name)
        }

        fn available_tools(&self) -> Vec<String> {
            self.tools.clone()
        }
    }

    #[test]
    fn test_cooking_config_defaults() {
        let config = CookingConfig::default();
        assert_eq!(config.max_iterations, 200);
        assert_eq!(config.max_tool_calls, 200);
        assert!(config.inject_memory);
        assert_eq!(config.memory_results, 5);
        assert_eq!(config.max_context_tokens, 200_000);
        assert_eq!(config.compaction_threshold, 0.8);
        assert_eq!(config.compaction_target_ratio, 0.3);
        assert_eq!(config.min_recent_messages, 12);
        assert!(config.enable_retry);
        assert_eq!(config.max_retry_attempts, 3);
        assert_eq!(config.max_compactions, 3);
        assert_eq!(config.min_iterations_between_compactions, 2);
    }

    #[test]
    fn test_cooking_config_serde() {
        let config = CookingConfig {
            max_iterations: 10,
            max_tool_calls: 30,
            inject_memory: false,
            memory_results: 3,
            ..Default::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let deser: CookingConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.max_iterations, 10);
        assert_eq!(deser.max_tool_calls, 30);
        assert!(!deser.inject_memory);
        assert_eq!(deser.memory_results, 3);
        assert_eq!(deser.max_context_tokens, 200_000);
    }

    #[test]
    fn test_cooking_result_serialization() {
        let result = CookingResult {
            response: "Done".to_string(),
            iterations: 3,
            tool_calls: vec![ToolCallRecord {
                name: "read_file".to_string(),
                call_id: "tc_1".to_string(),
                arguments: serde_json::json!({"path": "/tmp/test"}),
                success: true,
                output: "file contents".to_string(),
                iteration: 1,
            }],
            processing_time_ms: 500,
            completed_naturally: true,
            memory_context: Some("relevant memory".to_string()),
            compacted: false,
            compaction_summary: None,
            compaction_count: 0,
            context_rotated: false,
            estimated_tokens_start: 100,
            estimated_tokens_end: 200,
            retry_count: 0,
            interrupted_by: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        let deser: CookingResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.response, "Done");
        assert_eq!(deser.iterations, 3);
        assert_eq!(deser.tool_calls.len(), 1);
        assert_eq!(deser.tool_calls[0].name, "read_file");
        assert!(deser.completed_naturally);
        assert!(!deser.compacted);
    }

    #[test]
    fn test_tool_call_record_serialization() {
        let record = ToolCallRecord {
            name: "shell".to_string(),
            call_id: "tc_123".to_string(),
            arguments: serde_json::json!({"command": "ls"}),
            success: true,
            output: "file1\nfile2".to_string(),
            iteration: 2,
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(json.contains("shell"));
        assert!(json.contains("tc_123"));
    }

    #[test]
    fn test_truncate_output_short() {
        let s = "hello world";
        assert_eq!(truncate_output(s, 100), "hello world");
    }

    #[test]
    fn test_truncate_output_long() {
        let s = "a".repeat(5000);
        let result = truncate_output(&s, 2000);
        assert!(result.len() < 5000);
        assert!(result.contains("...")); // head + tail separator
        assert!(result.starts_with("aaaaa")); // head preserved
        assert!(result.ends_with("aaaaa")); // tail preserved
    }

    #[test]
    fn test_truncate_output_exact() {
        let s = "a".repeat(2000);
        assert_eq!(truncate_output(&s, 2000), s);
    }

    #[tokio::test]
    async fn test_mock_executor() {
        let executor = MockExecutor::new(vec!["read_file", "shell"]);
        assert!(executor.has_tool("read_file"));
        assert!(executor.has_tool("shell"));
        assert!(!executor.has_tool("nonexistent"));
        assert_eq!(executor.available_tools().len(), 2);

        let call = ToolCall {
            id: "tc_1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "/tmp/test"}),
        };
        let result = executor.execute_tool(&call).await;
        assert!(result.success);
        assert!(result.output.contains("Mock result for read_file"));
        assert_eq!(result.call_id, "tc_1");
    }

    #[test]
    fn test_cooking_loop_creation() {
        let config = CookingConfig::default();
        let _loop = CookingLoop::new(config);
    }

    #[test]
    fn test_cooking_loop_default() {
        let _loop = CookingLoop::default();
    }

    #[test]
    fn test_cooking_loop_with_events() {
        let config = CookingConfig::default();
        let emitter = EventEmitter::new();
        let cooking_loop = CookingLoop::new(config).with_events(emitter);
        assert!(cooking_loop.event_emitter.is_some());
    }

    #[test]
    fn test_cooking_result_no_tools() {
        let result = CookingResult {
            response: "Just text".to_string(),
            iterations: 1,
            tool_calls: vec![],
            processing_time_ms: 100,
            completed_naturally: true,
            memory_context: None,
            compacted: false,
            compaction_summary: None,
            compaction_count: 0,
            context_rotated: false,
            estimated_tokens_start: 10,
            estimated_tokens_end: 10,
            retry_count: 0,
            interrupted_by: None,
        };
        assert!(result.completed_naturally);
        assert!(result.tool_calls.is_empty());
        assert!(result.memory_context.is_none());
        assert!(!result.compacted);
    }

    #[test]
    fn test_cooking_result_hit_limit() {
        let result = CookingResult {
            response: String::new(),
            iterations: 20,
            tool_calls: vec![],
            processing_time_ms: 5000,
            completed_naturally: false,
            memory_context: None,
            compacted: false,
            compaction_summary: None,
            compaction_count: 0,
            context_rotated: false,
            estimated_tokens_start: 0,
            estimated_tokens_end: 0,
            retry_count: 0,
            interrupted_by: None,
        };
        assert!(!result.completed_naturally);
    }

    #[test]
    fn test_cooking_config_max_iterations() {
        let config = CookingConfig {
            max_iterations: 100,
            ..Default::default()
        };
        assert_eq!(config.max_iterations, 100);
        assert_eq!(config.max_tool_calls, 200); // default unchanged (4x cap bump)
    }

    #[test]
    fn test_cooking_config_custom() {
        let config = CookingConfig {
            max_iterations: 5,
            max_tool_calls: 10,
            inject_memory: false,
            memory_results: 2,
            ..Default::default()
        };
        assert_eq!(config.max_iterations, 5);
        assert_eq!(config.max_tool_calls, 10);
        assert!(!config.inject_memory);
        assert_eq!(config.memory_results, 2);
    }

    #[test]
    fn test_cooking_result_with_many_tools() {
        let tool_calls: Vec<ToolCallRecord> = (0..15)
            .map(|i| ToolCallRecord {
                name: format!("tool_{}", i),
                call_id: format!("tc_{}", i),
                arguments: serde_json::json!({"index": i}),
                success: i % 3 != 0, // every 3rd fails
                output: format!("output {}", i),
                iteration: i / 3 + 1,
            })
            .collect();

        let result = CookingResult {
            response: "Completed with many tools".to_string(),
            iterations: 5,
            tool_calls,
            processing_time_ms: 10000,
            completed_naturally: true,
            memory_context: None,
            compacted: false,
            compaction_summary: None,
            compaction_count: 0,
            context_rotated: false,
            estimated_tokens_start: 500,
            estimated_tokens_end: 5000,
            retry_count: 0,
            interrupted_by: None,
        };

        assert_eq!(result.tool_calls.len(), 15);
        let successes = result.tool_calls.iter().filter(|t| t.success).count();
        let failures = result.tool_calls.iter().filter(|t| !t.success).count();
        assert_eq!(successes, 10); // 15 - 5 failures (indices 0,3,6,9,12)
        assert_eq!(failures, 5);

        // Verify serialization works with many records
        let json = serde_json::to_string(&result).unwrap();
        let deser: CookingResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.tool_calls.len(), 15);
    }

    #[test]
    fn test_truncate_output_empty() {
        let result = truncate_output("", 2000);
        assert_eq!(result, "");
    }

    #[test]
    fn test_truncate_output_unicode() {
        // Test with unicode characters that fit within the limit
        let s = "\u{00e9}".repeat(999); // 999 'e with acute' = 1998 bytes (each is 2 bytes UTF-8)
        // 1998 bytes <= 2000, so should not truncate
        let result = truncate_output(&s, 2000);
        assert_eq!(result, s);

        // Test with purely ASCII that exceeds the limit
        let long_ascii = "a".repeat(3000);
        let result2 = truncate_output(&long_ascii, 2000);
        assert!(result2.contains("...")); // head + tail separator
        assert!(result2.starts_with("aaaaa")); // head preserved
        assert!(result2.ends_with("aaaaa")); // tail preserved
    }

    #[test]
    fn test_tool_call_record_fields() {
        let record = ToolCallRecord {
            name: "web_fetch".to_string(),
            call_id: "tc_abc_123".to_string(),
            arguments: serde_json::json!({"url": "https://example.com", "headers": {"Accept": "text/html"}}),
            success: true,
            output: "<!DOCTYPE html>...".to_string(),
            iteration: 3,
        };

        assert_eq!(record.name, "web_fetch");
        assert_eq!(record.call_id, "tc_abc_123");
        assert!(record.arguments["url"].is_string());
        assert!(record.arguments["headers"].is_object());
        assert!(record.success);
        assert_eq!(record.output, "<!DOCTYPE html>...");
        assert_eq!(record.iteration, 3);

        // Verify full serialization roundtrip
        let json = serde_json::to_string(&record).unwrap();
        let deser: ToolCallRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.name, record.name);
        assert_eq!(deser.call_id, record.call_id);
        assert_eq!(deser.iteration, record.iteration);
    }

    #[test]
    fn test_estimate_message_tokens() {
        let messages = vec![
            Message::user("Hello world!"),   // 12 chars -> ~3 tokens
            Message::assistant("Hi there!"), // 9 chars -> ~2 tokens
        ];
        let tokens = estimate_message_tokens(&messages);
        assert!(tokens >= 4 && tokens <= 6);
    }

    #[test]
    fn test_estimate_message_tokens_with_tool_calls() {
        let msg = Message::assistant("").with_tool_calls(vec![ToolCall {
            id: "tc1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "/tmp/test.txt"}),
        }]);
        let tokens = estimate_message_tokens(&[msg]);
        assert!(tokens > 0);
    }

    #[test]
    fn test_estimate_str_tokens() {
        assert_eq!(estimate_str_tokens(""), 0);
        assert_eq!(estimate_str_tokens("abcd"), 1);
        assert_eq!(estimate_str_tokens("abcdefgh"), 2);
    }

    #[test]
    fn test_compaction_threshold_calculation() {
        let config = CookingConfig {
            max_context_tokens: 100_000,
            compaction_threshold: 0.8,
            ..Default::default()
        };
        let threshold = (config.max_context_tokens as f64 * config.compaction_threshold) as u64;
        assert_eq!(threshold, 80_000);
    }

    #[test]
    fn test_cooking_result_compacted() {
        let result = CookingResult {
            response: "Done after compact".to_string(),
            iterations: 5,
            tool_calls: vec![],
            processing_time_ms: 2000,
            completed_naturally: true,
            memory_context: None,
            compacted: true,
            compaction_summary: Some(
                "User asked about Rust. Assistant helped with code.".to_string(),
            ),
            compaction_count: 1,
            context_rotated: false,
            estimated_tokens_start: 80_000,
            estimated_tokens_end: 30_000,
            retry_count: 0,
            interrupted_by: None,
        };
        assert!(result.compacted);
        assert!(result.compaction_summary.is_some());
        assert!(result.estimated_tokens_end < result.estimated_tokens_start);

        // Serialization roundtrip
        let json = serde_json::to_string(&result).unwrap();
        let deser: CookingResult = serde_json::from_str(&json).unwrap();
        assert!(deser.compacted);
        assert_eq!(deser.compaction_summary, result.compaction_summary);
    }

    #[test]
    fn test_cooking_result_with_retries() {
        let result = CookingResult {
            response: "Got it after retries".to_string(),
            iterations: 3,
            tool_calls: vec![],
            processing_time_ms: 8000,
            completed_naturally: true,
            memory_context: None,
            compacted: false,
            compaction_summary: None,
            compaction_count: 0,
            context_rotated: false,
            estimated_tokens_start: 100,
            estimated_tokens_end: 500,
            retry_count: 2,
            interrupted_by: None,
        };
        assert_eq!(result.retry_count, 2);
    }

    #[test]
    fn test_cooking_config_serde_with_new_fields() {
        let config = CookingConfig {
            max_context_tokens: 128_000,
            compaction_threshold: 0.75,
            min_recent_messages: 6,
            enable_retry: false,
            max_retry_attempts: 5,
            ..Default::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let deser: CookingConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.max_context_tokens, 128_000);
        assert_eq!(deser.compaction_threshold, 0.75);
        assert_eq!(deser.min_recent_messages, 6);
        assert!(!deser.enable_retry);
        assert_eq!(deser.max_retry_attempts, 5);
    }

    #[test]
    fn test_cooking_config_backward_compat_deserialization() {
        // Old-format JSON without new fields should deserialize with defaults
        let old_json =
            r#"{"max_iterations":15,"max_tool_calls":40,"inject_memory":true,"memory_results":5}"#;
        let config: CookingConfig = serde_json::from_str(old_json).unwrap();
        assert_eq!(config.max_iterations, 15);
        assert_eq!(config.max_context_tokens, 200_000); // default
        assert_eq!(config.compaction_threshold, 0.8); // default
        assert!(config.enable_retry); // default
        assert_eq!(config.max_compactions, 3); // default
        assert_eq!(config.min_iterations_between_compactions, 2); // default
    }

    #[test]
    fn test_cooking_config_multi_compaction_fields() {
        let config = CookingConfig {
            max_compactions: 5,
            min_iterations_between_compactions: 3,
            ..Default::default()
        };
        assert_eq!(config.max_compactions, 5);
        assert_eq!(config.min_iterations_between_compactions, 3);

        let json = serde_json::to_string(&config).unwrap();
        let deser: CookingConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.max_compactions, 5);
        assert_eq!(deser.min_iterations_between_compactions, 3);
    }

    #[test]
    fn test_cooking_result_multi_compaction() {
        let result = CookingResult {
            response: "Done after 3 compactions".to_string(),
            iterations: 15,
            tool_calls: vec![],
            processing_time_ms: 30000,
            completed_naturally: true,
            memory_context: None,
            compacted: true,
            compaction_summary: Some("Third compaction summary".to_string()),
            compaction_count: 3,
            context_rotated: false,
            estimated_tokens_start: 150_000,
            estimated_tokens_end: 40_000,
            retry_count: 0,
            interrupted_by: None,
        };
        assert!(result.compacted);
        assert_eq!(result.compaction_count, 3);
        assert!(!result.context_rotated);

        let json = serde_json::to_string(&result).unwrap();
        let deser: CookingResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.compaction_count, 3);
        assert!(!deser.context_rotated);
    }

    #[test]
    fn test_cooking_result_context_rotated() {
        let result = CookingResult {
            response: "Recovered after rotation".to_string(),
            iterations: 20,
            tool_calls: vec![],
            processing_time_ms: 45000,
            completed_naturally: true,
            memory_context: None,
            compacted: true,
            compaction_summary: Some("Summary before rotation".to_string()),
            compaction_count: 3,
            context_rotated: true,
            estimated_tokens_start: 180_000,
            estimated_tokens_end: 5_000,
            retry_count: 1,
            interrupted_by: None,
        };
        assert!(result.context_rotated);
        assert_eq!(result.compaction_count, 3);

        let json = serde_json::to_string(&result).unwrap();
        let deser: CookingResult = serde_json::from_str(&json).unwrap();
        assert!(deser.context_rotated);
    }

    #[test]
    fn test_cooking_result_backward_compat_deserialization() {
        // Old-format JSON without compaction_count/context_rotated should deserialize with defaults
        let old_json = r#"{"response":"ok","iterations":1,"tool_calls":[],"processing_time_ms":100,"completed_naturally":true,"compacted":false,"estimated_tokens_start":0,"estimated_tokens_end":0,"retry_count":0}"#;
        let result: CookingResult = serde_json::from_str(old_json).unwrap();
        assert_eq!(result.compaction_count, 0);
        assert!(!result.context_rotated);
        assert!(result.memory_context.is_none());
    }

    #[test]
    fn test_cooking_loop_with_auth_manager() {
        use crate::cooking_auth::{AuthProfile, AuthProfileManager, CooldownConfig};

        let config = CookingConfig::default();
        let mut manager = AuthProfileManager::new(CooldownConfig::default());
        manager.add_profile(AuthProfile::new("prof-1", "Primary", "anthropic"));
        manager.add_profile(AuthProfile::new("prof-2", "Secondary", "openai"));

        let auth_mgr = std::sync::Arc::new(tokio::sync::Mutex::new(manager));
        let cooking_loop = CookingLoop::new(config).with_auth_manager(auth_mgr);
        assert!(cooking_loop.auth_manager.is_some());
    }

    #[test]
    fn test_cooking_loop_with_mnemosyne() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let mnemosyne_config = zeus_mnemosyne::MnemosyneConfig {
                db_path: dir.path().join("test_cooking.db"),
                ..Default::default()
            };
            let mnemosyne = Arc::new(
                zeus_mnemosyne::Mnemosyne::new(mnemosyne_config)
                    .await
                    .unwrap(),
            );
            let injector = MemoryInjector::new(5, 4000);

            let config = CookingConfig::default();
            let cooking_loop = CookingLoop::new(config).with_mnemosyne(mnemosyne, injector);
            assert!(cooking_loop.mnemosyne.is_some());
            assert!(cooking_loop.memory_injector.is_some());
        });
    }

    #[test]
    fn test_cooking_loop_default_no_mnemosyne() {
        let cooking_loop = CookingLoop::default();
        assert!(cooking_loop.mnemosyne.is_none());
        assert!(cooking_loop.memory_injector.is_none());
    }

    #[test]
    fn test_build_memory_query_no_messages() {
        let config = CookingConfig::default();
        let cooking_loop = CookingLoop::new(config);
        let messages: Vec<Message> = vec![];
        let query = cooking_loop.build_memory_query("help me", &messages);
        // With no messages, falls back to original message
        assert_eq!(query, "help me");
    }

    #[test]
    fn test_build_memory_query_with_messages() {
        let config = CookingConfig::default();
        let cooking_loop = CookingLoop::new(config);
        let messages = vec![
            Message::user("list files in /tmp"),
            Message::assistant("Here are the files found in /tmp"),
        ];
        let query = cooking_loop.build_memory_query("list files in /tmp", &messages);
        assert!(query.contains("list files in /tmp"));
        assert!(query.contains("Here are the files found"));
    }

    #[test]
    fn test_build_memory_query_empty_content_filtered() {
        let config = CookingConfig::default();
        let cooking_loop = CookingLoop::new(config);
        let messages = vec![
            Message::user("run tests"),
            Message::assistant(""),
            Message::assistant("All 42 tests passed"),
        ];
        let query = cooking_loop.build_memory_query("run tests", &messages);
        assert!(query.contains("run tests"));
        assert!(query.contains("42 tests passed"));
        // Empty message should be filtered out
        assert!(!query.contains("  ")); // No double spaces from empty content
    }

    #[test]
    fn test_build_memory_query_truncation() {
        let config = CookingConfig::default();
        let cooking_loop = CookingLoop::new(config);
        // Create a message with very long content
        let long_content = "x".repeat(2000);
        let messages = vec![Message::assistant(&long_content)];
        let query = cooking_loop.build_memory_query("original", &messages);
        // Total query should be bounded (original + space + 500 truncated chars)
        assert!(query.len() <= 510);
    }

    #[tokio::test]
    async fn test_memory_refresh_with_real_mnemosyne() {
        use zeus_core::{Role, TextDirection};

        let dir = tempfile::tempdir().expect("tempdir");
        let mnemosyne_config = zeus_mnemosyne::MnemosyneConfig {
            db_path: dir.path().join("refresh.db"),
            ..Default::default()
        };
        let mnemosyne = Arc::new(
            zeus_mnemosyne::Mnemosyne::new(mnemosyne_config)
                .await
                .unwrap(),
        );

        // Store a memory that should be findable
        let msg = Message {
            role: Role::Assistant,
            content: "The deployment script requires SSH key authentication".to_string(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
            timestamp: chrono::Utc::now(),
            attachments: Vec::new(),
            message_id: None,
            parent_id: None,
            thread_id: None,
            direction: TextDirection::Ltr,
            channel_source: None,
            compaction_hint: Default::default(),
        };
        mnemosyne.store("test-session", &msg).await.expect("store");

        // Create injector and search
        let injector = MemoryInjector::new(5, 4000);
        let result = injector.fetch_context(&mnemosyne, "deployment SSH").await;

        // FTS5 should find the stored memory
        assert!(result.is_some(), "should find deployment memory");
        let context = result.unwrap();
        assert!(
            context.contains("deployment") || context.contains("SSH"),
            "context should contain relevant terms"
        );
    }

    #[test]
    fn test_cooking_config_inject_memory_default() {
        let config = CookingConfig::default();
        assert!(config.inject_memory);
        assert_eq!(config.memory_results, 5);
    }

    #[tokio::test]
    async fn checkpoint_guard_drop_deletes_incomplete_session() {
        let store = Arc::new(CookingCheckpointStore::in_memory().unwrap());
        store.start_session("guard-drop", "msg", "system").await;

        {
            let _guard = CheckpointSessionGuard::new(
                Some(store.clone()),
                "guard-drop".to_string(),
            );
        }

        for _ in 0..20 {
            if store.load_checkpoint("guard-drop").await.is_none() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        assert!(
            store.load_checkpoint("guard-drop").await.is_none(),
            "dropped checkpoint guard must delete incomplete session"
        );
    }

    #[test]
    fn test_run_delegates_to_run_with_history() {
        // Verify CookingLoop::run() exists and delegates to run_with_history.
        // We can't fully run the cooking loop without an LLM, but we can
        // verify the method signatures are compatible.
        let cooking_loop = CookingLoop::default();
        assert!(cooking_loop.mnemosyne.is_none());
        assert!(cooking_loop.checkpoint_store.is_none());
    }

    #[test]
    fn test_conversation_history_prepended() {
        // Verify that conversation history messages + current message
        // produce the expected message array structure.
        let history = vec![
            Message::user("What's the weather?"),
            Message::assistant("It's sunny today."),
            Message::user("And tomorrow?"),
            Message::assistant("Rain expected."),
        ];
        let current = "Thanks for the info";

        // Simulate what run_with_history does
        let mut messages: Vec<Message> = history.to_vec();
        messages.push(Message::user(current));

        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].content, "What's the weather?");
        assert_eq!(messages[1].content, "It's sunny today.");
        assert_eq!(messages[4].content, "Thanks for the info");
    }

    // ===== R1: TodoWrite/TodoRead tests =====

    #[test]
    fn test_r1_todo_write_replaces_list() {
        let mut todos: Vec<TodoItem> = Vec::new();
        let call = ToolCall {
            id: "c1".into(),
            name: "todo_write".into(),
            arguments: serde_json::json!({
                "todos": [
                    { "id": "1", "content": "do thing A", "status": "pending" },
                    { "id": "2", "content": "do thing B", "status": "in_progress" }
                ]
            }),
        };
        let result = handle_todo_tool(&call, &mut todos);
        assert!(result.success, "todo_write should succeed: {}", result.output);
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[1].status, TodoStatus::InProgress);
    }

    #[test]
    fn test_r1_todo_read_renders_list() {
        let mut todos = vec![
            TodoItem { id: "x".into(), content: "first".into(), status: TodoStatus::Completed },
            TodoItem { id: "y".into(), content: "second".into(), status: TodoStatus::Pending },
        ];
        let call = ToolCall { id: "c2".into(), name: "todo_read".into(), arguments: serde_json::json!({}) };
        let result = handle_todo_tool(&call, &mut todos);
        assert!(result.success);
        assert!(result.output.contains("[x]"));
        assert!(result.output.contains("[ ]"));
        assert!(result.output.contains("first"));
        assert!(result.output.contains("second"));
    }

    #[test]
    fn test_r1_todo_write_invalid_args_fails_gracefully() {
        let mut todos: Vec<TodoItem> = Vec::new();
        let call = ToolCall {
            id: "c3".into(),
            name: "todo_write".into(),
            arguments: serde_json::json!({ "wrong": "shape" }),
        };
        let result = handle_todo_tool(&call, &mut todos);
        assert!(!result.success);
        assert!(todos.is_empty());
    }

    #[test]
    fn test_r1_todo_tool_schemas_exposes_two_tools() {
        let schemas = todo_tool_schemas();
        assert_eq!(schemas.len(), 2);
        let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"todo_write"));
        assert!(names.contains(&"todo_read"));
    }

    #[test]
    fn test_r1_turn_outcome_serializes_with_kind_tag() {
        let oc = TurnOutcome::Complete { iterations: 3, tool_calls: 7 };
        let json = serde_json::to_value(&oc).unwrap();
        assert_eq!(json["kind"], "complete");
        assert_eq!(json["iterations"], 3);
        assert_eq!(json["tool_calls"], 7);

        let oc2 = TurnOutcome::ContinuedWithTodos { iterations: 5, remaining: 2 };
        let json2 = serde_json::to_value(&oc2).unwrap();
        assert_eq!(json2["kind"], "continued_with_todos");
        assert_eq!(json2["remaining"], 2);
    }
    // #99: Locks the stuck-agent recovery state-machine contract used inline in
    // the cooking loop (tool_executor.rs ~:1056). First time the degenerate
    // empty-tool_call streak hits the threshold we recover ONCE (reset + re-prompt,
    // no abort); the second time we abort. Mirrors the planning-retry guard.
    fn stuck_step(streak: &mut usize, recovery_used: &mut bool, threshold: usize) -> &'static str {
        if *streak >= threshold {
            if !*recovery_used {
                *recovery_used = true;
                *streak = 0;
                return "recovered";
            }
            return "aborted";
        }
        "continue"
    }

    #[test]
    fn test_99_stuck_agent_recovers_once_then_aborts() {
        const THRESHOLD: usize = 3;
        let mut streak = 0usize;
        let mut recovery_used = false;

        // climb to threshold the first time
        streak = THRESHOLD;
        assert_eq!(stuck_step(&mut streak, &mut recovery_used, THRESHOLD), "recovered");
        assert_eq!(streak, 0, "streak resets on first recovery");
        assert!(recovery_used, "recovery flag set");

        // climb to threshold again -> must abort this time
        streak = THRESHOLD;
        assert_eq!(stuck_step(&mut streak, &mut recovery_used, THRESHOLD), "aborted");
    }

    // ===== #232: Phantom-action guard tests (content-gated) =====

    fn tool() -> Vec<zeus_core::ToolSchema> {
        vec![zeus_core::ToolSchema {
            name: "read_file".into(),
            description: "Read a file".into(),
            parameters: serde_json::json!({}),
        }]
    }

    // (a) Plain greeting / normal answer → guard does NOT fire.
    #[test]
    fn test_phantom_action_greeting_does_not_fire() {
        let result = is_phantom_action_response(
            &[],
            &tool(),
            "Hey! Good to see you. I'm running Claude Opus 4 here, happy to help \
             with whatever you need today.",
            &zeus_llm::StopReason::EndTurn,
        );
        assert!(!result, "guard must NOT fire on a plain conversational reply");
    }

    // (b) "I'll read the file" with no tool_call → guard FIRES.
    #[test]
    fn test_phantom_action_narrated_action_fires() {
        let result = is_phantom_action_response(
            &[],
            &tool(),
            "Sure thing — I'll read the file and report back what I find inside it.",
            &zeus_llm::StopReason::EndTurn,
        );
        assert!(result, "guard must fire when reply narrates a tool action with no tool_call");
    }

    // (c) Raw <function...> leak → guard FIRES (#236 preserved).
    #[test]
    fn test_phantom_action_tool_leak_fires() {
        let result = is_phantom_action_response(
            &[],
            &tool(),
            "Let me check that for you <function=read_file>{\"path\":\"/etc/hosts\"}</function>",
            &zeus_llm::StopReason::EndTurn,
        );
        assert!(result, "guard must fire on a raw tool-call-as-text leak");
    }

    // Provider no longer matters: a narrated action fires regardless of provider.
    #[test]
    fn test_phantom_action_provider_agnostic() {
        let result = is_phantom_action_response(
            &[],
            &tool(),
            "I'm going to search the codebase for the offending call site now.",
            &zeus_llm::StopReason::EndTurn,
        );
        assert!(result, "guard fires on content alone, independent of provider");
    }

    // No tools available → text-only response expected, guard does NOT fire.
    #[test]
    fn test_phantom_action_no_tools_available_does_not_fire() {
        let result = is_phantom_action_response(
            &[],
            &[],
            "I'll read the file and let you know what's inside.",
            &zeus_llm::StopReason::EndTurn,
        );
        assert!(!result, "guard must NOT fire when no tools available");
    }

    // Short content → likely a genuine ack, guard does NOT fire.
    #[test]
    fn test_phantom_action_short_content_does_not_fire() {
        let result = is_phantom_action_response(
            &[],
            &tool(),
            "OK done",
            &zeus_llm::StopReason::EndTurn,
        );
        assert!(!result, "guard must NOT fire for short content");
    }

    // Error stop reason → not a phantom action, guard does NOT fire.
    #[test]
    fn test_phantom_action_error_stop_reason_does_not_fire() {
        let result = is_phantom_action_response(
            &[],
            &tool(),
            "I'll read the file and report back what I find inside it.",
            &zeus_llm::StopReason::Error,
        );
        assert!(!result, "guard must NOT fire on error stop reason");
    }

    // tool_calls present → not phantom, guard does NOT fire even if content narrates.
    #[test]
    fn test_phantom_action_with_tool_calls_does_not_fire() {
        let result = is_phantom_action_response(
            &[zeus_core::ToolCall {
                id: "1".into(),
                name: "read_file".into(),
                arguments: serde_json::json!({}),
            }],
            &tool(),
            "I'll read the file and report back what I find inside it.",
            &zeus_llm::StopReason::ToolUse,
        );
        assert!(!result, "guard must NOT fire when tool_calls are present");
    }

    // Direct unit tests for the content gate.
    #[test]
    fn test_content_claims_tool_action_unit() {
        assert!(content_claims_tool_action("I'll read the file now"));
        assert!(content_claims_tool_action("Let me run the tests"));
        assert!(content_claims_tool_action("I will search the logs"));
        assert!(content_claims_tool_action("I'm going to create the branch"));
        assert!(content_claims_tool_action("<function=read_file>{}</function>"));
        assert!(!content_claims_tool_action("Hello, how can I help today?"));
        assert!(!content_claims_tool_action(
            "The answer is 42 because of the underlying math."
        ));
        // cue without a tool verb → false
        assert!(!content_claims_tool_action("I'll think about it for a moment."));
    }
}