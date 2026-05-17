//! Cooking Loop Checkpointing — persist cooking state for crash-resume
//!
//! After each tool execution in the cooking loop, the checkpoint captures:
//! - Session ID (unique per cooking run)
//! - Current iteration index
//! - Accumulated messages (serialized JSON)
//! - Tool results so far
//! - Original message and config
//!
//! On restart, `find_interrupted_sessions()` locates cooking runs that
//! didn't complete, enabling resume-from-checkpoint.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use zeus_core::Message;

use crate::tool_executor::ToolCallRecord;

// ============================================================================
// Checkpoint Data
// ============================================================================

/// Serializable snapshot of a cooking loop's mid-execution state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookingCheckpoint {
    /// Unique session ID for this cooking run
    pub session_id: String,
    /// Original user message that started the cook
    pub original_message: String,
    /// Current iteration (1-based)
    pub iteration: usize,
    /// Total tool calls executed so far
    pub tool_call_count: usize,
    /// Conversation messages up to this point (serialized)
    pub messages: Vec<Message>,
    /// Tool call records accumulated
    pub tool_records: Vec<ToolCallRecord>,
    /// Whether the cooking run completed successfully
    pub completed: bool,
    /// Last checkpoint time
    pub updated_at: DateTime<Utc>,
    /// When the cooking run started
    pub started_at: DateTime<Utc>,
    /// System prompt used (for resume)
    pub system_prompt: String,
    /// R1: Agent-managed todo list (TodoWrite/TodoRead). Persisted across crash-resume.
    #[serde(default)]
    pub todos: Vec<crate::tool_executor::TodoItem>,
}

/// Summary of an interrupted session (for listing/resume decisions).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptedSession {
    pub session_id: String,
    pub original_message: String,
    pub iteration: usize,
    pub tool_call_count: usize,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Checkpoint Store
// ============================================================================

const COOKING_MIGRATIONS: &[&str] = &[
    // v1: initial schema
    "CREATE TABLE IF NOT EXISTS cooking_checkpoints (
                session_id    TEXT PRIMARY KEY,
                original_msg  TEXT NOT NULL,
                iteration     INTEGER NOT NULL DEFAULT 0,
                tool_calls    INTEGER NOT NULL DEFAULT 0,
                messages_json TEXT NOT NULL DEFAULT '[]',
                records_json  TEXT NOT NULL DEFAULT '[]',
                system_prompt TEXT NOT NULL DEFAULT '',
                completed     INTEGER NOT NULL DEFAULT 0,
                started_at    TEXT NOT NULL,
                updated_at    TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_cooking_incomplete
                ON cooking_checkpoints(completed) WHERE completed = 0;",
    // v2: R1 — agent-managed todo list persisted across crash-resume.
    "ALTER TABLE cooking_checkpoints ADD COLUMN todos_json TEXT NOT NULL DEFAULT '[]';",
];

/// SQLite-backed checkpoint persistence for cooking loops.
pub struct CookingCheckpointStore {
    db: Arc<Mutex<Connection>>,
}

impl CookingCheckpointStore {
    /// Open (or create) a checkpoint store at the given path.
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("SQLite open: {}", e))?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(|e| format!("SQLite pragma: {}", e))?;

        crate::db::run_migrations(&conn, COOKING_MIGRATIONS)
            .map_err(|e| format!("Cooking schema migration failed: {e}"))?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory store (for testing).
    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("SQLite: {}", e))?;
        crate::db::run_migrations(&conn, COOKING_MIGRATIONS)
            .map_err(|e| format!("Cooking schema migration failed: {e}"))?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Start a new cooking session.
    pub async fn start_session(
        &self,
        session_id: &str,
        original_message: &str,
        system_prompt: &str,
    ) {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        if let Err(e) = db.execute(
            "INSERT OR REPLACE INTO cooking_checkpoints
             (session_id, original_msg, iteration, tool_calls, messages_json, records_json, system_prompt, completed, started_at, updated_at, todos_json)
             VALUES (?1, ?2, 0, 0, '[]', '[]', ?3, 0, ?4, ?4, '[]')",
            params![session_id, original_message, system_prompt, now],
        ) {
            warn!("Failed to start cooking checkpoint: {}", e);
        }
    }

    /// Save a checkpoint after a tool execution.
    pub async fn save_checkpoint(&self, checkpoint: &CookingCheckpoint) {
        let db = self.db.lock().await;
        let messages_json = serde_json::to_string(&checkpoint.messages).unwrap_or_default();
        let records_json = serde_json::to_string(&checkpoint.tool_records).unwrap_or_default();
        let todos_json = serde_json::to_string(&checkpoint.todos).unwrap_or_else(|_| "[]".to_string());
        let now = Utc::now().to_rfc3339();

        if let Err(e) = db.execute(
            "UPDATE cooking_checkpoints SET
                iteration = ?1,
                tool_calls = ?2,
                messages_json = ?3,
                records_json = ?4,
                updated_at = ?5,
                todos_json = ?7
             WHERE session_id = ?6",
            params![
                checkpoint.iteration,
                checkpoint.tool_call_count,
                messages_json,
                records_json,
                now,
                checkpoint.session_id,
                todos_json,
            ],
        ) {
            warn!(
                session = %checkpoint.session_id,
                "Failed to save cooking checkpoint: {}", e
            );
        } else {
            debug!(
                session = %checkpoint.session_id,
                iteration = checkpoint.iteration,
                tools = checkpoint.tool_call_count,
                "Cooking checkpoint saved"
            );
        }
    }

    /// Mark a cooking session as completed.
    pub async fn mark_completed(&self, session_id: &str) {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        let _ = db.execute(
            "UPDATE cooking_checkpoints SET completed = 1, updated_at = ?1 WHERE session_id = ?2",
            params![now, session_id],
        );
        debug!(session = session_id, "Cooking session marked complete");
    }

    /// Mark a session as interrupted (not completed naturally, e.g. cap hit).
    /// Heartbeat uses `find_interrupted_sessions()` to detect and auto-resume these.
    pub async fn mark_interrupted(&self, session_id: &str) {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        let _ = db.execute(
            "UPDATE cooking_checkpoints SET completed = 0, updated_at = ?1 WHERE session_id = ?2",
            params![now, session_id],
        );
        debug!(session = session_id, "Cooking session marked interrupted (cap hit)");
    }

    /// Find all interrupted (incomplete) sessions.
    pub async fn find_interrupted_sessions(&self) -> Vec<InterruptedSession> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT session_id, original_msg, iteration, tool_calls, started_at, updated_at
             FROM cooking_checkpoints WHERE completed = 0 ORDER BY updated_at DESC",
        ) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to query interrupted sessions: {}", e);
                return Vec::new();
            }
        };

        let rows = stmt
            .query_map([], |row| {
                Ok(InterruptedSession {
                    session_id: row.get(0)?,
                    original_message: row.get(1)?,
                    iteration: row.get::<_, i64>(2)? as usize,
                    tool_call_count: row.get::<_, i64>(3)? as usize,
                    started_at: row
                        .get::<_, String>(4)
                        .ok()
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(Utc::now),
                    updated_at: row
                        .get::<_, String>(5)
                        .ok()
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(Utc::now),
                })
            })
            .ok();

        rows.map(|r| r.filter_map(|v| v.ok()).collect())
            .unwrap_or_default()
    }

    /// Load a full checkpoint for resuming.
    pub async fn load_checkpoint(&self, session_id: &str) -> Option<CookingCheckpoint> {
        let db = self.db.lock().await;
        let mut stmt = db
            .prepare(
                "SELECT session_id, original_msg, iteration, tool_calls, messages_json,
                        records_json, system_prompt, completed, started_at, updated_at, todos_json
                 FROM cooking_checkpoints WHERE session_id = ?1",
            )
            .ok()?;

        stmt.query_row(params![session_id], |row| {
            let messages_json: String = row.get(4)?;
            let records_json: String = row.get(5)?;
            let todos_json: String = row.get::<_, String>(10).unwrap_or_else(|_| "[]".to_string());

            Ok(CookingCheckpoint {
                session_id: row.get(0)?,
                original_message: row.get(1)?,
                iteration: row.get::<_, i64>(2)? as usize,
                tool_call_count: row.get::<_, i64>(3)? as usize,
                messages: serde_json::from_str(&messages_json).unwrap_or_default(),
                tool_records: serde_json::from_str(&records_json).unwrap_or_default(),
                // R1: agent-managed todo list — restored from todos_json column (v2).
                todos: serde_json::from_str(&todos_json).unwrap_or_default(),
                system_prompt: row.get(6)?,
                completed: row.get::<_, i64>(7)? != 0,
                started_at: row
                    .get::<_, String>(8)
                    .ok()
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now),
                updated_at: row
                    .get::<_, String>(9)
                    .ok()
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now),
            })
        })
        .ok()
    }

    /// Delete a checkpoint (after successful resume or cleanup).
    pub async fn delete_session(&self, session_id: &str) {
        let db = self.db.lock().await;
        let _ = db.execute(
            "DELETE FROM cooking_checkpoints WHERE session_id = ?1",
            params![session_id],
        );
    }

    /// Delete all completed sessions older than the given duration.
    pub async fn cleanup_old_sessions(&self, max_age: std::time::Duration) {
        let db = self.db.lock().await;
        let cutoff = Utc::now() - chrono::Duration::from_std(max_age).unwrap_or_default();
        let cutoff_str = cutoff.to_rfc3339();
        let deleted = db
            .execute(
                "DELETE FROM cooking_checkpoints WHERE completed = 1 AND updated_at < ?1",
                params![cutoff_str],
            )
            .unwrap_or(0);
        if deleted > 0 {
            info!("Cleaned up {} old cooking checkpoints", deleted);
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_start_and_complete_session() {
        let store = CookingCheckpointStore::in_memory().unwrap();

        store
            .start_session("sess-1", "help me code", "You are Zeus")
            .await;

        let interrupted = store.find_interrupted_sessions().await;
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].session_id, "sess-1");
        assert_eq!(interrupted[0].original_message, "help me code");

        store.mark_completed("sess-1").await;

        let interrupted = store.find_interrupted_sessions().await;
        assert!(interrupted.is_empty());
    }

    #[tokio::test]
    async fn test_save_and_load_checkpoint() {
        let store = CookingCheckpointStore::in_memory().unwrap();

        store
            .start_session("sess-2", "build a feature", "system prompt")
            .await;

        let checkpoint = CookingCheckpoint {
            session_id: "sess-2".into(),
            original_message: "build a feature".into(),
            iteration: 3,
            tool_call_count: 5,
            messages: vec![
                Message::user("build a feature"),
                Message::assistant("I'll start by reading the code"),
            ],
            tool_records: vec![],
            completed: false,
            updated_at: Utc::now(),
            started_at: Utc::now(),
            system_prompt: "system prompt".into(),
            todos: vec![],
        };

        store.save_checkpoint(&checkpoint).await;

        let loaded = store.load_checkpoint("sess-2").await.unwrap();
        assert_eq!(loaded.iteration, 3);
        assert_eq!(loaded.tool_call_count, 5);
        assert_eq!(loaded.messages.len(), 2);
        assert!(!loaded.completed);
    }

    #[tokio::test]
    async fn test_find_interrupted_only_incomplete() {
        let store = CookingCheckpointStore::in_memory().unwrap();

        store.start_session("complete", "done task", "sp").await;
        store.mark_completed("complete").await;

        store.start_session("incomplete", "in progress", "sp").await;

        let interrupted = store.find_interrupted_sessions().await;
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].session_id, "incomplete");
    }

    #[tokio::test]
    async fn test_delete_session() {
        let store = CookingCheckpointStore::in_memory().unwrap();

        store.start_session("sess-del", "test", "sp").await;
        assert!(store.load_checkpoint("sess-del").await.is_some());

        store.delete_session("sess-del").await;
        assert!(store.load_checkpoint("sess-del").await.is_none());
    }

    #[tokio::test]
    async fn test_cleanup_old_completed() {
        let store = CookingCheckpointStore::in_memory().unwrap();

        store.start_session("old-sess", "old task", "sp").await;
        store.mark_completed("old-sess").await;

        // Force old timestamp
        {
            let db = store.db.lock().await;
            let old = (Utc::now() - chrono::Duration::hours(48)).to_rfc3339();
            db.execute(
                "UPDATE cooking_checkpoints SET updated_at = ?1 WHERE session_id = 'old-sess'",
                params![old],
            )
            .unwrap();
        }

        store.start_session("new-sess", "new task", "sp").await;
        store.mark_completed("new-sess").await;

        // Cleanup sessions older than 24h
        store
            .cleanup_old_sessions(std::time::Duration::from_secs(86400))
            .await;

        // Old session should be gone, new should remain
        assert!(store.load_checkpoint("old-sess").await.is_none());
        assert!(store.load_checkpoint("new-sess").await.is_some());
    }

    #[tokio::test]
    async fn test_multiple_checkpoints_update() {
        let store = CookingCheckpointStore::in_memory().unwrap();

        store.start_session("sess-multi", "task", "sp").await;

        for i in 1..=5 {
            let cp = CookingCheckpoint {
                session_id: "sess-multi".into(),
                original_message: "task".into(),
                iteration: i,
                tool_call_count: i * 2,
                messages: vec![Message::user("task")],
                tool_records: vec![],
                completed: false,
                updated_at: Utc::now(),
                started_at: Utc::now(),
                system_prompt: "sp".into(),
                todos: vec![],
            };
            store.save_checkpoint(&cp).await;
        }

        let loaded = store.load_checkpoint("sess-multi").await.unwrap();
        assert_eq!(loaded.iteration, 5);
        assert_eq!(loaded.tool_call_count, 10);
    }
}
