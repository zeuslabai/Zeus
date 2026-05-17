//! SQLite-backed persistence for Agent Studio (Phase 5 — Super Cursor).
//!
//! Mirrors the `PantheonStore` / `DeployStore` pattern:
//! `Arc<Mutex<Connection>>` with WAL mode.
//!
//! Tables:
//!  - `studio_sessions`  — active autopilot sessions with goal, status, plan link
//!  - `studio_actions`    — individual UI puppet commands + results
//!  - `studio_artifacts`  — outputs produced by sessions (URLs, files, screenshots)

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;

// ============================================================================
// StudioStore
// ============================================================================

const STUDIO_MIGRATIONS: &[&str] = &[
    // v1: initial schema
    "CREATE TABLE IF NOT EXISTS studio_sessions (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL DEFAULT 'default',
                goal TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'idle',
                plan_id TEXT,
                room_id TEXT,
                agent_id TEXT,
                session_id TEXT,
                total_actions INTEGER NOT NULL DEFAULT 0,
                completed_actions INTEGER NOT NULL DEFAULT 0,
                failed_actions INTEGER NOT NULL DEFAULT 0,
                error_message TEXT NOT NULL DEFAULT '',
                metadata TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                completed_at TEXT
            );
            CREATE TABLE IF NOT EXISTS studio_actions (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                action_type TEXT NOT NULL,
                target TEXT NOT NULL DEFAULT '',
                value TEXT NOT NULL DEFAULT '',
                description TEXT NOT NULL DEFAULT '',
                delay_ms INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'pending',
                error_message TEXT NOT NULL DEFAULT '',
                sequence_num INTEGER NOT NULL DEFAULT 0,
                elapsed_ms INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                executed_at TEXT
            );
            CREATE TABLE IF NOT EXISTS studio_artifacts (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                artifact_type TEXT NOT NULL DEFAULT 'url',
                name TEXT NOT NULL DEFAULT '',
                value TEXT NOT NULL DEFAULT '',
                metadata TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_status ON studio_sessions(status);
            CREATE INDEX IF NOT EXISTS idx_sessions_user ON studio_sessions(user_id);
            CREATE INDEX IF NOT EXISTS idx_actions_session ON studio_actions(session_id);
            CREATE INDEX IF NOT EXISTS idx_actions_status ON studio_actions(status);
            CREATE INDEX IF NOT EXISTS idx_actions_seq ON studio_actions(session_id, sequence_num);
            CREATE INDEX IF NOT EXISTS idx_artifacts_session ON studio_artifacts(session_id);",
];

#[derive(Clone)]
pub struct StudioStore {
    db: Arc<Mutex<Connection>>,
}

impl StudioStore {
    /// Open (or create) the studio SQLite database.
    pub fn new(db_path: &PathBuf) -> Result<Self, String> {
        let conn =
            Connection::open(db_path).map_err(|e| format!("Failed to open studio db: {}", e))?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=ON;",
        )
        .map_err(|e| format!("Failed to set pragmas: {}", e))?;

        crate::db::run_migrations(&conn, STUDIO_MIGRATIONS)
            .map_err(|e| format!("Studio schema migration failed: {e}"))?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory studio store (for fallback / tests).
    pub fn in_memory() -> Result<Self, String> {
        let path = PathBuf::from(":memory:");
        Self::new(&path)
    }

    // ── Sessions ────────────────────────────────────────────────

    /// Create a new studio session.
    pub async fn create_session(&self, session: &StudioSessionRow) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        match db.execute(
            "INSERT INTO studio_sessions (id, user_id, goal, status, plan_id, room_id, agent_id, session_id, total_actions, completed_actions, failed_actions, error_message, metadata, created_at, updated_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                session.id, session.user_id, session.goal, session.status,
                session.plan_id, session.room_id, session.agent_id, session.session_id,
                session.total_actions, session.completed_actions, session.failed_actions,
                session.error_message, session.metadata_json,
                now, now, session.completed_at,
            ],
        ) {
            Ok(_) => true,
            Err(e) => { warn!("Failed to create studio session {}: {}", session.id, e); false }
        }
    }

    /// Get a session by ID.
    pub async fn get_session(&self, id: &str) -> Option<StudioSessionRow> {
        let db = self.db.lock().await;
        db.query_row(
            "SELECT id, user_id, goal, status, plan_id, room_id, agent_id, session_id, total_actions, completed_actions, failed_actions, error_message, metadata, created_at, updated_at, completed_at
             FROM studio_sessions WHERE id = ?1",
            params![id],
            |row| Ok(row_to_session(row)),
        ).ok()
    }

    /// List sessions for a user (most recent first).
    pub async fn list_sessions(&self, user_id: &str, limit: u32) -> Vec<StudioSessionRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, user_id, goal, status, plan_id, room_id, agent_id, session_id, total_actions, completed_actions, failed_actions, error_message, metadata, created_at, updated_at, completed_at
             FROM studio_sessions WHERE user_id = ?1 ORDER BY created_at DESC LIMIT ?2"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map(params![user_id, limit], |row| Ok(row_to_session(row))) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// List all active sessions (not idle/complete/failed).
    pub async fn list_active_sessions(&self) -> Vec<StudioSessionRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, user_id, goal, status, plan_id, room_id, agent_id, session_id, total_actions, completed_actions, failed_actions, error_message, metadata, created_at, updated_at, completed_at
             FROM studio_sessions WHERE status IN ('planning', 'awaiting_approval', 'driving', 'paused')
             ORDER BY updated_at DESC"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map([], |row| Ok(row_to_session(row))) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// Update session status.
    pub async fn update_session_status(&self, id: &str, status: &str, error: Option<&str>) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        let completed = match status {
            "complete" | "failed" => Some(now.clone()),
            _ => None,
        };
        let err = error.unwrap_or("");

        match db.execute(
            "UPDATE studio_sessions SET
                status = ?1,
                error_message = CASE WHEN ?2 = '' THEN error_message ELSE ?2 END,
                updated_at = ?3,
                completed_at = COALESCE(?4, completed_at)
             WHERE id = ?5",
            params![status, err, now, completed, id],
        ) {
            Ok(n) => n > 0,
            Err(e) => {
                warn!("Failed to update session {}: {}", id, e);
                false
            }
        }
    }

    /// Link a plan to a session.
    pub async fn link_plan(&self, session_id: &str, plan_id: &str) -> bool {
        let db = self.db.lock().await;
        match db.execute(
            "UPDATE studio_sessions SET plan_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![plan_id, Utc::now().to_rfc3339(), session_id],
        ) {
            Ok(n) => n > 0,
            Err(_) => false,
        }
    }

    /// Link a War Room to a session.
    pub async fn link_room(&self, session_id: &str, room_id: &str) -> bool {
        let db = self.db.lock().await;
        match db.execute(
            "UPDATE studio_sessions SET room_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![room_id, Utc::now().to_rfc3339(), session_id],
        ) {
            Ok(n) => n > 0,
            Err(_) => false,
        }
    }

    /// Delete a session and its actions/artifacts.
    pub async fn delete_session(&self, id: &str) -> bool {
        let db = self.db.lock().await;
        let _ = db.execute(
            "DELETE FROM studio_actions WHERE session_id = ?1",
            params![id],
        );
        let _ = db.execute(
            "DELETE FROM studio_artifacts WHERE session_id = ?1",
            params![id],
        );
        match db.execute("DELETE FROM studio_sessions WHERE id = ?1", params![id]) {
            Ok(n) => n > 0,
            Err(_) => false,
        }
    }

    // ── Actions (Puppet Commands) ───────────────────────────────

    /// Queue a new UI action for a session.
    pub async fn queue_action(&self, action: &StudioActionRow) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        match db.execute(
            "INSERT INTO studio_actions (id, session_id, action_type, target, value, description, delay_ms, status, error_message, sequence_num, elapsed_ms, created_at, executed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                action.id, action.session_id, action.action_type,
                action.target, action.value, action.description,
                action.delay_ms, action.status, action.error_message,
                action.sequence_num, action.elapsed_ms, now, action.executed_at,
            ],
        ) {
            Ok(_) => {
                // Increment total_actions on the session
                let _ = db.execute(
                    "UPDATE studio_sessions SET total_actions = total_actions + 1, updated_at = ?1 WHERE id = ?2",
                    params![now, action.session_id],
                );
                true
            }
            Err(e) => { warn!("Failed to queue action {}: {}", action.id, e); false }
        }
    }

    /// Record the result of an executed action.
    pub async fn complete_action(
        &self,
        action_id: &str,
        success: bool,
        error: Option<&str>,
        elapsed_ms: u64,
    ) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        let status = if success { "completed" } else { "failed" };
        let err = error.unwrap_or("");

        match db.execute(
            "UPDATE studio_actions SET status = ?1, error_message = ?2, elapsed_ms = ?3, executed_at = ?4 WHERE id = ?5",
            params![status, err, elapsed_ms as i64, now, action_id],
        ) {
            Ok(n) if n > 0 => {
                // Get session_id for this action
                if let Ok(session_id) = db.query_row(
                    "SELECT session_id FROM studio_actions WHERE id = ?1",
                    params![action_id],
                    |row| row.get::<_, String>(0),
                ) {
                    let counter = if success { "completed_actions" } else { "failed_actions" };
                    let _ = db.execute(
                        &format!("UPDATE studio_sessions SET {} = {} + 1, updated_at = ?1 WHERE id = ?2", counter, counter),
                        params![now, session_id],
                    );
                }
                true
            }
            _ => false,
        }
    }

    /// Get pending actions for a session (in sequence order).
    pub async fn pending_actions(&self, session_id: &str) -> Vec<StudioActionRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, session_id, action_type, target, value, description, delay_ms, status, error_message, sequence_num, elapsed_ms, created_at, executed_at
             FROM studio_actions WHERE session_id = ?1 AND status = 'pending'
             ORDER BY sequence_num ASC"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map(params![session_id], |row| Ok(row_to_action(row))) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// Get all actions for a session (for replay).
    pub async fn all_actions(&self, session_id: &str) -> Vec<StudioActionRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, session_id, action_type, target, value, description, delay_ms, status, error_message, sequence_num, elapsed_ms, created_at, executed_at
             FROM studio_actions WHERE session_id = ?1
             ORDER BY sequence_num ASC"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map(params![session_id], |row| Ok(row_to_action(row))) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    // ── Artifacts ───────────────────────────────────────────────

    /// Add an artifact to a session.
    pub async fn add_artifact(&self, artifact: &StudioArtifactRow) -> bool {
        let db = self.db.lock().await;
        match db.execute(
            "INSERT INTO studio_artifacts (id, session_id, artifact_type, name, value, metadata, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                artifact.id, artifact.session_id, artifact.artifact_type,
                artifact.name, artifact.value, artifact.metadata_json,
                Utc::now().to_rfc3339(),
            ],
        ) {
            Ok(_) => true,
            Err(e) => { warn!("Failed to add artifact {}: {}", artifact.id, e); false }
        }
    }

    /// Get artifacts for a session.
    pub async fn get_artifacts(&self, session_id: &str) -> Vec<StudioArtifactRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, session_id, artifact_type, name, value, metadata, created_at
             FROM studio_artifacts WHERE session_id = ?1 ORDER BY created_at ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map(params![session_id], |row| {
            Ok(StudioArtifactRow {
                id: row.get(0).unwrap_or_default(),
                session_id: row.get(1).unwrap_or_default(),
                artifact_type: row.get(2).unwrap_or_default(),
                name: row.get(3).unwrap_or_default(),
                value: row.get(4).unwrap_or_default(),
                metadata_json: row.get(5).unwrap_or_default(),
                created_at: row.get(6).unwrap_or_default(),
            })
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    // ── Stats ───────────────────────────────────────────────────

    /// Get overall studio stats.
    pub async fn stats(&self) -> StudioStats {
        let db = self.db.lock().await;
        let total_sessions: u64 = db
            .query_row("SELECT COUNT(*) FROM studio_sessions", [], |row| row.get(0))
            .unwrap_or(0);
        let active_sessions: u64 = db.query_row(
            "SELECT COUNT(*) FROM studio_sessions WHERE status IN ('planning', 'awaiting_approval', 'driving', 'paused')", [], |row| row.get(0)
        ).unwrap_or(0);
        let completed_sessions: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM studio_sessions WHERE status = 'complete'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let total_actions: u64 = db
            .query_row("SELECT COUNT(*) FROM studio_actions", [], |row| row.get(0))
            .unwrap_or(0);
        let total_artifacts: u64 = db
            .query_row("SELECT COUNT(*) FROM studio_artifacts", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);

        StudioStats {
            total_sessions,
            active_sessions,
            completed_sessions,
            total_actions,
            total_artifacts,
        }
    }
}

// ============================================================================
// Row types
// ============================================================================

fn row_to_session(row: &rusqlite::Row) -> StudioSessionRow {
    StudioSessionRow {
        id: row.get(0).unwrap_or_default(),
        user_id: row.get(1).unwrap_or_default(),
        goal: row.get(2).unwrap_or_default(),
        status: row.get(3).unwrap_or_default(),
        plan_id: row.get(4).ok(),
        room_id: row.get(5).ok(),
        agent_id: row.get(6).ok(),
        session_id: row.get(7).ok(),
        total_actions: row.get(8).unwrap_or(0),
        completed_actions: row.get(9).unwrap_or(0),
        failed_actions: row.get(10).unwrap_or(0),
        error_message: row.get(11).unwrap_or_default(),
        metadata_json: row.get(12).unwrap_or_default(),
        created_at: row.get(13).unwrap_or_default(),
        updated_at: row.get(14).unwrap_or_default(),
        completed_at: row.get(15).ok(),
    }
}

fn row_to_action(row: &rusqlite::Row) -> StudioActionRow {
    StudioActionRow {
        id: row.get(0).unwrap_or_default(),
        session_id: row.get(1).unwrap_or_default(),
        action_type: row.get(2).unwrap_or_default(),
        target: row.get(3).unwrap_or_default(),
        value: row.get(4).unwrap_or_default(),
        description: row.get(5).unwrap_or_default(),
        delay_ms: row.get(6).unwrap_or(0),
        status: row.get(7).unwrap_or_default(),
        error_message: row.get(8).unwrap_or_default(),
        sequence_num: row.get(9).unwrap_or(0),
        elapsed_ms: row.get(10).unwrap_or(0),
        created_at: row.get(11).unwrap_or_default(),
        executed_at: row.get(12).ok(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudioSessionRow {
    pub id: String,
    pub user_id: String,
    pub goal: String,
    /// Status: "idle", "planning", "awaiting_approval", "driving", "paused", "complete", "failed"
    pub status: String,
    /// Linked plan card ID (from Phase 2 planning)
    pub plan_id: Option<String>,
    /// Linked War Room ID (for observers)
    pub room_id: Option<String>,
    /// Agent ID driving this session
    pub agent_id: Option<String>,
    /// Chat session ID for LLM context
    pub session_id: Option<String>,
    pub total_actions: u64,
    pub completed_actions: u64,
    pub failed_actions: u64,
    pub error_message: String,
    pub metadata_json: String,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudioActionRow {
    pub id: String,
    pub session_id: String,
    /// Action type: "navigate", "click", "type", "scroll", "select", "wait", "highlight", "clear_highlight"
    pub action_type: String,
    /// CSS selector, route path, or field identifier
    pub target: String,
    /// Value for type/fill/select actions
    pub value: String,
    /// Human-readable description: "Navigating to Deploy page"
    pub description: String,
    /// Delay before executing (for animation pacing)
    pub delay_ms: u64,
    /// Status: "pending", "executing", "completed", "failed", "skipped"
    pub status: String,
    pub error_message: String,
    /// Order of execution within the session
    pub sequence_num: u64,
    /// Time taken to execute (ms)
    pub elapsed_ms: u64,
    pub created_at: String,
    pub executed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudioArtifactRow {
    pub id: String,
    pub session_id: String,
    /// Artifact type: "url", "file", "screenshot", "code"
    pub artifact_type: String,
    pub name: String,
    pub value: String,
    pub metadata_json: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudioStats {
    pub total_sessions: u64,
    pub active_sessions: u64,
    pub completed_sessions: u64,
    pub total_actions: u64,
    pub total_artifacts: u64,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_test_store() -> StudioStore {
        StudioStore::new(&PathBuf::from(":memory:")).unwrap()
    }

    fn test_session(id: &str, goal: &str) -> StudioSessionRow {
        StudioSessionRow {
            id: id.to_string(),
            user_id: "user-1".to_string(),
            goal: goal.to_string(),
            status: "idle".to_string(),
            plan_id: None,
            room_id: None,
            agent_id: None,
            session_id: None,
            total_actions: 0,
            completed_actions: 0,
            failed_actions: 0,
            error_message: String::new(),
            metadata_json: "{}".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
            completed_at: None,
        }
    }

    fn test_action(id: &str, session_id: &str, action_type: &str, seq: u64) -> StudioActionRow {
        StudioActionRow {
            id: id.to_string(),
            session_id: session_id.to_string(),
            action_type: action_type.to_string(),
            target: "/deploy".to_string(),
            value: String::new(),
            description: format!("{} action", action_type),
            delay_ms: 100,
            status: "pending".to_string(),
            error_message: String::new(),
            sequence_num: seq,
            elapsed_ms: 0,
            created_at: String::new(),
            executed_at: None,
        }
    }

    #[tokio::test]
    async fn test_create_and_get_session() {
        let store = make_test_store();
        let session = test_session("s1", "Build a landing page");
        assert!(store.create_session(&session).await);

        let fetched = store.get_session("s1").await;
        assert!(fetched.is_some());
        let f = fetched.unwrap();
        assert_eq!(f.goal, "Build a landing page");
        assert_eq!(f.status, "idle");
    }

    #[tokio::test]
    async fn test_session_status_lifecycle() {
        let store = make_test_store();
        store.create_session(&test_session("s1", "Build app")).await;

        store.update_session_status("s1", "planning", None).await;
        let s = store.get_session("s1").await.unwrap();
        assert_eq!(s.status, "planning");

        store.update_session_status("s1", "driving", None).await;
        let s = store.get_session("s1").await.unwrap();
        assert_eq!(s.status, "driving");

        store.update_session_status("s1", "complete", None).await;
        let s = store.get_session("s1").await.unwrap();
        assert_eq!(s.status, "complete");
        assert!(s.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_session_failure() {
        let store = make_test_store();
        store.create_session(&test_session("s1", "Build app")).await;

        store
            .update_session_status("s1", "failed", Some("LLM timeout"))
            .await;
        let s = store.get_session("s1").await.unwrap();
        assert_eq!(s.status, "failed");
        assert_eq!(s.error_message, "LLM timeout");
        assert!(s.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_list_sessions() {
        let store = make_test_store();
        store.create_session(&test_session("s1", "Task 1")).await;
        store.create_session(&test_session("s2", "Task 2")).await;
        store.create_session(&test_session("s3", "Task 3")).await;

        let sessions = store.list_sessions("user-1", 10).await;
        assert_eq!(sessions.len(), 3);
    }

    #[tokio::test]
    async fn test_active_sessions() {
        let store = make_test_store();
        store.create_session(&test_session("s1", "Task 1")).await;
        store.create_session(&test_session("s2", "Task 2")).await;

        store.update_session_status("s1", "driving", None).await;
        store.update_session_status("s2", "complete", None).await;

        let active = store.list_active_sessions().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "s1");
    }

    #[tokio::test]
    async fn test_link_plan_and_room() {
        let store = make_test_store();
        store.create_session(&test_session("s1", "Build app")).await;

        assert!(store.link_plan("s1", "plan-abc").await);
        assert!(store.link_room("s1", "room-war").await);

        let s = store.get_session("s1").await.unwrap();
        assert_eq!(s.plan_id, Some("plan-abc".to_string()));
        assert_eq!(s.room_id, Some("room-war".to_string()));
    }

    #[tokio::test]
    async fn test_queue_and_complete_actions() {
        let store = make_test_store();
        store.create_session(&test_session("s1", "Build app")).await;

        store
            .queue_action(&test_action("a1", "s1", "navigate", 0))
            .await;
        store
            .queue_action(&test_action("a2", "s1", "click", 1))
            .await;
        store
            .queue_action(&test_action("a3", "s1", "type", 2))
            .await;

        // Session should have total_actions = 3
        let s = store.get_session("s1").await.unwrap();
        assert_eq!(s.total_actions, 3);

        // All 3 should be pending
        let pending = store.pending_actions("s1").await;
        assert_eq!(pending.len(), 3);
        assert_eq!(pending[0].action_type, "navigate");

        // Complete first action
        store.complete_action("a1", true, None, 50).await;
        let s = store.get_session("s1").await.unwrap();
        assert_eq!(s.completed_actions, 1);

        // Fail second action
        store
            .complete_action("a2", false, Some("Element not found"), 30)
            .await;
        let s = store.get_session("s1").await.unwrap();
        assert_eq!(s.failed_actions, 1);

        // 1 pending remaining
        let pending = store.pending_actions("s1").await;
        assert_eq!(pending.len(), 1);
    }

    #[tokio::test]
    async fn test_action_replay() {
        let store = make_test_store();
        store.create_session(&test_session("s1", "Build app")).await;

        store
            .queue_action(&test_action("a1", "s1", "navigate", 0))
            .await;
        store
            .queue_action(&test_action("a2", "s1", "click", 1))
            .await;
        store.complete_action("a1", true, None, 50).await;

        let all = store.all_actions("s1").await;
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].status, "completed");
        assert_eq!(all[1].status, "pending");
    }

    #[tokio::test]
    async fn test_artifacts() {
        let store = make_test_store();
        store.create_session(&test_session("s1", "Build app")).await;

        store
            .add_artifact(&StudioArtifactRow {
                id: "art-1".to_string(),
                session_id: "s1".to_string(),
                artifact_type: "url".to_string(),
                name: "Landing Page".to_string(),
                value: "https://thunderfc.zeuslab.ai".to_string(),
                metadata_json: "{}".to_string(),
                created_at: String::new(),
            })
            .await;

        store
            .add_artifact(&StudioArtifactRow {
                id: "art-2".to_string(),
                session_id: "s1".to_string(),
                artifact_type: "screenshot".to_string(),
                name: "Final preview".to_string(),
                value: "/screenshots/s1-final.png".to_string(),
                metadata_json: "{}".to_string(),
                created_at: String::new(),
            })
            .await;

        let artifacts = store.get_artifacts("s1").await;
        assert_eq!(artifacts.len(), 2);
        assert_eq!(artifacts[0].artifact_type, "url");
        assert_eq!(artifacts[1].artifact_type, "screenshot");
    }

    #[tokio::test]
    async fn test_delete_session_cascades() {
        let store = make_test_store();
        store.create_session(&test_session("s1", "Build app")).await;
        store
            .queue_action(&test_action("a1", "s1", "navigate", 0))
            .await;
        store
            .add_artifact(&StudioArtifactRow {
                id: "art-1".to_string(),
                session_id: "s1".to_string(),
                artifact_type: "url".to_string(),
                name: "Test".to_string(),
                value: "https://test.com".to_string(),
                metadata_json: "{}".to_string(),
                created_at: String::new(),
            })
            .await;

        assert!(store.delete_session("s1").await);
        assert!(store.get_session("s1").await.is_none());
        assert!(store.all_actions("s1").await.is_empty());
        assert!(store.get_artifacts("s1").await.is_empty());
    }

    #[tokio::test]
    async fn test_stats() {
        let store = make_test_store();
        store.create_session(&test_session("s1", "Task 1")).await;
        store.create_session(&test_session("s2", "Task 2")).await;
        store.update_session_status("s1", "driving", None).await;
        store.update_session_status("s2", "complete", None).await;
        store
            .queue_action(&test_action("a1", "s1", "navigate", 0))
            .await;

        let stats = store.stats().await;
        assert_eq!(stats.total_sessions, 2);
        assert_eq!(stats.active_sessions, 1);
        assert_eq!(stats.completed_sessions, 1);
        assert_eq!(stats.total_actions, 1);
    }
}
