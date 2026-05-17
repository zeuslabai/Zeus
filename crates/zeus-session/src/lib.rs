//! Zeus Session - JSONL session storage
//!
//! Target: ~200 lines

pub mod analytics;
pub mod branching;
pub mod channel_router;
pub mod context_journal;
pub mod context_manager;
pub mod cost_estimator;
pub mod ephemeral;
pub mod export;
pub mod pruning;
pub mod reset;
pub mod search;
pub mod token_counter;
pub use analytics::{AnalyticsSummary, SessionAnalytics, SessionRecord, TimeBucket, ToolReport};
pub use export::{ExportFormat, ExportOptions, ExportResult, SessionExporter};

pub use branching::{BranchManager, BranchPoint};
pub use channel_router::{ChannelKey, ChannelSessionRouter, derive_session_id};
pub use context_journal::{ContextJournal, JournalEntry, JournalSummary};
pub use cost_estimator::{CostEstimator, ModelPricing, SessionCost};
pub use pruning::{
    PruneResult, RotationPolicy, SessionFileInfo, SessionPruner, start_pruning_task,
};
pub use reset::{ResetPolicy, SessionResetManager};
pub use search::{SearchQuery, SearchResult, SessionSearcher};
pub use token_counter::{
    MessageTokens, SessionTokenUsage, count_message_tokens, count_session_tokens, estimate_tokens,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tracing::debug;
use uuid::Uuid;
use zeus_core::{Error, Message, Provider, Result, Role, ToolResult};

pub use context_manager::{
    CompactionFlush, ContextManager, NO_REPLY_TOKEN, is_silent_reply, strip_silent_token,
    infer_pending_work,
};

// ============================================================================
// Session (~180 lines)
// ============================================================================

/// Lightweight session metadata — avoids loading full messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub id: String,
    pub created: DateTime<Utc>,
    pub message_count: usize,
    pub est_tokens: usize,
    pub last_preview: Option<String>,
    /// Optional human-readable label for persistent named sessions (S53-T8b).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A conversation session stored as JSONL
pub struct Session {
    pub id: String,
    pub created: DateTime<Utc>,
    pub messages: Vec<Message>,
    path: PathBuf,
    /// Optional label for persistent named sessions (S53-T8b).
    /// The label IS the agent — `get_or_create("pr-monitor")` always
    /// routes to the same session across restarts.
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionEntry {
    #[serde(rename = "type")]
    entry_type: String,
    #[serde(flatten)]
    data: serde_json::Value,
}

impl Session {
    /// Create a new session
    pub fn new(sessions_dir: impl AsRef<Path>) -> Self {
        let id = Uuid::new_v4().to_string();
        let path = sessions_dir.as_ref().join(format!("{}.jsonl", id));

        Self {
            id,
            created: Utc::now(),
            messages: Vec::new(),
            path,
            label: None,
        }
    }

    /// Create or resume a session with a deterministic ID.
    ///
    /// If a session file with the given `stable_id` exists, load it.
    /// Otherwise create a new session using that ID.  This ensures
    /// channel-bound agents (e.g. Discord default, thread agents)
    /// keep their conversation history across gateway restarts.
    pub async fn resume_or_create(sessions_dir: impl AsRef<Path>, stable_id: &str) -> Self {
        let path = sessions_dir.as_ref().join(format!("{}.jsonl", stable_id));
        if path.exists() {
            match Self::load(sessions_dir.as_ref(), stable_id).await {
                Ok(mut s) => {
                    // Repair orphaned tool_use entries — if the last assistant message
                    // has tool_calls but there's no following tool_result message,
                    // inject a synthetic error result to prevent Anthropic 400 errors.
                    let needs_repair = s.messages.last()
                        .map(|m| m.role == zeus_core::Role::Assistant && !m.tool_calls.is_empty())
                        .unwrap_or(false);
                    if needs_repair {
                        let tool_calls = s.messages.last().unwrap().tool_calls.clone();
                        let repair_results: Vec<zeus_core::ToolResult> = tool_calls.iter().map(|tc| {
                            zeus_core::ToolResult {
                                call_id: tc.id.clone(),
                                success: false,
                                output: "Tool execution was interrupted (session recovered)".to_string(),
                            }
                        }).collect();
                        let repair_msg = zeus_core::Message {
                            role: zeus_core::Role::Tool,
                            content: String::new(),
                            tool_calls: vec![],
                            tool_results: repair_results,
                            timestamp: chrono::Utc::now(),
                            attachments: vec![],
                            message_id: None,
                            parent_id: None,
                            thread_id: None,
                            direction: Default::default(), channel_source: None,
                            compaction_hint: Default::default(),
                        };
                        s.messages.push(repair_msg);
                        tracing::warn!(
                            "Session '{}': repaired {} orphaned tool_use entries",
                            stable_id, tool_calls.len()
                        );
                    }
                    tracing::info!(
                        "Resumed persistent session '{}' ({} messages)",
                        stable_id,
                        s.messages.len()
                    );
                    return s;
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to load session '{}', creating fresh: {}",
                        stable_id,
                        e
                    );
                }
            }
        }
        let sess = Self {
            id: stable_id.to_string(),
            created: Utc::now(),
            messages: Vec::new(),
            path,
            label: None,
        };
        tracing::info!("Created persistent session '{}'", stable_id);
        sess
    }

    /// Create or resume a labeled persistent session (S53-T8b).
    ///
    /// The label is the agent's identity — `get_or_create_labeled("pr-monitor")`
    /// always routes to the same session, surviving gateway restarts.
    /// Session ID is derived from the label with an `agent-` prefix.
    pub async fn get_or_create_labeled(
        sessions_dir: impl AsRef<Path>,
        label: &str,
    ) -> Self {
        let stable_id = format!("agent-{}", label);
        let mut session = Self::resume_or_create(sessions_dir, &stable_id).await;
        session.label = Some(label.to_string());
        session
    }

    /// Load an existing session by ID
    pub async fn load(sessions_dir: impl AsRef<Path>, id: &str) -> Result<Self> {
        let path = sessions_dir.as_ref().join(format!("{}.jsonl", id));

        if !path.exists() {
            return Err(Error::Session(format!("Session not found: {}", id)));
        }

        let content = fs::read_to_string(&path).await?;
        let mut messages = Vec::new();
        let mut created = Utc::now();

        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }

            let entry: SessionEntry = serde_json::from_str(line)
                .map_err(|e| Error::Session(format!("Failed to parse entry: {}", e)))?;

            match entry.entry_type.as_str() {
                "session_start" => {
                    if let Some(ts) = entry.data.get("created").and_then(|c| c.as_str())
                        && let Ok(dt) = ts.parse::<DateTime<Utc>>()
                    {
                        created = dt;
                    }
                }
                "message" => {
                    let msg: Message = serde_json::from_value(entry.data)
                        .map_err(|e| Error::Session(format!("Failed to parse message: {}", e)))?;
                    messages.push(msg);
                }
                _ => {}
            }
        }

        // Repair orphaned tool_use blocks on load to prevent API 400 errors
        repair_orphaned_tool_calls(&mut messages, None);

        // Derive label from session ID if it has the agent- prefix
        let label = id
            .strip_prefix("agent-")
            .map(|l| l.to_string());

        Ok(Self {
            id: id.to_string(),
            created,
            messages,
            path,
            label,
        })
    }

    /// List all sessions in the sessions directory
    pub async fn list(sessions_dir: impl AsRef<Path>) -> Result<Vec<(String, DateTime<Utc>)>> {
        let dir = sessions_dir.as_ref();
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        let mut entries = fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "jsonl")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                // Try to get creation time from file metadata or first line
                let created = if let Ok(content) = fs::read_to_string(&path).await {
                    if let Some(first_line) = content.lines().next() {
                        if let Ok(entry) = serde_json::from_str::<SessionEntry>(first_line) {
                            entry
                                .data
                                .get("created")
                                .and_then(|c| c.as_str())
                                .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                                .unwrap_or_else(Utc::now)
                        } else {
                            Utc::now()
                        }
                    } else {
                        Utc::now()
                    }
                } else {
                    Utc::now()
                };

                sessions.push((stem.to_string(), created));
            }
        }

        // Sort by creation time, newest first
        sessions.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(sessions)
    }

    /// Lightweight metadata for a session — avoids full Message deserialization.
    ///
    /// Reads the JSONL file line-by-line, counting message entries and extracting
    /// the last user/assistant content preview without parsing full Message structs.
    pub async fn quick_metadata(
        sessions_dir: impl AsRef<Path>,
        id: &str,
    ) -> Result<SessionMetadata> {
        let path = sessions_dir.as_ref().join(format!("{}.jsonl", id));
        let content = fs::read_to_string(&path)
            .await
            .map_err(|e| Error::Session(format!("Failed to read session {}: {}", id, e)))?;

        let mut created = Utc::now();
        let mut message_count: usize = 0;
        let mut last_preview: Option<String> = None;

        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            // Parse as generic JSON — avoids full Message struct deserialization
            let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            match val.get("type").and_then(|t| t.as_str()) {
                Some("session_start") => {
                    if let Some(ts) = val.get("created").and_then(|c| c.as_str())
                        && let Ok(dt) = ts.parse::<DateTime<Utc>>() {
                            created = dt;
                        }
                }
                Some("message") => {
                    message_count += 1;
                    let role = val.get("role").and_then(|r| r.as_str()).unwrap_or("");
                    if (role == "user" || role == "assistant")
                        && let Some(content) = val.get("content").and_then(|c| c.as_str()) {
                            let preview: String = content
                                .lines()
                                .next()
                                .unwrap_or("")
                                .chars()
                                .take(80)
                                .collect();
                            last_preview = Some(preview);
                        }
                }
                _ => {}
            }
        }

        // Rough token estimate: ~20 tokens per message on average (heuristic)
        let est_tokens = message_count * 20;

        let label = id
            .strip_prefix("agent-")
            .map(|l| l.to_string());

        Ok(SessionMetadata {
            id: id.to_string(),
            created,
            message_count,
            est_tokens,
            last_preview,
            label,
        })
    }

    /// List sessions with lightweight metadata, loaded concurrently.
    ///
    /// Returns up to `limit` sessions (newest first) with message count and
    /// last message preview — without deserializing full Message structs.
    pub async fn list_with_metadata(
        sessions_dir: impl AsRef<Path>,
        limit: usize,
    ) -> Result<Vec<SessionMetadata>> {
        let sessions = Self::list(&sessions_dir).await?;
        let dir = sessions_dir.as_ref().to_path_buf();

        // Load metadata concurrently for the requested page
        let futs: Vec<_> = sessions
            .into_iter()
            .take(limit)
            .map(|(id, _)| {
                let d = dir.clone();
                async move { Self::quick_metadata(&d, &id).await }
            })
            .collect();

        let results = futures::future::join_all(futs).await;
        Ok(results.into_iter().filter_map(|r| r.ok()).collect())
    }

    /// Load the most recent session (if any exist)
    pub async fn latest(sessions_dir: impl AsRef<Path>) -> Result<Option<Self>> {
        let sessions = Self::list(&sessions_dir).await?;
        if let Some((id, _)) = sessions.first() {
            Ok(Some(Self::load(&sessions_dir, id).await?))
        } else {
            Ok(None)
        }
    }

    /// Initialize the session file
    pub async fn init(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let entry = SessionEntry {
            entry_type: "session_start".to_string(),
            data: serde_json::json!({
                "id": self.id,
                "created": self.created.to_rfc3339(),
            }),
        };

        let line = serde_json::to_string(&entry)?;
        fs::write(&self.path, format!("{}\n", line)).await?;
        debug!("Created session: {}", self.id);

        Ok(())
    }

    /// Add a message to the session
    ///
    /// The message is stored unredacted in memory (so the active session
    /// remains usable), but secrets are redacted before writing to the
    /// JSONL file to prevent credential leakage in persisted history.
    pub async fn add(&mut self, message: Message) -> Result<()> {
        use zeus_core::sanitize::redact_secrets;

        // NOTE: repair_orphaned_tool_calls() is intentionally NOT called here.
        // Running repair on every add() is too aggressive — it catches in-flight
        // tool calls from concurrent channel cooks (multi-channel agents like ASSISTANT
        // share one session across Discord/Telegram/Signal/IRC). The repair would
        // inject synthetic "[session corrupted]" results for tool calls that are still
        // actively cooking on another channel.
        //
        // Repair runs at two safe points instead:
        // 1. Session::load() — catches orphans from crashed cooks (on disk)
        // 2. Agent loop pre-LLM call — catches orphans right before the API call

        let entry = SessionEntry {
            entry_type: "message".to_string(),
            data: serde_json::to_value(&message)?,
        };

        // Redact credentials from the serialized JSON before persisting
        let raw_line = serde_json::to_string(&entry)?;
        let safe_line = redact_secrets(&raw_line);

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;

        file.write_all(format!("{}\n", safe_line).as_bytes())
            .await?;

        // Flush + fsync so the entry is durable on disk before we return.
        // Without this, a crash between write_all and the OS flushing the
        // page cache can leave a partial JSONL line that corrupts the
        // session on next load.
        file.flush().await?;
        file.sync_all().await?;

        // Keep unredacted in memory for active session use
        self.messages.push(message);

        Ok(())
    }

    /// Export session to markdown with automatic secret redaction.
    ///
    /// Tool call arguments and results are passed through `redact_secrets()`
    /// to prevent accidental credential leakage in exports.
    pub async fn export_markdown(&self) -> String {
        use zeus_core::sanitize::redact_secrets;

        let mut md = String::new();

        md.push_str(&format!("# Session {}\n\n", self.id));
        md.push_str(&format!(
            "Created: {}\n\n",
            self.created.format("%Y-%m-%d %H:%M:%S")
        ));
        md.push_str("---\n\n");

        for msg in &self.messages {
            let role = match msg.role {
                zeus_core::Role::User => "**User**",
                zeus_core::Role::Assistant => "**Assistant**",
                zeus_core::Role::System => "**System**",
                zeus_core::Role::Tool => "**Tool**",
            };

            md.push_str(&format!("{}\n\n", role));

            if !msg.content.is_empty() {
                md.push_str(&redact_secrets(&msg.content));
                md.push_str("\n\n");
            }

            for tc in &msg.tool_calls {
                let args_str = serde_json::to_string_pretty(&tc.arguments).unwrap_or_default();
                md.push_str(&format!(
                    "*Tool: {}*\n```json\n{}\n```\n\n",
                    tc.name,
                    redact_secrets(&args_str)
                ));
            }

            for tr in &msg.tool_results {
                let status = if tr.success { "Success" } else { "Error" };
                md.push_str(&format!(
                    "*Result ({})*\n```\n{}\n```\n\n",
                    status,
                    redact_secrets(&tr.output)
                ));
            }

            md.push_str("---\n\n");
        }

        md
    }

    /// Get the number of messages
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Check if session is empty
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Get the session path
    pub fn path(&self) -> &Path {
        &self.path
    }
}

// ============================================================================
// Orphaned tool_use repair
// ============================================================================

// Providers that REJECT synthetic tool_result injection — must strip orphans instead.
fn must_strip_orphaned_tool_calls(provider: Option<&zeus_core::Provider>) -> bool {
    matches!(provider, Some(zeus_core::Provider::Moonshot) | Some(zeus_core::Provider::Minimax))
}

/// Repair orphaned tool_use blocks in a message list.
/// For Moonshot/MiniMax: STRIPS orphaned tool_calls instead of injecting synthetic results.
/// For all other providers: injects synthetic tool_results (standard approach).
///
/// Scans for assistant messages with tool_calls whose IDs don't have
/// matching tool_results **anywhere** in the remaining message list.
/// Only injects a synthetic tool_result for call_ids that have NO
/// match at all, preventing both false positives and cascade corruption.
///
/// Previous bug: only checked `i + 1` for matching results. If a tool
/// result existed further ahead (after a system message, or in a later
/// tool message), the repair would inject a synthetic result. On next
/// load, the real result became an orphan (because the synthetic one
/// already matched the call_id), creating a cascade of corruption.
///
/// Called at Session::load() time so corrupted sessions are
/// repaired on gateway restart, not just after context truncation.
pub fn repair_orphaned_tool_calls(messages: &mut Vec<Message>, provider: Option<&zeus_core::Provider>) {
    // Phase 1: Compute per-turn-segment satisfied call_ids via turn_boundary helper.
    //
    // CATCH #57-ii: Prior implementation used a single global HashSet across the
    // entire message list. This masked kimi-style tool_call_id reuse across turns
    // (e.g., `shell:0` reused turn N+2 after being satisfied turn N) — the orphan
    // at turn N+2 was incorrectly treated as satisfied by turn N's tool_result,
    // never repaired, and surfaced as an Anthropic API 400 on next call.
    //
    // Fix: per-segment HashSet (one per User→...→User turn boundary). An orphan
    // is only "satisfied" if a tool_result with matching call_id exists in the
    // SAME segment.
    let mut segment_satisfied = zeus_core::turn_boundary::segment_satisfied_call_ids(messages);

    // Phase 2: For each assistant message with tool_calls, find call_ids
    // that have NO matching result in their own turn-segment. Only those are
    // true orphans.
    let mut inserts: Vec<(usize, Message)> = Vec::new();
    let mut extends: Vec<(usize, Vec<ToolResult>)> = Vec::new();
    let strip_orphans = must_strip_orphaned_tool_calls(provider);

    for i in 0..messages.len() {
        if messages[i].role == Role::Assistant && !messages[i].tool_calls.is_empty() {
            let seg = zeus_core::turn_boundary::turn_segment_for_index(messages, i);
            let orphans: Vec<String> = messages[i]
                .tool_calls
                .iter()
                .map(|tc| tc.id.clone())
                .filter(|id| {
                    segment_satisfied
                        .get(seg)
                        .map(|s| !s.contains(id))
                        .unwrap_or(true)
                })
                .collect();

            if orphans.is_empty() {
                continue;
            }

            // Moonshot/MiniMax: strip orphaned tool_calls instead of injecting
            // synthetic tool_results (the providers explicitly reject synthetic
            // injection — see must_strip_orphaned_tool_calls). Mark stripped IDs
            // as satisfied in the turn-segment so subsequent repair passes do
            // not re-process them.
            if strip_orphans {
                let orphan_set: std::collections::HashSet<&String> = orphans.iter().collect();
                messages[i].tool_calls.retain(|tc| !orphan_set.contains(&tc.id));
                let seg = zeus_core::turn_boundary::turn_segment_for_index(messages, i);
                if let Some(seg_set) = segment_satisfied.get_mut(seg) {
                    for id in &orphans {
                        seg_set.insert(id.clone());
                    }
                }
                debug!(
                    "Session repair (Moonshot/MiniMax): stripped {} orphaned tool_call ID(s)",
                    orphans.len()
                );
                continue;
            }

            debug!(
                "Session repair: {} orphaned tool_use ID(s) — injecting synthetic tool_results",
                orphans.len()
            );

            let synthetic_results: Vec<ToolResult> = orphans
                .iter()
                .map(|id| ToolResult {
                    call_id: id.clone(),
                    success: false,
                    output: "[session corrupted — tool result unavailable]".to_string(),
                })
                .collect();

            // Mark these as satisfied in the current turn-segment so we don't
            // inject duplicates for the same call_id within the same turn.
            let seg = zeus_core::turn_boundary::turn_segment_for_index(messages, i);
            if let Some(seg_set) = segment_satisfied.get_mut(seg) {
                for id in &orphans {
                    seg_set.insert(id.clone());
                }
            }

            let next_is_tool = i + 1 < messages.len() && messages[i + 1].role == Role::Tool;
            if next_is_tool {
                extends.push((i + 1, synthetic_results));
            } else {
                let repair_msg = Message {
                    role: Role::Tool,
                    content: String::new(),
                    tool_calls: vec![],
                    tool_results: synthetic_results,
                    timestamp: chrono::Utc::now(),
                    attachments: vec![],
                    message_id: None,
                    parent_id: None,
                    thread_id: None,
                    direction: Default::default(),
                    channel_source: None,
                    compaction_hint: Default::default(),
                };
                inserts.push((i + 1, repair_msg));
            }
        }
    }

    // Phase 3: Apply repairs — process in reverse order so indices stay valid.
    // Extends first (they don't shift indices), then inserts.
    for (idx, results) in extends {
        messages[idx].tool_results.extend(results);
    }
    for (idx, msg) in inserts.into_iter().rev() {
        messages.insert(idx, msg);
    }

    // Remove empty assistant messages — some providers (Kimi) reject them.
    // An empty assistant message can result from a failed/timed-out LLM call
    // that got saved to the session before a response arrived.
    messages.retain(|m| {
        !(m.role == Role::Assistant
            && m.content.trim().is_empty()
            && m.tool_calls.is_empty())
    });
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_session_create_and_load() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");

        // Create session
        let mut session = Session::new(tmp.path());
        session
            .init()
            .await
            .expect("async operation should succeed");

        // Add message
        session
            .add(Message::user("Hello"))
            .await
            .expect("async operation should succeed");
        session
            .add(Message::assistant("Hi there!"))
            .await
            .expect("async operation should succeed");

        assert_eq!(session.len(), 2);

        // Load session
        let loaded = Session::load(tmp.path(), &session.id)
            .await
            .expect("async operation should succeed");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.messages[0].content, "Hello");
    }

    #[tokio::test]
    async fn test_session_list() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");

        // Create multiple sessions
        let s1 = Session::new(tmp.path());
        s1.init().await.expect("async operation should succeed");

        let s2 = Session::new(tmp.path());
        s2.init().await.expect("async operation should succeed");

        let list = Session::list(tmp.path())
            .await
            .expect("async operation should succeed");
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn test_export_markdown() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let mut session = Session::new(tmp.path());
        session
            .init()
            .await
            .expect("async operation should succeed");

        session
            .add(Message::user("Test message"))
            .await
            .expect("Failed to add test message to session");

        let md = session.export_markdown().await;
        assert!(md.contains("Test message"));
        assert!(md.contains("**User**"));
    }

    #[tokio::test]
    async fn test_quick_metadata() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let mut session = Session::new(tmp.path());
        session
            .init()
            .await
            .expect("async operation should succeed");

        session
            .add(Message::user("First question"))
            .await
            .expect("add user message");
        session
            .add(Message::assistant("Here is the answer"))
            .await
            .expect("add assistant message");
        session
            .add(Message::user("Follow-up"))
            .await
            .expect("add second user message");

        let meta = Session::quick_metadata(tmp.path(), &session.id)
            .await
            .expect("quick_metadata should succeed");

        assert_eq!(meta.id, session.id);
        assert_eq!(meta.message_count, 3);
        assert_eq!(meta.est_tokens, 60); // 3 * 20
        assert_eq!(meta.last_preview.as_deref(), Some("Follow-up"));
    }

    #[tokio::test]
    async fn test_list_with_metadata() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");

        // Create two sessions with messages
        let mut s1 = Session::new(tmp.path());
        s1.init().await.expect("init s1");
        s1.add(Message::user("Hello from s1"))
            .await
            .expect("add to s1");

        let mut s2 = Session::new(tmp.path());
        s2.init().await.expect("init s2");
        s2.add(Message::user("Hello from s2"))
            .await
            .expect("add to s2");
        s2.add(Message::assistant("Reply in s2"))
            .await
            .expect("add reply to s2");

        let metas = Session::list_with_metadata(tmp.path(), 10)
            .await
            .expect("list_with_metadata should succeed");

        assert_eq!(metas.len(), 2);
        // Both sessions should have correct message counts
        let counts: Vec<usize> = metas.iter().map(|m| m.message_count).collect();
        assert!(counts.contains(&1)); // s1: 1 message
        assert!(counts.contains(&2)); // s2: 2 messages
    }

    // ── repair_orphaned_tool_calls tests (S46-T1) ────────────────────────

    #[test]
    fn test_repair_orphaned_no_tool_msg() {
        // Assistant with tool_calls but no following tool message
        let mut messages = vec![
            Message {
                role: Role::Assistant,
                content: "calling tool".into(),
                tool_calls: vec![zeus_core::ToolCall {
                    id: "tc_orphan".into(),
                    name: "shell".into(),
                    arguments: Default::default(),
                }],
                tool_results: vec![],
                timestamp: Utc::now(),
                attachments: vec![],
                message_id: None,
                parent_id: None,
                thread_id: None,
                direction: Default::default(), channel_source: None, compaction_hint: Default::default(),
            },
            Message::user("next"),
        ];
        repair_orphaned_tool_calls(&mut messages, None);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].role, Role::Tool);
        assert_eq!(messages[1].tool_results[0].call_id, "tc_orphan");
        assert!(!messages[1].tool_results[0].success);
    }

    #[test]
    fn test_repair_orphaned_partial_results() {
        // 2 tool_calls but only 1 matching result
        let mut messages = vec![
            Message {
                role: Role::Assistant,
                content: "calling".into(),
                tool_calls: vec![
                    zeus_core::ToolCall { id: "tc_a".into(), name: "shell".into(), arguments: Default::default() },
                    zeus_core::ToolCall { id: "tc_b".into(), name: "shell".into(), arguments: Default::default() },
                ],
                tool_results: vec![],
                timestamp: Utc::now(),
                attachments: vec![], message_id: None, parent_id: None, thread_id: None, direction: Default::default(), channel_source: None, compaction_hint: Default::default(),
            },
            Message {
                role: Role::Tool,
                content: String::new(),
                tool_calls: vec![],
                tool_results: vec![ToolResult { call_id: "tc_a".into(), success: true, output: "ok".into() }],
                timestamp: Utc::now(),
                attachments: vec![], message_id: None, parent_id: None, thread_id: None, direction: Default::default(), channel_source: None, compaction_hint: Default::default(),
            },
        ];
        repair_orphaned_tool_calls(&mut messages, None);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].tool_results.len(), 2);
        let ids: Vec<&str> = messages[1].tool_results.iter().map(|r| r.call_id.as_str()).collect();
        assert!(ids.contains(&"tc_a"));
        assert!(ids.contains(&"tc_b"));
    }

    #[test]
    fn test_repair_no_orphans() {
        let mut messages = vec![
            Message {
                role: Role::Assistant,
                content: "calling".into(),
                tool_calls: vec![zeus_core::ToolCall { id: "tc_ok".into(), name: "shell".into(), arguments: Default::default() }],
                tool_results: vec![],
                timestamp: Utc::now(),
                attachments: vec![], message_id: None, parent_id: None, thread_id: None, direction: Default::default(), channel_source: None, compaction_hint: Default::default(),
            },
            Message {
                role: Role::Tool,
                content: String::new(),
                tool_calls: vec![],
                tool_results: vec![ToolResult { call_id: "tc_ok".into(), success: true, output: "done".into() }],
                timestamp: Utc::now(),
                attachments: vec![], message_id: None, parent_id: None, thread_id: None, direction: Default::default(), channel_source: None, compaction_hint: Default::default(),
            },
        ];
        let len_before = messages.len();
        repair_orphaned_tool_calls(&mut messages, None);
        assert_eq!(messages.len(), len_before);
    }

    #[test]
    fn test_repair_no_cascade_when_result_exists_later() {
        // Regression test: tool result exists but NOT immediately after the assistant message.
        // Old code only checked i+1, so it would inject a synthetic result, causing cascade
        // corruption on next load (real result becomes orphan because synthetic already matched).
        let mut messages = vec![
            Message {
                role: Role::Assistant,
                content: "calling tool".into(),
                tool_calls: vec![zeus_core::ToolCall {
                    id: "tc_later".into(),
                    name: "shell".into(),
                    arguments: Default::default(),
                }],
                tool_results: vec![],
                timestamp: Utc::now(),
                attachments: vec![],
                message_id: None, parent_id: None, thread_id: None,
                direction: Default::default(), channel_source: None, compaction_hint: Default::default(),
            },
            // System message between tool_call and tool_result — this is what broke the old code
            Message {
                role: Role::System,
                content: "context update".into(),
                tool_calls: vec![],
                tool_results: vec![],
                timestamp: Utc::now(),
                attachments: vec![],
                message_id: None, parent_id: None, thread_id: None,
                direction: Default::default(), channel_source: None, compaction_hint: Default::default(),
            },
            // The actual tool result — exists but not at i+1
            Message {
                role: Role::Tool,
                content: String::new(),
                tool_calls: vec![],
                tool_results: vec![ToolResult { call_id: "tc_later".into(), success: true, output: "real result".into() }],
                timestamp: Utc::now(),
                attachments: vec![],
                message_id: None, parent_id: None, thread_id: None,
                direction: Default::default(), channel_source: None, compaction_hint: Default::default(),
            },
        ];

        repair_orphaned_tool_calls(&mut messages, None);

        // Should NOT inject any synthetic results — the real result exists
        assert_eq!(messages.len(), 3, "no synthetic messages should be injected when result exists later");
        assert_eq!(messages[2].tool_results.len(), 1, "original tool result should be untouched");
        assert_eq!(messages[2].tool_results[0].output, "real result");
        assert!(messages[2].tool_results[0].success);
    }

    #[test]
    fn test_repair_idempotent_no_cascade() {
        // Running repair twice should produce the same result — no cascade.
        let mut messages = vec![
            Message {
                role: Role::Assistant,
                content: "calling".into(),
                tool_calls: vec![zeus_core::ToolCall {
                    id: "tc_idempotent".into(),
                    name: "shell".into(),
                    arguments: Default::default(),
                }],
                tool_results: vec![],
                timestamp: Utc::now(),
                attachments: vec![],
                message_id: None, parent_id: None, thread_id: None,
                direction: Default::default(), channel_source: None, compaction_hint: Default::default(),
            },
            Message::user("next"),
        ];

        repair_orphaned_tool_calls(&mut messages, None);
        let len_after_first = messages.len();

        // Run again — should not add more synthetic results
        repair_orphaned_tool_calls(&mut messages, None);
        assert_eq!(messages.len(), len_after_first, "second repair should not add more messages");
    }

    // ── #61 strip_orphans dead-gate fix tests (Moonshot/MiniMax) ─────────

    #[test]
    fn test_repair_orphaned_moonshot_strips_no_synthetic_injection() {
        // Moonshot: assistant has orphan tool_call → MUST strip from tool_calls,
        // MUST NOT inject synthetic Tool message.
        let mut messages = vec![
            Message {
                role: Role::Assistant,
                content: "calling tool".into(),
                tool_calls: vec![zeus_core::ToolCall {
                    id: "tc_orphan_moon".into(),
                    name: "shell".into(),
                    arguments: Default::default(),
                }],
                tool_results: vec![],
                timestamp: Utc::now(),
                attachments: vec![],
                message_id: None,
                parent_id: None,
                thread_id: None,
                direction: Default::default(),
                channel_source: None,
                compaction_hint: Default::default(),
            },
            Message::user("next"),
        ];
        let len_before = messages.len();
        repair_orphaned_tool_calls(&mut messages, Some(&zeus_core::Provider::Moonshot));

        // No synthetic Tool message inserted.
        assert_eq!(
            messages.len(),
            len_before,
            "Moonshot path must NOT insert synthetic tool_result messages"
        );
        // Orphan stripped from assistant tool_calls.
        assert!(
            messages[0].tool_calls.is_empty(),
            "orphaned tool_call must be stripped from assistant message"
        );
        // No Tool-role messages present anywhere.
        assert!(
            messages.iter().all(|m| m.role != Role::Tool),
            "no synthetic Tool messages should exist on Moonshot path"
        );
    }

    #[test]
    fn test_repair_orphaned_minimax_strips_partial_orphans() {
        // MiniMax: assistant has 2 tool_calls, only 1 has a real result.
        // Orphan must be stripped; satisfied call_id must be preserved.
        let mut messages = vec![
            Message {
                role: Role::Assistant,
                content: "calling tools".into(),
                tool_calls: vec![
                    zeus_core::ToolCall {
                        id: "tc_satisfied".into(),
                        name: "shell".into(),
                        arguments: Default::default(),
                    },
                    zeus_core::ToolCall {
                        id: "tc_orphan_mm".into(),
                        name: "shell".into(),
                        arguments: Default::default(),
                    },
                ],
                tool_results: vec![],
                timestamp: Utc::now(),
                attachments: vec![],
                message_id: None,
                parent_id: None,
                thread_id: None,
                direction: Default::default(),
                channel_source: None,
                compaction_hint: Default::default(),
            },
            Message {
                role: Role::Tool,
                content: String::new(),
                tool_calls: vec![],
                tool_results: vec![ToolResult {
                    call_id: "tc_satisfied".into(),
                    success: true,
                    output: "ok".into(),
                }],
                timestamp: Utc::now(),
                attachments: vec![],
                message_id: None,
                parent_id: None,
                thread_id: None,
                direction: Default::default(),
                channel_source: None,
                compaction_hint: Default::default(),
            },
        ];
        let len_before = messages.len();
        repair_orphaned_tool_calls(&mut messages, Some(&zeus_core::Provider::Minimax));

        // No synthetic insertion.
        assert_eq!(messages.len(), len_before, "MiniMax path must NOT insert synthetic messages");
        // Satisfied call_id preserved, orphan stripped.
        let remaining_ids: Vec<&str> = messages[0]
            .tool_calls
            .iter()
            .map(|tc| tc.id.as_str())
            .collect();
        assert_eq!(remaining_ids, vec!["tc_satisfied"], "only satisfied tool_call should remain");
        // No "[session corrupted ...]" synthetic results anywhere.
        for msg in &messages {
            for tr in &msg.tool_results {
                assert_ne!(tr.call_id, "tc_orphan_mm", "MiniMax must not get synthetic result for orphan");
            }
        }
    }

    #[tokio::test]
    async fn test_session_load_repairs_orphaned_tool_calls() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let mut session = Session::new(tmp.path());
        session.init().await.expect("init");

        // Add an assistant message with a tool call
        let mut assistant = Message::assistant("calling tool");
        assistant.tool_calls.push(zeus_core::ToolCall {
            id: "tc_load_test".into(),
            name: "shell".into(),
            arguments: Default::default(),
        });
        session.add(assistant).await.expect("add assistant");

        // Add a user message WITHOUT a tool result — simulating corruption
        session.add(Message::user("next question")).await.expect("add user");

        // Load the session — repair should inject a synthetic tool result
        let loaded = Session::load(tmp.path(), &session.id).await.expect("load");

        // Should now have 3 messages: assistant, synthetic tool, user
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded.messages[1].role, Role::Tool);
        assert_eq!(loaded.messages[1].tool_results[0].call_id, "tc_load_test");
        assert!(!loaded.messages[1].tool_results[0].success);
    }
}
