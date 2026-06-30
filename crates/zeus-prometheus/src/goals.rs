//! Goal Stack - Persistent goal management with SQLite storage
//!
//! Goals are first-class persistent data structures that give Zeus directed,
//! purposeful behavior. They support hierarchical decomposition, dependency
//! tracking, priority ordering, and lifecycle management.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::debug;
use zeus_core::Result;

// ============================================================================
// Types
// ============================================================================

/// Priority level for goals
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Background = 0,
    Low = 1,
    Normal = 2,
    High = 3,
    Critical = 4,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Priority::Background => write!(f, "background"),
            Priority::Low => write!(f, "low"),
            Priority::Normal => write!(f, "normal"),
            Priority::High => write!(f, "high"),
            Priority::Critical => write!(f, "critical"),
        }
    }
}

impl Priority {
    fn from_i32(v: i32) -> Self {
        match v {
            0 => Priority::Background,
            1 => Priority::Low,
            3 => Priority::High,
            4 => Priority::Critical,
            _ => Priority::Normal,
        }
    }
}

/// Status of a goal
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GoalStatus {
    Pending,
    Active,
    Blocked { reason: String },
    Completed { outcome: String },
    Failed { reason: String },
    Abandoned { reason: String },
}

impl GoalStatus {
    fn to_db(&self) -> (&str, Option<&str>) {
        match self {
            GoalStatus::Pending => ("pending", None),
            GoalStatus::Active => ("active", None),
            GoalStatus::Blocked { reason } => ("blocked", Some(reason)),
            GoalStatus::Completed { outcome } => ("completed", Some(outcome)),
            GoalStatus::Failed { reason } => ("failed", Some(reason)),
            GoalStatus::Abandoned { reason } => ("abandoned", Some(reason)),
        }
    }

    fn from_db(state: &str, detail: Option<String>) -> Self {
        match state {
            "pending" => GoalStatus::Pending,
            "active" => GoalStatus::Active,
            "blocked" => GoalStatus::Blocked {
                reason: detail.unwrap_or_default(),
            },
            "completed" => GoalStatus::Completed {
                outcome: detail.unwrap_or_default(),
            },
            "failed" => GoalStatus::Failed {
                reason: detail.unwrap_or_default(),
            },
            "abandoned" => GoalStatus::Abandoned {
                reason: detail.unwrap_or_default(),
            },
            _ => GoalStatus::Pending,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            GoalStatus::Completed { .. } | GoalStatus::Failed { .. } | GoalStatus::Abandoned { .. }
        )
    }
}

/// How a goal was created
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GoalSource {
    User,
    System,
    Decomposition { parent_id: String },
}

impl GoalSource {
    fn to_db(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| r#"{"type":"system"}"#.to_string())
    }

    fn from_db(s: &str) -> Self {
        serde_json::from_str(s).unwrap_or(GoalSource::System)
    }
}

/// A persistent goal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub description: String,
    pub priority: Priority,
    pub status: GoalStatus,
    pub parent_id: Option<String>,
    pub blocked_by: Vec<String>,
    pub success_criteria: Vec<String>,
    pub deadline: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub context: String,
    pub source: GoalSource,
    /// #157: durable re-cook counter, incremented in code per non-terminal
    /// re-cook. Seeded from goal-file front-matter at promotion.
    pub attempt: u32,
    /// #157: bounded-retry cap. `0` = unbounded. Enforced in code at the
    /// re-cook site; the goal is Abandoned once `attempt >= max_attempts`.
    pub max_attempts: u32,
}

impl Goal {
    /// Create a new goal with defaults
    pub fn new(description: &str, priority: Priority, source: GoalSource) -> Self {
        let now = Utc::now();
        Self {
            id: ulid::Ulid::new().to_string(),
            description: description.to_string(),
            priority,
            status: GoalStatus::Pending,
            parent_id: None,
            blocked_by: Vec::new(),
            success_criteria: Vec::new(),
            deadline: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
            context: String::new(),
            source,
            attempt: 0,
            max_attempts: 0,
        }
    }
}

// ============================================================================
// GoalStack
// ============================================================================

const GOAL_MIGRATIONS: &[&str] = &[
    // v1: initial schema
    "CREATE TABLE IF NOT EXISTS goals (
                id TEXT PRIMARY KEY,
                description TEXT NOT NULL,
                priority INTEGER NOT NULL DEFAULT 2,
                status TEXT NOT NULL DEFAULT 'pending',
                status_detail TEXT,
                parent_id TEXT,
                blocked_by TEXT NOT NULL DEFAULT '[]',
                success_criteria TEXT NOT NULL DEFAULT '[]',
                deadline TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                completed_at TEXT,
                context TEXT NOT NULL DEFAULT '',
                source TEXT NOT NULL DEFAULT '{\"type\":\"system\"}'
            );
            CREATE INDEX IF NOT EXISTS idx_goals_status ON goals(status);
            CREATE INDEX IF NOT EXISTS idx_goals_priority ON goals(priority);
            CREATE INDEX IF NOT EXISTS idx_goals_parent ON goals(parent_id);",
    // v2 (#157): durable bounded-retry counter. The pre-promotion goal `.md`
    // file carries `attempt`/`max_attempts` in front-matter, but once the
    // hot-loader promotes the goal it deletes the file and the goal becomes a
    // SQLite row that `top_goal` re-selects every tick — there's no file left
    // to re-stamp, so the cap MUST live on the durable row. `attempt` is
    // incremented in code per non-terminal re-cook and enforced in code; the
    // front-matter value is only the pre-promotion seed. `max_attempts = 0`
    // means unbounded (legacy rows / goals created without a cap).
    "ALTER TABLE goals ADD COLUMN attempt INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE goals ADD COLUMN max_attempts INTEGER NOT NULL DEFAULT 0;",
];

/// SQLite-backed persistent goal stack
pub struct GoalStack {
    path: PathBuf,
}

impl GoalStack {
    /// Create a new GoalStack, initializing the database schema
    pub fn new(db_path: impl Into<PathBuf>) -> Result<Self> {
        let path = db_path.into();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let stack = Self { path };
        stack.init()?;
        Ok(stack)
    }

    /// Initialize the database schema
    fn init(&self) -> Result<()> {
        let conn = self.conn()?;
        crate::db::run_migrations(&conn, GOAL_MIGRATIONS)?;
        Ok(())
    }

    /// Open a fresh connection (follows scheduler.rs pattern)
    fn conn(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path)
            .map_err(|e| zeus_core::Error::Database(format!("Failed to open goals db: {}", e)))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| {
                zeus_core::Error::Database(format!("Failed to set goals db pragmas: {}", e))
            })?;
        Ok(conn)
    }

    /// Add a new goal, returning its ID
    pub fn add(&self, goal: &Goal) -> Result<String> {
        let conn = self.conn()?;
        let (status, detail) = goal.status.to_db();
        let blocked_by = serde_json::to_string(&goal.blocked_by).unwrap_or_else(|_| "[]".into());
        let criteria =
            serde_json::to_string(&goal.success_criteria).unwrap_or_else(|_| "[]".into());
        let deadline = goal.deadline.map(|d| d.to_rfc3339());
        let completed = goal.completed_at.map(|d| d.to_rfc3339());

        conn.execute(
            "INSERT INTO goals (id, description, priority, status, status_detail, parent_id, blocked_by, success_criteria, deadline, created_at, updated_at, completed_at, context, source, attempt, max_attempts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                goal.id,
                goal.description,
                goal.priority as i32,
                status,
                detail,
                goal.parent_id,
                blocked_by,
                criteria,
                deadline,
                goal.created_at.to_rfc3339(),
                goal.updated_at.to_rfc3339(),
                completed,
                goal.context,
                goal.source.to_db(),
                goal.attempt,
                goal.max_attempts,
            ],
        )
        .map_err(|e| zeus_core::Error::Database(format!("Failed to add goal: {}", e)))?;

        debug!(id = %goal.id, desc = %goal.description, "Goal added");
        Ok(goal.id.clone())
    }

    /// Update a goal's status
    pub fn update_status(&self, id: &str, status: GoalStatus) -> Result<()> {
        let conn = self.conn()?;
        let (state, detail) = status.to_db();
        let now = Utc::now().to_rfc3339();
        let completed = if status.is_terminal() {
            Some(now.clone())
        } else {
            None
        };

        let updated = conn
            .execute(
                "UPDATE goals SET status = ?1, status_detail = ?2, updated_at = ?3, completed_at = COALESCE(?4, completed_at) WHERE id = ?5",
                params![state, detail, now, completed, id],
            )
            .map_err(|e| zeus_core::Error::Database(format!("Failed to update goal: {}", e)))?;

        if updated == 0 {
            return Err(zeus_core::Error::NotFound(format!(
                "Goal '{}' not found",
                id
            )));
        }

        debug!(id, status = state, "Goal status updated");
        Ok(())
    }

    /// #157: durable bounded-retry enforcement.
    ///
    /// Atomically increments the goal's `attempt` counter in SQLite and, if the
    /// goal is bounded (`max_attempts > 0`) and the new count reaches the cap,
    /// marks it `Abandoned` in code. Returns `Ok(Some(new_attempt))` if the cap
    /// was reached (caller should notify), `Ok(None)` if the goal is still
    /// within bounds or unbounded.
    ///
    /// This is the load-bearing post-promotion gate: once the hot-loader
    /// promotes a goal `.md` to a SQLite row and deletes the file, there is no
    /// front-matter left to re-stamp — the cap must be enforced here, in code,
    /// per re-cook, never derived from any LLM-echoed body directive.
    pub fn bump_attempt_and_enforce(&self, id: &str) -> Result<Option<u32>> {
        let conn = self.conn()?;
        let now = Utc::now().to_rfc3339();

        // Atomic increment; read back the new value + cap in one connection.
        conn.execute(
            "UPDATE goals SET attempt = attempt + 1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )
        .map_err(|e| zeus_core::Error::Database(format!("Failed to bump attempt: {}", e)))?;

        let (attempt, max_attempts): (i64, i64) = conn
            .query_row(
                "SELECT attempt, max_attempts FROM goals WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| zeus_core::Error::Database(format!("Failed to read attempt: {}", e)))?;

        // max_attempts == 0 → unbounded (legacy rows / uncapped goals).
        if max_attempts > 0 && attempt >= max_attempts {
            let reason = format!(
                "bounded-retry cap reached: {} of {} attempts — abandoning to free the goal stack",
                attempt, max_attempts
            );
            self.update_status(id, GoalStatus::Abandoned { reason })?;
            debug!(id, attempt, max_attempts, "Goal abandoned at retry cap");
            return Ok(Some(attempt as u32));
        }

        Ok(None)
    }

    /// Get a goal by ID
    pub fn get(&self, id: &str) -> Result<Option<Goal>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT id, description, priority, status, status_detail, parent_id, blocked_by, success_criteria, deadline, created_at, updated_at, completed_at, context, source, attempt, max_attempts FROM goals WHERE id = ?1")
            .map_err(|e| zeus_core::Error::Database(format!("Failed to prepare query: {}", e)))?;

        let mut rows = stmt
            .query_map(params![id], Self::row_to_goal)
            .map_err(|e| zeus_core::Error::Database(format!("Query failed: {}", e)))?;

        match rows.next() {
            Some(Ok(goal)) => Ok(Some(goal)),
            Some(Err(e)) => Err(zeus_core::Error::Database(format!(
                "Failed to read goal: {}",
                e
            ))),
            None => Ok(None),
        }
    }

    /// Get all active (non-terminal) goals, ordered by priority descending
    pub fn active_goals(&self) -> Result<Vec<Goal>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, description, priority, status, status_detail, parent_id, blocked_by, success_criteria, deadline, created_at, updated_at, completed_at, context, source, attempt, max_attempts
                 FROM goals
                 WHERE status IN ('pending', 'active', 'blocked')
                 ORDER BY priority DESC, created_at ASC",
            )
            .map_err(|e| zeus_core::Error::Database(format!("Failed to prepare query: {}", e)))?;

        let rows = stmt
            .query_map([], Self::row_to_goal)
            .map_err(|e| zeus_core::Error::Database(format!("Query failed: {}", e)))?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| zeus_core::Error::Database(format!("Failed to collect goals: {}", e)))
    }

    /// Get the highest-priority active goal
    pub fn top_goal(&self) -> Result<Option<Goal>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, description, priority, status, status_detail, parent_id, blocked_by, success_criteria, deadline, created_at, updated_at, completed_at, context, source, attempt, max_attempts
                 FROM goals
                 WHERE status IN ('pending', 'active')
                 ORDER BY priority DESC, created_at ASC
                 LIMIT 1",
            )
            .map_err(|e| zeus_core::Error::Database(format!("Failed to prepare query: {}", e)))?;

        let mut rows = stmt
            .query_map([], Self::row_to_goal)
            .map_err(|e| zeus_core::Error::Database(format!("Query failed: {}", e)))?;

        match rows.next() {
            Some(Ok(goal)) => Ok(Some(goal)),
            Some(Err(e)) => Err(zeus_core::Error::Database(format!(
                "Failed to read goal: {}",
                e
            ))),
            None => Ok(None),
        }
    }

    /// Get child goals of a parent
    pub fn children(&self, parent_id: &str) -> Result<Vec<Goal>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, description, priority, status, status_detail, parent_id, blocked_by, success_criteria, deadline, created_at, updated_at, completed_at, context, source, attempt, max_attempts
                 FROM goals
                 WHERE parent_id = ?1
                 ORDER BY priority DESC, created_at ASC",
            )
            .map_err(|e| zeus_core::Error::Database(format!("Failed to prepare query: {}", e)))?;

        let rows = stmt
            .query_map(params![parent_id], Self::row_to_goal)
            .map_err(|e| zeus_core::Error::Database(format!("Query failed: {}", e)))?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| zeus_core::Error::Database(format!("Failed to collect children: {}", e)))
    }

    /// Mark a goal completed and unblock goals that depended on it.
    /// Returns IDs of newly unblocked goals.
    pub fn unblock(&self, completed_goal_id: &str) -> Result<Vec<String>> {
        let conn = self.conn()?;
        let now = Utc::now().to_rfc3339();

        // Find goals that have completed_goal_id in their blocked_by list
        let mut stmt = conn
            .prepare("SELECT id, blocked_by FROM goals WHERE status = 'blocked'")
            .map_err(|e| zeus_core::Error::Database(format!("Query failed: {}", e)))?;

        let blocked: Vec<(String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| zeus_core::Error::Database(format!("Query failed: {}", e)))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| zeus_core::Error::Database(format!("Row read failed: {}", e)))?;

        let mut unblocked = Vec::new();

        for (goal_id, blocked_by_json) in blocked {
            let mut deps: Vec<String> = serde_json::from_str(&blocked_by_json).unwrap_or_default();

            if deps.contains(&completed_goal_id.to_string()) {
                deps.retain(|d| d != completed_goal_id);
                let new_json = serde_json::to_string(&deps).unwrap_or_else(|_| "[]".into());

                if deps.is_empty() {
                    // Fully unblocked → set to pending
                    conn.execute(
                        "UPDATE goals SET status = 'pending', status_detail = NULL, blocked_by = ?1, updated_at = ?2 WHERE id = ?3",
                        params![new_json, now, goal_id],
                    )
                    .map_err(|e| zeus_core::Error::Database(format!("Unblock failed: {}", e)))?;
                    unblocked.push(goal_id);
                } else {
                    // Still blocked by other goals
                    conn.execute(
                        "UPDATE goals SET blocked_by = ?1, updated_at = ?2 WHERE id = ?3",
                        params![new_json, now, goal_id],
                    )
                    .map_err(|e| {
                        zeus_core::Error::Database(format!("Update deps failed: {}", e))
                    })?;
                }
            }
        }

        Ok(unblocked)
    }

    /// Delete completed/failed/abandoned goals older than the given number of days.
    /// Returns the count of pruned goals.
    pub fn prune_completed(&self, older_than_days: i64) -> Result<usize> {
        let conn = self.conn()?;
        let cutoff = (Utc::now() - chrono::Duration::days(older_than_days)).to_rfc3339();

        let deleted = conn
            .execute(
                "DELETE FROM goals WHERE status IN ('completed', 'failed', 'abandoned') AND updated_at < ?1",
                params![cutoff],
            )
            .map_err(|e| zeus_core::Error::Database(format!("Prune failed: {}", e)))?;

        debug!(pruned = deleted, "Pruned completed goals");
        Ok(deleted)
    }

    /// #173-b: Purge the live pending-goal queue.
    ///
    /// Marks every non-terminal goal (`pending`/`active`/`blocked`) as
    /// `abandoned` so a `/clear` + fresh start does not leave stuck rows that
    /// the cook loop will re-pick on the next wake. We *abandon* rather than
    /// `DELETE` so completed/failed history is preserved (the `/clear-state`
    /// loop-bug escape hatch only needs to break the resume gate, not wipe the
    /// audit trail). Returns the number of rows transitioned.
    ///
    /// This is the durable fix for the bug where `/clear` + `--fresh` cleared
    /// context/files/procs but NOT the goals.db queue — the pending row is the
    /// real terminate gate, so it must be transitioned to truly stop the loop.
    pub fn clear_pending(&self) -> Result<usize> {
        let conn = self.conn()?;
        let now = Utc::now().to_rfc3339();
        let cleared = conn
            .execute(
                "UPDATE goals \
                 SET status = 'abandoned', \
                     status_detail = 'cleared by /clear (pending-queue purge)', \
                     updated_at = ?1 \
                 WHERE status IN ('pending', 'active', 'blocked')",
                params![now],
            )
            .map_err(|e| {
                zeus_core::Error::Database(format!("clear_pending failed: {}", e))
            })?;

        debug!(cleared, "Cleared pending goal queue");
        Ok(cleared)
    }

    /// Map a SQLite row to a Goal struct
    fn row_to_goal(row: &rusqlite::Row<'_>) -> rusqlite::Result<Goal> {
        let priority_val: i32 = row.get(2)?;
        let status_str: String = row.get(3)?;
        let status_detail: Option<String> = row.get(4)?;
        let blocked_by_json: String = row.get(6)?;
        let criteria_json: String = row.get(7)?;
        let deadline_str: Option<String> = row.get(8)?;
        let created_str: String = row.get(9)?;
        let updated_str: String = row.get(10)?;
        let completed_str: Option<String> = row.get(11)?;
        let source_str: String = row.get(13)?;

        Ok(Goal {
            id: row.get(0)?,
            description: row.get(1)?,
            priority: Priority::from_i32(priority_val),
            status: GoalStatus::from_db(&status_str, status_detail),
            parent_id: row.get(5)?,
            blocked_by: serde_json::from_str(&blocked_by_json).unwrap_or_default(),
            success_criteria: serde_json::from_str(&criteria_json).unwrap_or_default(),
            deadline: deadline_str
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.into())),
            created_at: DateTime::parse_from_rfc3339(&created_str)
                .map(|d| d.into())
                .unwrap_or_else(|_| Utc::now()),
            updated_at: DateTime::parse_from_rfc3339(&updated_str)
                .map(|d| d.into())
                .unwrap_or_else(|_| Utc::now()),
            completed_at: completed_str
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|d| d.into())),
            context: row.get(12)?,
            source: GoalSource::from_db(&source_str),
            attempt: row.get::<_, i64>(14).unwrap_or(0) as u32,
            max_attempts: row.get::<_, i64>(15).unwrap_or(0) as u32,
        })
    }
}

/// #168 Phase 4b — shared `[Work State]` formatter.
///
/// One formatter, two callers (Prometheus cook path + zeus-api REST handlers)
/// so the block can't drift between the live Discord/cook path and the
/// REST/agent.run path. Same no-drift discipline as `store_active_goal_working`.
///
/// Returns `None` when there is nothing to report (no active goals, no
/// incomplete plans, no pending tasks) so callers can skip the prepend
/// entirely. The returned string is prefixed with `\n\n` so it appends
/// cleanly onto an existing system prompt.
pub fn format_work_state(
    active_goals: &[String],
    incomplete_plans: &[String],
    pending_tasks: Option<&str>,
) -> Option<String> {
    let pending_tasks = pending_tasks
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if active_goals.is_empty() && incomplete_plans.is_empty() && pending_tasks.is_none() {
        return None;
    }
    let mut work_state = String::from(
        "\n\n[Work State]\nYour current durable work state (recall this before answering status questions):\n",
    );
    if !active_goals.is_empty() {
        work_state.push_str("Active goals:\n");
        for g in active_goals {
            work_state.push_str(&format!("- {}\n", g));
        }
    }
    if !incomplete_plans.is_empty() {
        work_state.push_str("Incomplete plans:\n");
        for p in incomplete_plans {
            work_state.push_str(&format!("- {}\n", p));
        }
    }
    // #168 Phase 4: gateway-threaded pending tasks (the TaskStore-active
    // string the gateway computes but which was dead on this cook path).
    if let Some(tasks) = pending_tasks {
        work_state.push_str("Pending tasks:\n");
        work_state.push_str(tasks);
        if !tasks.ends_with('\n') {
            work_state.push('\n');
        }
    }
    Some(work_state)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_stack() -> (TempDir, GoalStack) {
        let tmp = TempDir::new().unwrap();
        let stack = GoalStack::new(tmp.path().join("goals.db")).unwrap();
        (tmp, stack)
    }

    #[test]
    fn test_goal_creation() {
        let goal = Goal::new("Test goal", Priority::Normal, GoalSource::User);
        assert_eq!(goal.description, "Test goal");
        assert_eq!(goal.priority, Priority::Normal);
        assert_eq!(goal.status, GoalStatus::Pending);
        assert!(!goal.id.is_empty());
    }

    #[test]
    fn test_goal_stack_add_and_get() {
        let (_tmp, stack) = temp_stack();
        let goal = Goal::new("Write tests", Priority::High, GoalSource::User);
        let id = stack.add(&goal).unwrap();

        let retrieved = stack.get(&id).unwrap().unwrap();
        assert_eq!(retrieved.description, "Write tests");
        assert_eq!(retrieved.priority, Priority::High);
        assert_eq!(retrieved.status, GoalStatus::Pending);
    }

    #[test]
    fn test_goal_not_found() {
        let (_tmp, stack) = temp_stack();
        let result = stack.get("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_update_status() {
        let (_tmp, stack) = temp_stack();
        let goal = Goal::new("Deploy app", Priority::Critical, GoalSource::User);
        let id = stack.add(&goal).unwrap();

        stack.update_status(&id, GoalStatus::Active).unwrap();
        let g = stack.get(&id).unwrap().unwrap();
        assert_eq!(g.status, GoalStatus::Active);

        stack
            .update_status(
                &id,
                GoalStatus::Completed {
                    outcome: "Deployed successfully".into(),
                },
            )
            .unwrap();
        let g = stack.get(&id).unwrap().unwrap();
        assert!(matches!(g.status, GoalStatus::Completed { .. }));
        assert!(g.completed_at.is_some());
    }

    #[test]
    fn test_update_nonexistent_goal() {
        let (_tmp, stack) = temp_stack();
        let result = stack.update_status("nonexistent", GoalStatus::Active);
        assert!(result.is_err());
    }

    #[test]
    fn test_priority_ordering() {
        let (_tmp, stack) = temp_stack();

        let low = Goal::new("Low priority", Priority::Low, GoalSource::User);
        let high = Goal::new("High priority", Priority::High, GoalSource::User);
        let critical = Goal::new("Critical", Priority::Critical, GoalSource::System);

        stack.add(&low).unwrap();
        stack.add(&high).unwrap();
        stack.add(&critical).unwrap();

        let active = stack.active_goals().unwrap();
        assert_eq!(active.len(), 3);
        assert_eq!(active[0].priority, Priority::Critical);
        assert_eq!(active[1].priority, Priority::High);
        assert_eq!(active[2].priority, Priority::Low);
    }

    #[test]
    fn test_top_goal() {
        let (_tmp, stack) = temp_stack();

        let low = Goal::new("Low", Priority::Low, GoalSource::User);
        let high = Goal::new("High", Priority::High, GoalSource::User);
        stack.add(&low).unwrap();
        stack.add(&high).unwrap();

        let top = stack.top_goal().unwrap().unwrap();
        assert_eq!(top.description, "High");
    }

    #[test]
    fn test_top_goal_empty() {
        let (_tmp, stack) = temp_stack();
        assert!(stack.top_goal().unwrap().is_none());
    }

    #[test]
    fn test_children() {
        let (_tmp, stack) = temp_stack();

        let parent = Goal::new("Parent goal", Priority::High, GoalSource::User);
        let parent_id = stack.add(&parent).unwrap();

        let mut child1 = Goal::new(
            "Child 1",
            Priority::Normal,
            GoalSource::Decomposition {
                parent_id: parent_id.clone(),
            },
        );
        child1.parent_id = Some(parent_id.clone());
        let mut child2 = Goal::new(
            "Child 2",
            Priority::Normal,
            GoalSource::Decomposition {
                parent_id: parent_id.clone(),
            },
        );
        child2.parent_id = Some(parent_id.clone());

        stack.add(&child1).unwrap();
        stack.add(&child2).unwrap();

        let kids = stack.children(&parent_id).unwrap();
        assert_eq!(kids.len(), 2);
    }

    #[test]
    fn test_unblock() {
        let (_tmp, stack) = temp_stack();

        let dep = Goal::new("Dependency", Priority::High, GoalSource::User);
        let dep_id = stack.add(&dep).unwrap();

        let mut blocked = Goal::new("Blocked goal", Priority::Normal, GoalSource::User);
        blocked.blocked_by = vec![dep_id.clone()];
        blocked.status = GoalStatus::Blocked {
            reason: format!("Waiting on {}", dep_id),
        };
        let blocked_id = stack.add(&blocked).unwrap();

        // Unblock
        let unblocked = stack.unblock(&dep_id).unwrap();
        assert_eq!(unblocked, vec![blocked_id.clone()]);

        // Verify it's now pending
        let g = stack.get(&blocked_id).unwrap().unwrap();
        assert_eq!(g.status, GoalStatus::Pending);
        assert!(g.blocked_by.is_empty());
    }

    #[test]
    fn test_unblock_partial() {
        let (_tmp, stack) = temp_stack();

        let dep1 = Goal::new("Dep 1", Priority::High, GoalSource::User);
        let dep1_id = stack.add(&dep1).unwrap();
        let dep2 = Goal::new("Dep 2", Priority::High, GoalSource::User);
        let dep2_id = stack.add(&dep2).unwrap();

        let mut blocked = Goal::new("Double blocked", Priority::Normal, GoalSource::User);
        blocked.blocked_by = vec![dep1_id.clone(), dep2_id.clone()];
        blocked.status = GoalStatus::Blocked {
            reason: "Waiting on deps".into(),
        };
        let blocked_id = stack.add(&blocked).unwrap();

        // Unblock one dep
        let unblocked = stack.unblock(&dep1_id).unwrap();
        assert!(unblocked.is_empty()); // Still blocked by dep2

        let g = stack.get(&blocked_id).unwrap().unwrap();
        assert_eq!(g.blocked_by, vec![dep2_id.clone()]);

        // Unblock second dep
        let unblocked = stack.unblock(&dep2_id).unwrap();
        assert_eq!(unblocked, vec![blocked_id]);
    }

    #[test]
    fn test_prune_completed() {
        let (_tmp, stack) = temp_stack();

        let mut g1 = Goal::new("Done goal", Priority::Normal, GoalSource::User);
        g1.status = GoalStatus::Completed {
            outcome: "done".into(),
        };
        stack.add(&g1).unwrap();

        // Fresh goal won't be pruned (older_than_days = 0 means prune everything older than now)
        let pruned = stack.prune_completed(30).unwrap();
        assert_eq!(pruned, 0); // Just created, not old enough

        // Active goals never get pruned
        let g2 = Goal::new("Active goal", Priority::Normal, GoalSource::User);
        stack.add(&g2).unwrap();
        let pruned = stack.prune_completed(0).unwrap();
        assert_eq!(pruned, 1); // Only completed goal pruned
    }

    #[test]
    fn test_active_goals_excludes_terminal() {
        let (_tmp, stack) = temp_stack();

        let g1 = Goal::new("Active", Priority::Normal, GoalSource::User);
        stack.add(&g1).unwrap();

        let mut g2 = Goal::new("Completed", Priority::High, GoalSource::User);
        g2.status = GoalStatus::Completed {
            outcome: "done".into(),
        };
        stack.add(&g2).unwrap();

        let mut g3 = Goal::new("Failed", Priority::High, GoalSource::User);
        g3.status = GoalStatus::Failed {
            reason: "oops".into(),
        };
        stack.add(&g3).unwrap();

        let active = stack.active_goals().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].description, "Active");
    }

    #[test]
    fn test_goal_status_serialization() {
        let statuses = vec![
            GoalStatus::Pending,
            GoalStatus::Active,
            GoalStatus::Blocked {
                reason: "waiting".into(),
            },
            GoalStatus::Completed {
                outcome: "done".into(),
            },
            GoalStatus::Failed {
                reason: "error".into(),
            },
            GoalStatus::Abandoned {
                reason: "cancelled".into(),
            },
        ];

        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let deser: GoalStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deser, status);
        }
    }

    #[test]
    fn test_goal_source_serialization() {
        let sources = vec![
            GoalSource::User,
            GoalSource::System,
            GoalSource::Decomposition {
                parent_id: "abc".into(),
            },
        ];

        for source in sources {
            let json = serde_json::to_string(&source).unwrap();
            let deser: GoalSource = serde_json::from_str(&json).unwrap();
            assert_eq!(deser, source);
        }
    }

    #[test]
    fn test_priority_ordering_enum() {
        assert!(Priority::Critical > Priority::High);
        assert!(Priority::High > Priority::Normal);
        assert!(Priority::Normal > Priority::Low);
        assert!(Priority::Low > Priority::Background);
    }

    #[test]
    fn test_success_criteria_persistence() {
        let (_tmp, stack) = temp_stack();
        let mut goal = Goal::new("Goal with criteria", Priority::Normal, GoalSource::User);
        goal.success_criteria = vec!["Tests pass".into(), "No warnings".into()];
        let id = stack.add(&goal).unwrap();

        let g = stack.get(&id).unwrap().unwrap();
        assert_eq!(g.success_criteria, vec!["Tests pass", "No warnings"]);
    }

    #[test]
    fn test_goal_default_fields() {
        let goal = Goal::new("Test", Priority::Normal, GoalSource::User);
        assert!(goal.parent_id.is_none());
        assert!(goal.blocked_by.is_empty());
        assert!(goal.success_criteria.is_empty());
        assert!(goal.deadline.is_none());
        assert!(goal.completed_at.is_none());
        assert!(goal.context.is_empty());
    }

    #[test]
    fn test_goal_status_is_terminal() {
        assert!(!GoalStatus::Pending.is_terminal());
        assert!(!GoalStatus::Active.is_terminal());
        assert!(!GoalStatus::Blocked { reason: "x".into() }.is_terminal());
        assert!(
            GoalStatus::Completed {
                outcome: "done".into()
            }
            .is_terminal()
        );
        assert!(
            GoalStatus::Failed {
                reason: "err".into()
            }
            .is_terminal()
        );
        assert!(
            GoalStatus::Abandoned {
                reason: "cancel".into()
            }
            .is_terminal()
        );
    }

    #[test]
    fn test_goal_status_to_db_roundtrip() {
        let statuses = vec![
            GoalStatus::Pending,
            GoalStatus::Active,
            GoalStatus::Blocked {
                reason: "waiting for dep".into(),
            },
            GoalStatus::Completed {
                outcome: "all done".into(),
            },
            GoalStatus::Failed {
                reason: "timeout".into(),
            },
            GoalStatus::Abandoned {
                reason: "user cancelled".into(),
            },
        ];

        for status in statuses {
            let (state, detail) = status.to_db();
            let reconstructed = GoalStatus::from_db(state, detail.map(|s| s.to_string()));
            assert_eq!(reconstructed, status);
        }
    }

    #[test]
    fn test_goal_status_from_db_unknown_state() {
        let status = GoalStatus::from_db("unknown_state", None);
        assert_eq!(status, GoalStatus::Pending);
    }

    #[test]
    fn test_goal_source_to_db_from_db_roundtrip() {
        let sources = vec![
            GoalSource::User,
            GoalSource::System,
            GoalSource::Decomposition {
                parent_id: "parent123".into(),
            },
        ];

        for source in sources {
            let db_str = source.to_db();
            let reconstructed = GoalSource::from_db(&db_str);
            assert_eq!(reconstructed, source);
        }
    }

    #[test]
    fn test_goal_source_from_db_invalid_json() {
        let source = GoalSource::from_db("not valid json");
        assert_eq!(source, GoalSource::System);
    }

    #[test]
    fn test_priority_from_i32_all_values() {
        assert_eq!(Priority::from_i32(0), Priority::Background);
        assert_eq!(Priority::from_i32(1), Priority::Low);
        assert_eq!(Priority::from_i32(2), Priority::Normal); // default
        assert_eq!(Priority::from_i32(3), Priority::High);
        assert_eq!(Priority::from_i32(4), Priority::Critical);
        // Unknown values default to Normal
        assert_eq!(Priority::from_i32(99), Priority::Normal);
        assert_eq!(Priority::from_i32(-1), Priority::Normal);
    }

    #[test]
    fn test_priority_display() {
        assert_eq!(format!("{}", Priority::Background), "background");
        assert_eq!(format!("{}", Priority::Low), "low");
        assert_eq!(format!("{}", Priority::Normal), "normal");
        assert_eq!(format!("{}", Priority::High), "high");
        assert_eq!(format!("{}", Priority::Critical), "critical");
    }

    #[test]
    fn test_goal_with_context() {
        let (_tmp, stack) = temp_stack();
        let mut goal = Goal::new("Goal with context", Priority::Normal, GoalSource::User);
        goal.context = "This is related to the deployment pipeline".to_string();
        let id = stack.add(&goal).unwrap();

        let g = stack.get(&id).unwrap().unwrap();
        assert_eq!(g.context, "This is related to the deployment pipeline");
    }

    #[test]
    fn test_active_goals_includes_blocked() {
        let (_tmp, stack) = temp_stack();

        let g1 = Goal::new("Pending goal", Priority::Normal, GoalSource::User);
        stack.add(&g1).unwrap();

        let mut g2 = Goal::new("Blocked goal", Priority::High, GoalSource::User);
        g2.status = GoalStatus::Blocked {
            reason: "waiting".into(),
        };
        stack.add(&g2).unwrap();

        let active = stack.active_goals().unwrap();
        assert_eq!(active.len(), 2);
        // Blocked goals are included in active (non-terminal) set
        assert!(active.iter().any(|g| g.description == "Blocked goal"));
    }

    #[test]
    fn test_children_empty() {
        let (_tmp, stack) = temp_stack();
        let parent = Goal::new("Lonely parent", Priority::Normal, GoalSource::User);
        let parent_id = stack.add(&parent).unwrap();

        let kids = stack.children(&parent_id).unwrap();
        assert!(kids.is_empty());
    }

    #[test]
    fn test_unblock_no_blocked_goals() {
        let (_tmp, stack) = temp_stack();
        let g = Goal::new("Regular goal", Priority::Normal, GoalSource::User);
        let id = stack.add(&g).unwrap();

        let unblocked = stack.unblock(&id).unwrap();
        assert!(unblocked.is_empty());
    }

    #[test]
    fn test_goal_with_empty_criteria() {
        let (_tmp, stack) = temp_stack();
        let mut goal = Goal::new("No criteria goal", Priority::Normal, GoalSource::User);
        goal.success_criteria = vec![];
        let id = stack.add(&goal).unwrap();

        let g = stack.get(&id).unwrap().unwrap();
        assert!(g.success_criteria.is_empty());
    }

    #[test]
    fn test_goal_status_transition_pending_to_active() {
        let (_tmp, stack) = temp_stack();
        let goal = Goal::new("Transition test", Priority::Normal, GoalSource::User);
        let id = stack.add(&goal).unwrap();

        // Verify initial status is Pending
        let g = stack.get(&id).unwrap().unwrap();
        assert_eq!(g.status, GoalStatus::Pending);

        // Transition to Active
        stack.update_status(&id, GoalStatus::Active).unwrap();
        let g = stack.get(&id).unwrap().unwrap();
        assert_eq!(g.status, GoalStatus::Active);
        // Active is not terminal, so completed_at should still be None
        assert!(g.completed_at.is_none());
    }

    #[test]
    fn test_goal_status_transition_active_to_completed() {
        let (_tmp, stack) = temp_stack();
        let goal = Goal::new("Complete me", Priority::High, GoalSource::User);
        let id = stack.add(&goal).unwrap();

        stack.update_status(&id, GoalStatus::Active).unwrap();
        stack
            .update_status(
                &id,
                GoalStatus::Completed {
                    outcome: "Successfully finished".into(),
                },
            )
            .unwrap();

        let g = stack.get(&id).unwrap().unwrap();
        assert!(matches!(g.status, GoalStatus::Completed { .. }));
        if let GoalStatus::Completed { outcome } = &g.status {
            assert_eq!(outcome, "Successfully finished");
        }
        assert!(g.completed_at.is_some());
    }

    #[test]
    fn test_goal_status_transition_active_to_failed() {
        let (_tmp, stack) = temp_stack();
        let goal = Goal::new("Fail me", Priority::Normal, GoalSource::User);
        let id = stack.add(&goal).unwrap();

        stack.update_status(&id, GoalStatus::Active).unwrap();
        stack
            .update_status(
                &id,
                GoalStatus::Failed {
                    reason: "Out of memory".into(),
                },
            )
            .unwrap();

        let g = stack.get(&id).unwrap().unwrap();
        assert!(matches!(g.status, GoalStatus::Failed { .. }));
        if let GoalStatus::Failed { reason } = &g.status {
            assert_eq!(reason, "Out of memory");
        }
        assert!(g.completed_at.is_some());
    }

    #[test]
    fn test_prune_completed_keeps_recent() {
        let (_tmp, stack) = temp_stack();

        // Add a completed goal (just created, so it's recent)
        let mut g = Goal::new("Just completed", Priority::Normal, GoalSource::User);
        g.status = GoalStatus::Completed {
            outcome: "done".into(),
        };
        let id = stack.add(&g).unwrap();

        // Prune with 30 days threshold - recently completed should NOT be pruned
        let pruned = stack.prune_completed(30).unwrap();
        assert_eq!(pruned, 0);

        // Goal should still exist
        let retrieved = stack.get(&id).unwrap();
        assert!(retrieved.is_some());
    }

    #[test]
    fn test_clear_pending_abandons_live_queue_keeps_history() {
        let (_tmp, stack) = temp_stack();

        // Three live (non-terminal) goals + one completed (history).
        let pending = stack.add(&Goal::new("pending one", Priority::Normal, GoalSource::User)).unwrap();
        let mut active = Goal::new("active one", Priority::High, GoalSource::System);
        active.status = GoalStatus::Active;
        let active_id = stack.add(&active).unwrap();
        let mut blocked = Goal::new("blocked one", Priority::Low, GoalSource::User);
        blocked.status = GoalStatus::Blocked { reason: "dep".into() };
        let blocked_id = stack.add(&blocked).unwrap();
        let mut done = Goal::new("done one", Priority::Normal, GoalSource::User);
        done.status = GoalStatus::Completed { outcome: "shipped".into() };
        let done_id = stack.add(&done).unwrap();

        // Sanity: 3 live goals before clear.
        assert_eq!(stack.active_goals().unwrap().len(), 3);

        // Clear the pending queue.
        let cleared = stack.clear_pending().unwrap();
        assert_eq!(cleared, 3, "all 3 live goals should be cleared");

        // No live goals remain — the resume gate is broken.
        assert!(stack.active_goals().unwrap().is_empty());

        // Live rows transitioned to Abandoned (history preserved, not deleted).
        for id in [&pending, &active_id, &blocked_id] {
            let g = stack.get(id).unwrap().unwrap();
            assert!(matches!(g.status, GoalStatus::Abandoned { .. }), "live goal must be abandoned");
        }
        // Completed history untouched.
        let d = stack.get(&done_id).unwrap().unwrap();
        assert!(matches!(d.status, GoalStatus::Completed { .. }), "completed history must survive");

        // Idempotent: a second clear finds nothing left to transition.
        assert_eq!(stack.clear_pending().unwrap(), 0);
    }

    /// #173-b: exercises the exact idiom the `--fresh` gateway-restart block uses
    /// (`GoalStack::new(&db).and_then(|s| s.clear_pending())`) against an on-disk
    /// db that is *reopened* cold — simulating a process restart, where the
    /// pending rows are the most-persistent surface that survives both `/clear`
    /// and `--fresh`. The primitive test above uses an already-open handle; this
    /// proves the cold-open chain that main.rs:613 relies on.
    #[test]
    fn test_fresh_restart_reopen_clears_pending_keeps_history() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("goals.db");

        // Seed: 2 live + 1 completed, then drop the handle (process "exits").
        let (pending_id, active_id, done_id) = {
            let stack = GoalStack::new(&db).unwrap();
            let p = stack
                .add(&Goal::new("pending survives restart", Priority::Normal, GoalSource::User))
                .unwrap();
            let mut act = Goal::new("active survives restart", Priority::High, GoalSource::System);
            act.status = GoalStatus::Active;
            let a = stack.add(&act).unwrap();
            let mut done = Goal::new("done history", Priority::Normal, GoalSource::User);
            done.status = GoalStatus::Completed { outcome: "shipped".into() };
            let d = stack.add(&done).unwrap();
            assert_eq!(stack.active_goals().unwrap().len(), 2);
            (p, a, d)
        };

        // `--fresh` cold-open + clear, the exact main.rs:613 chain.
        let cleared = GoalStack::new(&db)
            .and_then(|s| s.clear_pending())
            .unwrap();
        assert_eq!(cleared, 2, "both live goals cleared on cold-open --fresh");

        // Reopen again to verify persistence of the abandon (file preserved,
        // not deleted — abandon-not-delete).
        let stack = GoalStack::new(&db).unwrap();
        assert_eq!(stack.active_goals().unwrap().len(), 0, "no live goals after --fresh");
        for id in [&pending_id, &active_id] {
            let g = stack.get(id).unwrap().unwrap();
            assert!(
                matches!(g.status, GoalStatus::Abandoned { .. }),
                "live goals abandoned, not deleted"
            );
        }
        // Completed history must survive the --fresh purge.
        let d = stack.get(&done_id).unwrap().unwrap();
        assert!(
            matches!(d.status, GoalStatus::Completed { .. }),
            "completed history preserved across --fresh"
        );

        // Idempotent: a second --fresh cold-open finds nothing to clear.
        assert_eq!(
            GoalStack::new(&db).and_then(|s| s.clear_pending()).unwrap(),
            0,
            "--fresh is idempotent"
        );
    }

    #[test]
    fn test_multiple_top_goals_same_priority() {
        let (_tmp, stack) = temp_stack();

        let g1 = Goal::new("Critical A", Priority::Critical, GoalSource::User);
        let g2 = Goal::new("Critical B", Priority::Critical, GoalSource::System);

        stack.add(&g1).unwrap();
        // Small delay is not needed; created_at ordering handles it
        stack.add(&g2).unwrap();

        let active = stack.active_goals().unwrap();
        assert_eq!(active.len(), 2);
        // Both should be Critical priority
        assert_eq!(active[0].priority, Priority::Critical);
        assert_eq!(active[1].priority, Priority::Critical);

        // top_goal should return the first one created (ordered by created_at ASC)
        let top = stack.top_goal().unwrap().unwrap();
        assert_eq!(top.priority, Priority::Critical);
    }

    // ── #157: durable bounded-retry counter ──────────────────────────────

    #[test]
    fn test_bounded_retry_caps_and_abandons_in_code() {
        let (_tmp, stack) = temp_stack();
        let mut g = Goal::new("bounded loop", Priority::Normal, GoalSource::System);
        g.max_attempts = 3;
        let id = stack.add(&g).unwrap();

        // attempts 1 and 2 are within bounds → no abandon
        assert_eq!(stack.bump_attempt_and_enforce(&id).unwrap(), None);
        assert_eq!(stack.bump_attempt_and_enforce(&id).unwrap(), None);
        // goal still selectable
        assert!(stack.top_goal().unwrap().is_some());

        // attempt 3 reaches the cap → Some(3), abandoned in code
        assert_eq!(stack.bump_attempt_and_enforce(&id).unwrap(), Some(3));
        let g2 = stack.get(&id).unwrap().unwrap();
        assert!(matches!(g2.status, GoalStatus::Abandoned { .. }));
        // Abandoned drops out of top_goal (WHERE status IN ('pending','active'))
        assert!(stack.top_goal().unwrap().is_none());
    }

    #[test]
    fn test_unbounded_goal_never_capped() {
        let (_tmp, stack) = temp_stack();
        // max_attempts == 0 (default) → unbounded
        let g = Goal::new("uncapped", Priority::Normal, GoalSource::System);
        let id = stack.add(&g).unwrap();
        for _ in 0..50 {
            assert_eq!(stack.bump_attempt_and_enforce(&id).unwrap(), None);
        }
        let g2 = stack.get(&id).unwrap().unwrap();
        assert!(matches!(g2.status, GoalStatus::Pending));
        assert_eq!(g2.attempt, 50);
    }

    #[test]
    fn test_attempt_counter_persists_across_reload() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("goals.db");
        let id = {
            let stack = GoalStack::new(&db).unwrap();
            let mut g = Goal::new("survives restart", Priority::Normal, GoalSource::System);
            g.max_attempts = 5;
            let id = stack.add(&g).unwrap();
            stack.bump_attempt_and_enforce(&id).unwrap();
            stack.bump_attempt_and_enforce(&id).unwrap();
            id
        };
        // Reopen the DB (simulates a process restart) — the counter must survive,
        // unlike the in-memory #156 goal_noop_counts HashMap.
        let stack2 = GoalStack::new(&db).unwrap();
        let g = stack2.get(&id).unwrap().unwrap();
        assert_eq!(g.attempt, 2);
        assert_eq!(g.max_attempts, 5);
        // One more bump to the cap to confirm enforcement holds post-reload.
        stack2.bump_attempt_and_enforce(&id).unwrap();
        stack2.bump_attempt_and_enforce(&id).unwrap();
        assert_eq!(stack2.bump_attempt_and_enforce(&id).unwrap(), Some(5));
        let g2 = stack2.get(&id).unwrap().unwrap();
        assert!(matches!(g2.status, GoalStatus::Abandoned { .. }));
    }
}
