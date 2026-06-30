//! Task Flows - Durable multi-step flows with an explicit state machine.
//!
//! A `FlowRun` is a persisted, restart-surviving sequence of steps that move
//! through an explicit lifecycle: `pending → running → done | blocked`. Unlike
//! a one-shot task or a `StandingOrder` (an open-ended directive), a flow has
//! ordered steps and a deterministic state machine so the gateway can resume a
//! half-finished flow after a restart instead of dropping it.
//!
//! Mirrors the store/schema/migration idiom of `standing_orders.rs`:
//! a SQLite-backed store opened (and migrated) via `crate::db::run_migrations`.
//!
//! State machine:
//!
//! ```text
//!   pending ──start──▶ running ──finish──▶ done
//!      │                  │
//!      └──────────────────┴──block──▶ blocked ──unblock──▶ pending
//! ```
//!
//! A flow in `blocked` is waiting on an external dependency; `unblock` returns
//! it to `pending` so it can be picked up again. `done` is terminal.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::debug;
use zeus_core::Result;

// ============================================================================
// Types
// ============================================================================

/// Lifecycle state of a flow run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowStatus {
    /// Not yet started — eligible to be picked up.
    Pending,
    /// Actively executing.
    Running,
    /// Finished successfully — terminal.
    Done,
    /// Waiting on an external dependency; returns to `Pending` via `unblock`.
    Blocked,
}

impl FlowStatus {
    fn as_str(&self) -> &'static str {
        match self {
            FlowStatus::Pending => "pending",
            FlowStatus::Running => "running",
            FlowStatus::Done => "done",
            FlowStatus::Blocked => "blocked",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "running" => FlowStatus::Running,
            "done" => FlowStatus::Done,
            "blocked" => FlowStatus::Blocked,
            _ => FlowStatus::Pending,
        }
    }

    /// Whether a transition from `self` to `next` is legal.
    ///
    /// ```text
    ///   pending  → running, blocked
    ///   running  → done, blocked
    ///   blocked  → pending
    ///   done     → (terminal)
    /// ```
    fn can_transition_to(self, next: FlowStatus) -> bool {
        use FlowStatus::*;
        matches!(
            (self, next),
            (Pending, Running)
                | (Pending, Blocked)
                | (Running, Done)
                | (Running, Blocked)
                | (Blocked, Pending)
        )
    }
}

impl std::fmt::Display for FlowStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A single ordered step within a flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowStep {
    /// 0-based position in the flow.
    pub idx: usize,
    /// Human-readable description of what this step does.
    pub description: String,
    /// Whether this step has been completed.
    pub done: bool,
}

impl FlowStep {
    pub fn new(idx: usize, description: impl Into<String>) -> Self {
        Self {
            idx,
            description: description.into(),
            done: false,
        }
    }
}

/// A durable, restart-surviving flow run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowRun {
    pub id: String,
    /// Human-readable name of the flow.
    pub name: String,
    pub status: FlowStatus,
    /// Ordered steps; serialized as JSON in the `steps` column.
    pub steps: Vec<FlowStep>,
    /// Index of the next step to execute.
    pub cursor: usize,
    /// Free-form notes (e.g. why blocked).
    pub notes: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl FlowRun {
    pub fn new(name: impl Into<String>, steps: Vec<FlowStep>) -> Self {
        let now = Utc::now();
        Self {
            id: ulid::Ulid::new().to_string(),
            name: name.into(),
            status: FlowStatus::Pending,
            steps,
            cursor: 0,
            notes: String::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Whether all steps are done.
    pub fn is_complete(&self) -> bool {
        self.cursor >= self.steps.len()
    }
}

// ============================================================================
// Store
// ============================================================================

const FLOW_MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE IF NOT EXISTS task_flows (
        id           TEXT PRIMARY KEY,
        name         TEXT NOT NULL,
        status       TEXT NOT NULL,
        steps        TEXT NOT NULL,
        cursor       INTEGER NOT NULL,
        notes        TEXT NOT NULL DEFAULT '',
        created_at   TEXT NOT NULL,
        updated_at   TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_task_flows_status
        ON task_flows(status);
    "#,
];

/// SQLite-backed store for task flows.
pub struct FlowStore {
    path: PathBuf,
}

impl FlowStore {
    /// Open (and migrate) the store at the given path.
    pub fn new(db_path: impl Into<PathBuf>) -> Result<Self> {
        let path = db_path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let store = Self { path };
        store.init()?;
        Ok(store)
    }

    /// Default location: `~/.zeus/task_flows.db`.
    pub fn default_path() -> Result<Self> {
        let path = dirs::home_dir()
            .ok_or_else(|| zeus_core::Error::Internal("no home directory".into()))?
            .join(".zeus/task_flows.db");
        Self::new(path)
    }

    fn init(&self) -> Result<()> {
        let conn = self.conn()?;
        crate::db::run_migrations(&conn, FLOW_MIGRATIONS)?;
        Ok(())
    }

    fn conn(&self) -> Result<Connection> {
        Connection::open(&self.path).map_err(|e| {
            zeus_core::Error::Database(format!("Failed to open task_flows db: {}", e))
        })
    }

    /// Insert a new flow run. Returns its id.
    pub fn add(&self, flow: &FlowRun) -> Result<String> {
        let conn = self.conn()?;
        let steps_json = serde_json::to_string(&flow.steps)
            .map_err(|e| zeus_core::Error::Internal(format!("serialize steps: {}", e)))?;
        conn.execute(
            "INSERT INTO task_flows
                (id, name, status, steps, cursor, notes, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                flow.id,
                flow.name,
                flow.status.as_str(),
                steps_json,
                flow.cursor as i64,
                flow.notes,
                flow.created_at.to_rfc3339(),
                flow.updated_at.to_rfc3339(),
            ],
        )
        .map_err(|e| zeus_core::Error::Database(e.to_string()))?;
        debug!(id = %flow.id, name = %flow.name, "task flow added");
        Ok(flow.id.clone())
    }

    /// Fetch a flow by id.
    pub fn get(&self, id: &str) -> Result<Option<FlowRun>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, status, steps, cursor, notes, created_at, updated_at
                 FROM task_flows WHERE id = ?1",
            )
            .map_err(|e| zeus_core::Error::Database(e.to_string()))?;
        let mut rows = stmt
            .query_map(params![id], Self::map_row)
            .map_err(|e| zeus_core::Error::Database(e.to_string()))?;
        match rows.next() {
            Some(r) => Ok(Some(r.map_err(|e| zeus_core::Error::Database(e.to_string()))?)),
            None => Ok(None),
        }
    }

    /// List flows, optionally filtered by status, ordered oldest-first.
    pub fn list(&self, status: Option<FlowStatus>) -> Result<Vec<FlowRun>> {
        let conn = self.conn()?;
        let (sql, bind): (&str, Option<&'static str>) = match status {
            Some(s) => (
                "SELECT id, name, status, steps, cursor, notes, created_at, updated_at
                 FROM task_flows WHERE status = ?1 ORDER BY created_at ASC",
                Some(s.as_str()),
            ),
            None => (
                "SELECT id, name, status, steps, cursor, notes, created_at, updated_at
                 FROM task_flows ORDER BY created_at ASC",
                None,
            ),
        };
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| zeus_core::Error::Database(e.to_string()))?;
        let mapped = if let Some(s) = bind {
            stmt.query_map(params![s], Self::map_row)
        } else {
            stmt.query_map([], Self::map_row)
        }
        .map_err(|e| zeus_core::Error::Database(e.to_string()))?;
        let mut out = Vec::new();
        for r in mapped {
            out.push(r.map_err(|e| zeus_core::Error::Database(e.to_string()))?);
        }
        Ok(out)
    }

    fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FlowRun> {
        let steps_json: String = row.get(3)?;
        let steps: Vec<FlowStep> = serde_json::from_str(&steps_json).unwrap_or_default();
        let created: String = row.get(6)?;
        let updated: String = row.get(7)?;
        Ok(FlowRun {
            id: row.get(0)?,
            name: row.get(1)?,
            status: FlowStatus::from_str(&row.get::<_, String>(2)?),
            steps,
            cursor: row.get::<_, i64>(4)? as usize,
            notes: row.get(5)?,
            created_at: DateTime::parse_from_rfc3339(&created)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: DateTime::parse_from_rfc3339(&updated)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }

    /// Persist a status transition, enforcing the state machine. On illegal
    /// transition this returns an error and the row is untouched.
    fn transition(&self, id: &str, next: FlowStatus) -> Result<FlowRun> {
        let mut flow = self.get(id)?.ok_or_else(|| {
            zeus_core::Error::NotFound(format!("task flow {} not found", id))
        })?;
        if !flow.status.can_transition_to(next) {
            return Err(zeus_core::Error::Internal(format!(
                "illegal flow transition {} → {} for {}",
                flow.status, next, id
            )));
        }
        flow.status = next;
        flow.updated_at = Utc::now();
        self.persist_state(&flow)?;
        Ok(flow)
    }

    fn persist_state(&self, flow: &FlowRun) -> Result<()> {
        let conn = self.conn()?;
        let steps_json = serde_json::to_string(&flow.steps)
            .map_err(|e| zeus_core::Error::Internal(format!("serialize steps: {}", e)))?;
        let n = conn
            .execute(
                "UPDATE task_flows
                 SET status = ?1, steps = ?2, cursor = ?3, notes = ?4, updated_at = ?5
                 WHERE id = ?6",
                params![
                    flow.status.as_str(),
                    steps_json,
                    flow.cursor as i64,
                    flow.notes,
                    flow.updated_at.to_rfc3339(),
                    flow.id,
                ],
            )
            .map_err(|e| zeus_core::Error::Database(e.to_string()))?;
        if n == 0 {
            return Err(zeus_core::Error::NotFound(format!(
                "task flow {} not found",
                flow.id
            )));
        }
        Ok(())
    }

    /// `pending → running`.
    pub fn start(&self, id: &str) -> Result<FlowRun> {
        self.transition(id, FlowStatus::Running)
    }

    /// `running → done`. Caller should ensure all steps are complete.
    pub fn finish(&self, id: &str) -> Result<FlowRun> {
        self.transition(id, FlowStatus::Done)
    }

    /// `pending|running → blocked`, recording why.
    pub fn block(&self, id: &str, reason: impl Into<String>) -> Result<FlowRun> {
        let mut flow = self.transition(id, FlowStatus::Blocked)?;
        flow.notes = reason.into();
        flow.updated_at = Utc::now();
        self.persist_state(&flow)?;
        Ok(flow)
    }

    /// `blocked → pending`.
    pub fn unblock(&self, id: &str) -> Result<FlowRun> {
        self.transition(id, FlowStatus::Pending)
    }

    /// Advance the cursor by completing the current step. When the cursor
    /// reaches the end the flow is auto-finished (`running → done`).
    pub fn advance(&self, id: &str) -> Result<FlowRun> {
        let mut flow = self.get(id)?.ok_or_else(|| {
            zeus_core::Error::NotFound(format!("task flow {} not found", id))
        })?;
        if flow.status != FlowStatus::Running {
            return Err(zeus_core::Error::Internal(format!(
                "cannot advance flow {} in state {}",
                id, flow.status
            )));
        }
        if let Some(step) = flow.steps.get_mut(flow.cursor) {
            step.done = true;
        }
        flow.cursor += 1;
        flow.updated_at = Utc::now();
        if flow.is_complete() {
            flow.status = FlowStatus::Done;
        }
        self.persist_state(&flow)?;
        Ok(flow)
    }

    /// Delete a flow permanently.
    pub fn remove(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        let n = conn
            .execute("DELETE FROM task_flows WHERE id = ?1", params![id])
            .map_err(|e| zeus_core::Error::Database(e.to_string()))?;
        if n == 0 {
            return Err(zeus_core::Error::NotFound(format!(
                "task flow {} not found",
                id
            )));
        }
        Ok(())
    }
}

// ============================================================================
// Run engine
// ============================================================================

/// Outcome of one `run_due` tick.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RunReport {
    /// Flows that were `pending` and got `started` this tick.
    pub started: usize,
    /// Total step `advance`s applied across all driven flows this tick.
    pub steps_advanced: usize,
    /// Flows that reached `done` (terminal) this tick.
    pub completed: usize,
}

impl RunReport {
    /// Whether this tick did any work at all.
    pub fn is_idle(&self) -> bool {
        self.started == 0 && self.steps_advanced == 0 && self.completed == 0
    }
}

impl FlowStore {
    /// Drive every `pending` flow through the state machine to completion.
    ///
    /// This is the execution half of the task-flow subsystem — the piece that
    /// makes a `FlowRun` actually *run* rather than just persist. For each
    /// `pending` flow it:
    ///
    /// 1. **`start`** — `pending → running` (transition-guarded by the store).
    /// 2. **`advance`** — drive the step cursor forward one step at a time; the
    ///    store auto-flips `running → done` when the last step lands (see
    ///    [`FlowRun::is_complete`]).
    ///
    /// `blocked` flows are skipped (they await an external `unblock`); `running`
    /// flows left half-finished by a prior restart are *resumed* — this is the
    /// restart-survival guarantee the durable store buys us. Each `advance`
    /// re-reads + re-persists state, so a crash mid-tick resumes cleanly on the
    /// next tick from the last persisted cursor.
    ///
    /// Returns a [`RunReport`] tallying the work done so the caller (the
    /// scheduler tick arm) can surface a human-readable summary.
    pub fn run_due(&self) -> Result<RunReport> {
        let mut report = RunReport::default();

        // Resume any flow already `running` (restart-survival) plus pick up
        // every `pending` flow. Order is stable by creation (list() is ULID-
        // ordered), so flows fire oldest-first.
        let pending = self.list(Some(FlowStatus::Pending))?;
        let running = self.list(Some(FlowStatus::Running))?;

        for flow in pending {
            // pending → running. Guard against a race where the row changed
            // status between list() and start(): treat a rejected transition as
            // "someone else took it" and skip rather than abort the whole tick.
            if self.start(&flow.id).is_err() {
                continue;
            }
            report.started += 1;
            self.drive_to_done(&flow.id, &mut report)?;
        }

        for flow in running {
            // Resume a flow left mid-run by a prior restart from its persisted
            // cursor — no `start` (already running), just keep advancing.
            self.drive_to_done(&flow.id, &mut report)?;
        }

        Ok(report)
    }

    /// Advance a single `running` flow step-by-step until it is `done` (or no
    /// longer `running` — e.g. an external `block` slipped in). Tallies into
    /// `report`. Each step is persisted independently for crash-resume.
    fn drive_to_done(&self, id: &str, report: &mut RunReport) -> Result<()> {
        loop {
            let flow = match self.get(id)? {
                Some(f) => f,
                None => return Ok(()), // removed mid-drive — nothing to do.
            };
            if flow.status != FlowStatus::Running {
                // done | blocked | (raced) — stop driving this flow.
                if flow.status == FlowStatus::Done {
                    report.completed += 1;
                }
                return Ok(());
            }
            let after = self.advance(id)?;
            report.steps_advanced += 1;
            if after.status == FlowStatus::Done {
                report.completed += 1;
                return Ok(());
            }
        }
    }

    /// Run all due flows and surface a one-line summary into the workspace note,
    /// mirroring `standing_orders::surface_stale` / `commitments::deliver_due`.
    ///
    /// This is the delivery loop the scheduler's `SubsystemTick { kind:
    /// "task-flows" }` arm calls on cadence. Returns the number of flows that
    /// reached `done` this tick (0 ⇒ nothing ran, no note written).
    pub async fn tick(&self, workspace: &zeus_memory::Workspace) -> Result<usize> {
        let report = self.run_due()?;
        if report.is_idle() {
            return Ok(0);
        }

        workspace
            .note(&format!(
                "⚙️ task-flows: started {}, advanced {} step(s), completed {}",
                report.started, report.steps_advanced, report.completed
            ))
            .await?;

        debug!(
            started = report.started,
            steps_advanced = report.steps_advanced,
            completed = report.completed,
            "task-flows: ran due batch on cadence"
        );
        Ok(report.completed)
    }

    /// Scheduler task config emitting a `SubsystemTick { kind: "task-flows" }`
    /// on `cadence_cron`, mirroring `StandingOrderConfig::scheduler_tasks`.
    pub fn scheduler_tasks(cadence_cron: &str) -> Vec<crate::scheduler::TaskConfig> {
        vec![crate::scheduler::TaskConfig {
            name: "task-flows:run-due".to_string(),
            cron: cadence_cron.to_string(),
            task_type: crate::scheduler::TaskType::SubsystemTick {
                kind: "task-flows".to_string(),
            },
            enabled: true,
            run_at: None,
            run_once: false,
            wake_mode: crate::scheduler::WakeMode::Now,
        delivery_mode: crate::scheduler::DeliveryMode::Channel,
        }]
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (FlowStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = FlowStore::new(dir.path().join("flows.db")).unwrap();
        (store, dir)
    }

    fn sample_flow() -> FlowRun {
        FlowRun::new(
            "deploy",
            vec![
                FlowStep::new(0, "build"),
                FlowStep::new(1, "test"),
                FlowStep::new(2, "ship"),
            ],
        )
    }

    #[test]
    fn add_get_list_remove_roundtrip() {
        let (store, _d) = temp_store();
        let flow = sample_flow();
        let id = store.add(&flow).unwrap();

        let got = store.get(&id).unwrap().expect("flow exists");
        assert_eq!(got.name, "deploy");
        assert_eq!(got.steps.len(), 3);
        assert_eq!(got.status, FlowStatus::Pending);

        let all = store.list(None).unwrap();
        assert_eq!(all.len(), 1);
        let pending = store.list(Some(FlowStatus::Pending)).unwrap();
        assert_eq!(pending.len(), 1);
        let done = store.list(Some(FlowStatus::Done)).unwrap();
        assert_eq!(done.len(), 0);

        store.remove(&id).unwrap();
        assert!(store.get(&id).unwrap().is_none());
    }

    #[test]
    fn happy_path_pending_running_done() {
        let (store, _d) = temp_store();
        let id = store.add(&sample_flow()).unwrap();

        let f = store.start(&id).unwrap();
        assert_eq!(f.status, FlowStatus::Running);

        // advance through all three steps; last advance auto-finishes.
        let f = store.advance(&id).unwrap();
        assert_eq!(f.cursor, 1);
        assert_eq!(f.status, FlowStatus::Running);
        assert!(f.steps[0].done);

        store.advance(&id).unwrap();
        let f = store.advance(&id).unwrap();
        assert_eq!(f.cursor, 3);
        assert!(f.is_complete());
        assert_eq!(f.status, FlowStatus::Done, "auto-finishes at end of steps");
    }

    #[test]
    fn explicit_finish_marks_done() {
        let (store, _d) = temp_store();
        let id = store.add(&sample_flow()).unwrap();
        store.start(&id).unwrap();
        let f = store.finish(&id).unwrap();
        assert_eq!(f.status, FlowStatus::Done);
    }

    #[test]
    fn block_then_unblock_returns_to_pending() {
        let (store, _d) = temp_store();
        let id = store.add(&sample_flow()).unwrap();
        store.start(&id).unwrap();

        let f = store.block(&id, "waiting on upstream review").unwrap();
        assert_eq!(f.status, FlowStatus::Blocked);
        assert_eq!(f.notes, "waiting on upstream review");

        let f = store.unblock(&id).unwrap();
        assert_eq!(f.status, FlowStatus::Pending);
    }

    #[test]
    fn illegal_transition_is_rejected_and_row_untouched() {
        let (store, _d) = temp_store();
        let id = store.add(&sample_flow()).unwrap();

        // pending → done is illegal (must go through running).
        let err = store.finish(&id);
        assert!(err.is_err(), "pending → done must be rejected");

        // row remains pending.
        assert_eq!(store.get(&id).unwrap().unwrap().status, FlowStatus::Pending);
    }

    #[test]
    fn done_is_terminal() {
        let (store, _d) = temp_store();
        let id = store.add(&sample_flow()).unwrap();
        store.start(&id).unwrap();
        store.finish(&id).unwrap();

        // any transition out of done is illegal.
        assert!(store.start(&id).is_err());
        assert!(store.block(&id, "x").is_err());
    }

    #[test]
    fn cannot_advance_unless_running() {
        let (store, _d) = temp_store();
        let id = store.add(&sample_flow()).unwrap();
        // still pending → advance rejected.
        assert!(store.advance(&id).is_err());
    }

    // ---- run engine ----

    #[test]
    fn run_due_drives_pending_flow_to_done() {
        let (store, _d) = temp_store();
        let id = store.add(&sample_flow()).unwrap(); // 3 steps, pending

        let report = store.run_due().unwrap();
        assert_eq!(report.started, 1);
        assert_eq!(report.steps_advanced, 3, "all 3 steps advanced");
        assert_eq!(report.completed, 1);

        // flow is terminal and every step is marked done.
        let f = store.get(&id).unwrap().unwrap();
        assert_eq!(f.status, FlowStatus::Done);
        assert_eq!(f.cursor, 3);
        assert!(f.steps.iter().all(|s| s.done));
    }

    #[test]
    fn run_due_is_idle_with_no_pending() {
        let (store, _d) = temp_store();
        // no flows at all.
        assert!(store.run_due().unwrap().is_idle());

        // a single done flow contributes no work on the next tick.
        let id = store.add(&sample_flow()).unwrap();
        store.run_due().unwrap(); // drives it to done
        assert!(store.run_due().unwrap().is_idle(), "done flow re-ticks idle");
        assert_eq!(store.get(&id).unwrap().unwrap().status, FlowStatus::Done);
    }

    #[test]
    fn run_due_skips_blocked_flow() {
        let (store, _d) = temp_store();
        let id = store.add(&sample_flow()).unwrap();
        store.start(&id).unwrap();
        store.block(&id, "waiting on dep").unwrap();

        // blocked flow must not be driven.
        let report = store.run_due().unwrap();
        assert!(report.is_idle(), "blocked flow is skipped");
        assert_eq!(store.get(&id).unwrap().unwrap().status, FlowStatus::Blocked);

        // unblock returns it to pending; next tick drives it to done.
        store.unblock(&id).unwrap();
        let report = store.run_due().unwrap();
        assert_eq!(report.completed, 1);
        assert_eq!(store.get(&id).unwrap().unwrap().status, FlowStatus::Done);
    }

    #[test]
    fn run_due_resumes_running_flow_from_cursor() {
        let (store, _d) = temp_store();
        let id = store.add(&sample_flow()).unwrap();
        // simulate a restart mid-run: started + one step advanced, then crash.
        store.start(&id).unwrap();
        store.advance(&id).unwrap(); // cursor now 1, still running
        assert_eq!(store.get(&id).unwrap().unwrap().status, FlowStatus::Running);

        // a fresh tick resumes the running flow from its persisted cursor.
        let report = store.run_due().unwrap();
        assert_eq!(report.started, 0, "already running — not re-started");
        assert_eq!(report.steps_advanced, 2, "remaining 2 steps advanced");
        assert_eq!(report.completed, 1);
        assert_eq!(store.get(&id).unwrap().unwrap().status, FlowStatus::Done);
    }

    #[test]
    fn run_due_drives_multiple_pending_flows() {
        let (store, _d) = temp_store();
        store.add(&sample_flow()).unwrap();
        store.add(&FlowRun::new("other", vec![FlowStep::new(0, "only")])).unwrap();

        let report = store.run_due().unwrap();
        assert_eq!(report.started, 2);
        assert_eq!(report.steps_advanced, 4, "3 + 1 steps");
        assert_eq!(report.completed, 2);
        assert_eq!(store.list(Some(FlowStatus::Done)).unwrap().len(), 2);
    }

    #[test]
    fn scheduler_tasks_emits_subsystem_tick() {
        let tasks = FlowStore::scheduler_tasks("0 */5 * * * *");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "task-flows:run-due");
        assert_eq!(tasks[0].cron, "0 */5 * * * *");
        assert!(tasks[0].enabled);
        match &tasks[0].task_type {
            crate::scheduler::TaskType::SubsystemTick { kind } => {
                assert_eq!(kind, "task-flows");
            }
            other => panic!("expected SubsystemTick, got {:?}", other),
        }
    }
}
