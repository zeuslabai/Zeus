//! SQLite-backed agent task store for checkpoint/resume across restarts (S52-T1).
//!
//! Follows the `DeployStore` / `PantheonStore` pattern:
//! `Arc<Mutex<Connection>>` with WAL mode and versioned migrations.
//!
//! Tables:
//!  - `agent_tasks` — persistent task records with checkpoint blobs

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;

/// Versioned schema migrations for the task store.
const TASK_MIGRATIONS: &[&str] = &[
    // v1 — initial schema
    "CREATE TABLE IF NOT EXISTS agent_tasks (
        id TEXT PRIMARY KEY,
        agent_id TEXT NOT NULL DEFAULT 'default',
        description TEXT NOT NULL DEFAULT '',
        status TEXT NOT NULL DEFAULT 'pending',
        checkpoint TEXT NOT NULL DEFAULT '{}',
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_tasks_agent ON agent_tasks(agent_id);
    CREATE INDEX IF NOT EXISTS idx_tasks_status ON agent_tasks(status);
    CREATE INDEX IF NOT EXISTS idx_tasks_updated ON agent_tasks(updated_at);",
    // v2 — task-driven autonomy fields (scope, budget, assignment tracking)
    "ALTER TABLE agent_tasks ADD COLUMN scope_json TEXT NOT NULL DEFAULT '{}';
    ALTER TABLE agent_tasks ADD COLUMN iterations_used INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE agent_tasks ADD COLUMN iterations_budget INTEGER NOT NULL DEFAULT 20;
    ALTER TABLE agent_tasks ADD COLUMN assigned_by TEXT NOT NULL DEFAULT 'coordinator';
    ALTER TABLE agent_tasks ADD COLUMN source_channel TEXT NOT NULL DEFAULT '';
    ALTER TABLE agent_tasks ADD COLUMN parent_id TEXT;
    ALTER TABLE agent_tasks ADD COLUMN branch TEXT NOT NULL DEFAULT '';
    ALTER TABLE agent_tasks ADD COLUMN priority INTEGER NOT NULL DEFAULT 1;",
];

/// Task status lifecycle: pending → active → paused → completed / failed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Active,
    Paused,
    Completed,
    Failed,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "active" => Self::Active,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A persisted agent task row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: String,
    pub agent_id: String,
    pub description: String,
    pub status: TaskStatus,
    /// JSON checkpoint blob — agents save progress here before shutdown.
    pub checkpoint: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
    /// Structured task scope: files, acceptance checks, budget
    #[serde(default)]
    pub scope_json: serde_json::Value,
    /// Number of cooking iterations used so far
    #[serde(default)]
    pub iterations_used: i64,
    /// Maximum cooking iterations budget
    #[serde(default = "default_iterations_budget")]
    pub iterations_budget: i64,
    /// Who assigned this task (coordinator, human, self, trigger)
    #[serde(default)]
    pub assigned_by: String,
    /// Source channel (discord:123, telegram:-456, heartbeat.md)
    #[serde(default)]
    pub source_channel: String,
    /// Parent task ID for subtask trees
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Git branch for this task
    #[serde(default)]
    pub branch: String,
    /// Priority (1=low, 5=critical)
    #[serde(default = "default_priority")]
    pub priority: i64,
}

fn default_iterations_budget() -> i64 { 20 }
fn default_priority() -> i64 { 1 }

// ============================================================================
// TaskStore
// ============================================================================

#[derive(Clone)]
pub struct TaskStore {
    db: Arc<Mutex<Connection>>,
}

impl TaskStore {
    /// Open (or create) the task SQLite database.
    pub fn new(db_path: &PathBuf) -> Result<Self, String> {
        let conn =
            Connection::open(db_path).map_err(|e| format!("Failed to open task db: {}", e))?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=ON;",
        )
        .map_err(|e| format!("Failed to set pragmas: {}", e))?;

        crate::db::run_migrations(&conn, TASK_MIGRATIONS)
            .map_err(|e| format!("Task schema migration failed: {e}"))?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory task store (for fallback / tests).
    pub fn in_memory() -> Result<Self, String> {
        let path = PathBuf::from(":memory:");
        Self::new(&path)
    }

    // ── CRUD ────────────────────────────────────────────────────

    /// Create a new task. Returns the task ID.
    pub async fn create(&self, task: &AgentTask) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        let checkpoint_str = serde_json::to_string(&task.checkpoint).unwrap_or_default();
        let scope_str = serde_json::to_string(&task.scope_json).unwrap_or_default();
        match db.execute(
            "INSERT INTO agent_tasks (id, agent_id, description, status, checkpoint, created_at, updated_at,
             scope_json, iterations_used, iterations_budget, assigned_by, source_channel, parent_id, branch, priority)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                task.id, task.agent_id, task.description,
                task.status.as_str(), checkpoint_str, now, now,
                scope_str, task.iterations_used, task.iterations_budget,
                task.assigned_by, task.source_channel,
                task.parent_id, task.branch, task.priority,
            ],
        ) {
            Ok(_) => true,
            Err(e) => { warn!("Failed to create task {}: {}", task.id, e); false }
        }
    }

    /// Get a task by ID.
    pub async fn get(&self, id: &str) -> Option<AgentTask> {
        let db = self.db.lock().await;
        db.query_row(
            "SELECT id, agent_id, description, status, checkpoint, created_at, updated_at
             FROM agent_tasks WHERE id = ?1",
            params![id],
            |row| Ok(row_to_task(row)),
        ).ok()
    }

    /// List tasks, optionally filtered by agent_id and/or status.
    pub async fn list(
        &self,
        agent_id: Option<&str>,
        status: Option<TaskStatus>,
        limit: usize,
        offset: usize,
    ) -> Vec<AgentTask> {
        let db = self.db.lock().await;
        let (sql, param_values) = build_list_query(agent_id, status, limit, offset);
        let mut stmt = match db.prepare(&sql) {
            Ok(s) => s,
            Err(e) => { warn!("Failed to prepare task list query: {}", e); return vec![]; }
        };
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = param_values
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| Ok(row_to_task(row)));
        match rows {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(e) => { warn!("Failed to list tasks: {}", e); vec![] }
        }
    }

    /// Update task status and/or checkpoint.
    pub async fn update(
        &self,
        id: &str,
        status: Option<TaskStatus>,
        checkpoint: Option<&serde_json::Value>,
        description: Option<&str>,
    ) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();

        // Read existing, merge changes, write back
        let existing = db.query_row(
            "SELECT id, agent_id, description, status, checkpoint, created_at, updated_at
             FROM agent_tasks WHERE id = ?1",
            params![id],
            |row| Ok(row_to_task(row)),
        );
        let existing = match existing {
            Ok(t) => t,
            Err(_) => return false,
        };

        let final_status = status.map(|s| s.as_str().to_string())
            .unwrap_or_else(|| existing.status.as_str().to_string());
        let final_checkpoint = checkpoint
            .map(|c| serde_json::to_string(c).unwrap_or_default())
            .unwrap_or_else(|| serde_json::to_string(&existing.checkpoint).unwrap_or_default());
        let final_description = description.unwrap_or(&existing.description).to_string();

        match db.execute(
            "UPDATE agent_tasks SET status = ?1, checkpoint = ?2, description = ?3, updated_at = ?4
             WHERE id = ?5",
            params![final_status, final_checkpoint, final_description, now, id],
        ) {
            Ok(n) => n > 0,
            Err(e) => { warn!("Failed to update task {}: {}", id, e); false }
        }
    }

    /// Delete a task by ID.
    pub async fn delete(&self, id: &str) -> bool {
        let db = self.db.lock().await;
        match db.execute("DELETE FROM agent_tasks WHERE id = ?1", params![id]) {
            Ok(n) => n > 0,
            Err(e) => { warn!("Failed to delete task {}: {}", id, e); false }
        }
    }

    /// Get all tasks with status=active (for resume on startup).
    pub async fn get_active_tasks(&self) -> Vec<AgentTask> {
        self.list(None, Some(TaskStatus::Active), 100, 0).await
    }

    /// Persist an auto-detected task assignment from a coordinator message.
    ///
    /// Idempotent on `source_channel` — if a task with the same source_channel
    /// (e.g. `discord:1488620262676238426:msg_id`) already exists, returns its ID
    /// without inserting a duplicate. This protects against re-detection on
    /// heartbeat replays or gateway restarts.
    ///
    /// Returns `(task_id, was_inserted)`. `was_inserted=false` means an existing
    /// task was found and returned — caller can use this to skip HEARTBEAT.md writes.
    pub async fn persist_detected(
        &self,
        agent_id: &str,
        description: &str,
        source_channel: &str,
        assigned_by: &str,
    ) -> Result<(String, bool), String> {
        // Idempotency check: look up by source_channel first.
        {
            let db = self.db.lock().await;
            let existing: Option<String> = db
                .query_row(
                    "SELECT id FROM agent_tasks WHERE source_channel = ?1 LIMIT 1",
                    params![source_channel],
                    |row| row.get(0),
                )
                .ok();
            if let Some(id) = existing {
                return Ok((id, false));
            }
        }

        // Insert new task.
        let task = AgentTask {
            id: format!("task_{}", uuid::Uuid::new_v4().simple()),
            agent_id: agent_id.to_string(),
            description: description.to_string(),
            status: TaskStatus::Pending,
            checkpoint: serde_json::Value::Object(Default::default()),
            created_at: String::new(),
            updated_at: String::new(),
            scope_json: serde_json::Value::Object(Default::default()),
            iterations_used: 0,
            iterations_budget: default_iterations_budget(),
            assigned_by: assigned_by.to_string(),
            source_channel: source_channel.to_string(),
            parent_id: None,
            branch: String::new(),
            priority: default_priority(),
        };

        if self.create(&task).await {
            Ok((task.id, true))
        } else {
            Err(format!(
                "Failed to persist detected task for agent {}",
                agent_id
            ))
        }
    }

    /// Count tasks by status.
    pub async fn count_by_status(&self) -> std::collections::HashMap<String, usize> {
        let db = self.db.lock().await;
        let mut result = std::collections::HashMap::new();
        let mut stmt = match db.prepare(
            "SELECT status, COUNT(*) FROM agent_tasks GROUP BY status"
        ) {
            Ok(s) => s,
            Err(_) => return result,
        };
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
        });
        if let Ok(mapped) = rows {
            for r in mapped.flatten() {
                result.insert(r.0, r.1);
            }
        }
        result
    }
}

// ── Helpers ─────────────────────────────────────────────────────

fn row_to_task(row: &rusqlite::Row) -> AgentTask {
    let status_str: String = row.get(3).unwrap_or_default();
    let checkpoint_str: String = row.get(4).unwrap_or_default();
    let scope_str: String = row.get(7).unwrap_or_default();
    AgentTask {
        id: row.get(0).unwrap_or_default(),
        agent_id: row.get(1).unwrap_or_default(),
        description: row.get(2).unwrap_or_default(),
        status: TaskStatus::parse(&status_str),
        checkpoint: serde_json::from_str(&checkpoint_str).unwrap_or(serde_json::Value::Object(Default::default())),
        created_at: row.get(5).unwrap_or_default(),
        updated_at: row.get(6).unwrap_or_default(),
        scope_json: serde_json::from_str(&scope_str).unwrap_or(serde_json::Value::Object(Default::default())),
        iterations_used: row.get(8).unwrap_or(0),
        iterations_budget: row.get(9).unwrap_or(20),
        assigned_by: row.get(10).unwrap_or_default(),
        source_channel: row.get(11).unwrap_or_default(),
        parent_id: row.get(12).unwrap_or(None),
        branch: row.get(13).unwrap_or_default(),
        priority: row.get(14).unwrap_or(1),
    }
}

fn build_list_query(
    agent_id: Option<&str>,
    status: Option<TaskStatus>,
    limit: usize,
    offset: usize,
) -> (String, Vec<String>) {
    let mut sql = String::from(
        "SELECT id, agent_id, description, status, checkpoint, created_at, updated_at, scope_json, iterations_used, iterations_budget, assigned_by, source_channel, parent_id, branch, priority FROM agent_tasks"
    );
    let mut conditions = Vec::new();
    let mut params = Vec::new();
    let mut idx = 1;

    if let Some(aid) = agent_id {
        conditions.push(format!("agent_id = ?{}", idx));
        params.push(aid.to_string());
        idx += 1;
    }
    if let Some(s) = status {
        conditions.push(format!("status = ?{}", idx));
        params.push(s.as_str().to_string());
        idx += 1;
    }
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    sql.push_str(" ORDER BY updated_at DESC");
    sql.push_str(&format!(" LIMIT ?{}", idx));
    params.push(limit.to_string());
    idx += 1;
    sql.push_str(&format!(" OFFSET ?{}", idx));
    params.push(offset.to_string());

    (sql, params)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> TaskStore {
        TaskStore::in_memory().unwrap()
    }

    #[tokio::test]
    async fn test_create_and_get() {
        let store = test_store().await;
        let task = AgentTask {
            id: "t1".to_string(),
            agent_id: "agent-a".to_string(),
            description: "Build the thing".to_string(),
            status: TaskStatus::Pending,
            checkpoint: serde_json::json!({}),
            created_at: String::new(),
            updated_at: String::new(),
            scope_json: serde_json::json!({}),
            iterations_used: 0,
            iterations_budget: default_iterations_budget(),
            assigned_by: String::new(),
            source_channel: String::new(),
            parent_id: None,
            branch: String::new(),
            priority: default_priority(),
        };
        assert!(store.create(&task).await);
        let got = store.get("t1").await.expect("should find task");
        assert_eq!(got.id, "t1");
        assert_eq!(got.agent_id, "agent-a");
        assert_eq!(got.description, "Build the thing");
        assert_eq!(got.status, TaskStatus::Pending);
    }

    #[tokio::test]
    async fn test_update_status_and_checkpoint() {
        let store = test_store().await;
        let task = AgentTask {
            id: "t2".to_string(),
            agent_id: "default".to_string(),
            description: "Test task".to_string(),
            status: TaskStatus::Pending,
            checkpoint: serde_json::json!({}),
            created_at: String::new(),
            updated_at: String::new(),
            scope_json: serde_json::json!({}),
            iterations_used: 0,
            iterations_budget: default_iterations_budget(),
            assigned_by: String::new(),
            source_channel: String::new(),
            parent_id: None,
            branch: String::new(),
            priority: default_priority(),
        };
        store.create(&task).await;

        // Update to active with checkpoint
        let cp = serde_json::json!({"step": 3, "progress": "compiling"});
        assert!(store.update("t2", Some(TaskStatus::Active), Some(&cp), None).await);

        let got = store.get("t2").await.unwrap();
        assert_eq!(got.status, TaskStatus::Active);
        assert_eq!(got.checkpoint["step"], 3);
    }

    #[tokio::test]
    async fn test_list_with_filters() {
        let store = test_store().await;
        for (id, agent, status) in [
            ("a1", "zeus100", TaskStatus::Active),
            ("a2", "zeus100", TaskStatus::Completed),
            ("a3", "zeus112", TaskStatus::Active),
        ] {
            store.create(&AgentTask {
                id: id.to_string(),
                agent_id: agent.to_string(),
                description: format!("Task {}", id),
                status,
                checkpoint: serde_json::json!({}),
                created_at: String::new(),
                updated_at: String::new(),
                scope_json: serde_json::json!({}),
                iterations_used: 0,
                iterations_budget: default_iterations_budget(),
                assigned_by: String::new(),
                source_channel: String::new(),
                parent_id: None,
                branch: String::new(),
                priority: default_priority(),
            }).await;
        }

        // All tasks
        let all = store.list(None, None, 100, 0).await;
        assert_eq!(all.len(), 3);

        // By agent
        let zeus100 = store.list(Some("zeus100"), None, 100, 0).await;
        assert_eq!(zeus100.len(), 2);

        // By status
        let active = store.list(None, Some(TaskStatus::Active), 100, 0).await;
        assert_eq!(active.len(), 2);

        // By both
        let z100_active = store.list(Some("zeus100"), Some(TaskStatus::Active), 100, 0).await;
        assert_eq!(z100_active.len(), 1);
    }

    #[tokio::test]
    async fn test_delete() {
        let store = test_store().await;
        store.create(&AgentTask {
            id: "del1".to_string(),
            agent_id: "default".to_string(),
            description: "To be deleted".to_string(),
            status: TaskStatus::Pending,
            checkpoint: serde_json::json!({}),
            created_at: String::new(),
            updated_at: String::new(),
            scope_json: serde_json::json!({}),
            iterations_used: 0,
            iterations_budget: default_iterations_budget(),
            assigned_by: String::new(),
            source_channel: String::new(),
            parent_id: None,
            branch: String::new(),
            priority: default_priority(),
        }).await;
        assert!(store.delete("del1").await);
        assert!(store.get("del1").await.is_none());
    }

    #[tokio::test]
    async fn test_get_active_tasks() {
        let store = test_store().await;
        for (id, status) in [("r1", TaskStatus::Active), ("r2", TaskStatus::Pending), ("r3", TaskStatus::Active)] {
            store.create(&AgentTask {
                id: id.to_string(),
                agent_id: "default".to_string(),
                description: format!("Task {}", id),
                status,
                checkpoint: serde_json::json!({"resume": true}),
                created_at: String::new(),
                updated_at: String::new(),
                scope_json: serde_json::json!({}),
                iterations_used: 0,
                iterations_budget: default_iterations_budget(),
                assigned_by: String::new(),
                source_channel: String::new(),
                parent_id: None,
                branch: String::new(),
                priority: default_priority(),
            }).await;
        }
        let active = store.get_active_tasks().await;
        assert_eq!(active.len(), 2);
    }

    #[tokio::test]
    async fn test_count_by_status() {
        let store = test_store().await;
        for (id, status) in [
            ("c1", TaskStatus::Active),
            ("c2", TaskStatus::Active),
            ("c3", TaskStatus::Completed),
            ("c4", TaskStatus::Failed),
        ] {
            store.create(&AgentTask {
                id: id.to_string(),
                agent_id: "default".to_string(),
                description: "count test".to_string(),
                status,
                checkpoint: serde_json::json!({}),
                created_at: String::new(),
                updated_at: String::new(),
                scope_json: serde_json::json!({}),
                iterations_used: 0,
                iterations_budget: default_iterations_budget(),
                assigned_by: String::new(),
                source_channel: String::new(),
                parent_id: None,
                branch: String::new(),
                priority: default_priority(),
            }).await;
        }
        let counts = store.count_by_status().await;
        assert_eq!(counts.get("active"), Some(&2));
        assert_eq!(counts.get("completed"), Some(&1));
        assert_eq!(counts.get("failed"), Some(&1));
    }

    #[tokio::test]
    async fn test_task_status_roundtrip() {
        for status in [TaskStatus::Pending, TaskStatus::Active, TaskStatus::Paused, TaskStatus::Completed, TaskStatus::Failed] {
            let json = serde_json::to_string(&status).unwrap();
            let parsed: TaskStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, parsed);
        }
    }
}
