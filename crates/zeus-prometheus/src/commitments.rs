//! Commitments — inferred follow-ups the agent honors across sessions.
//!
//! When a conversation implies a time-bound promise ("I'll check the build
//! after lunch", "ping me when the deploy's green"), the agent extracts it as
//! a [`Commitment`], stores it with a due-window, and the heartbeat delivers it
//! back when due — so promises don't evaporate at session end.
//!
//! This is a sibling of [`crate::standing_orders`]: same proven SQLite store
//! skeleton, different semantics. Standing orders are durable directives the
//! *human* sets; commitments are follow-ups the *agent* infers and must remember
//! to honor.
//!
//! Mirrors OpenClaw's `CommitmentsConfig { enabled, maxPerDay }` + commitment
//! store + heartbeat delivery (`listDueCommitmentsForSession` /
//! `markCommitmentsAttempted`).
//!
//! **Off by default.** No extraction or delivery happens unless
//! [`CommitmentsConfig::enabled`] is flipped on — no surprise LLM spend, no
//! nagging. Delivery wiring into the heartbeat tick lands once #133's
//! heartbeat-cadence pattern is in; this module ships the store + model now.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::debug;
use zeus_core::Result;

// ============================================================================
// Config
// ============================================================================

/// Operator knobs for the commitments subsystem. Matches OpenClaw's exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitmentsConfig {
    /// Master switch. **Default `false`** — no extraction, no delivery, no
    /// LLM spend until explicitly enabled.
    pub enabled: bool,
    /// Cap on commitments *extracted* per day, so a chatty session can't
    /// spawn an unbounded backlog of follow-ups. Default `3`.
    pub max_per_day: u32,
    /// Cron cadence on which the shared scheduler fires the delivery tick.
    /// Only registered while [`enabled`](Self::enabled) is true. Default: every
    /// 15 minutes.
    #[serde(default = "default_delivery_cron")]
    pub delivery_cron: String,
}

fn default_delivery_cron() -> String {
    "*/15 * * * *".to_string()
}

impl Default for CommitmentsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_per_day: 3,
            delivery_cron: default_delivery_cron(),
        }
    }
}

// ============================================================================
// Types
// ============================================================================

/// Lifecycle of a commitment.
///
/// `Pending` → not yet due, or due and awaiting delivery.
/// `Attempted` → delivered into a heartbeat prompt at least once (dedup guard).
/// `Done` → the agent confirmed the follow-up was honored.
/// `Expired` → the due-window closed without delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitmentStatus {
    Pending,
    Attempted,
    Done,
    Expired,
}

impl CommitmentStatus {
    fn as_str(&self) -> &'static str {
        match self {
            CommitmentStatus::Pending => "pending",
            CommitmentStatus::Attempted => "attempted",
            CommitmentStatus::Done => "done",
            CommitmentStatus::Expired => "expired",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "attempted" => CommitmentStatus::Attempted,
            "done" => CommitmentStatus::Done,
            "expired" => CommitmentStatus::Expired,
            _ => CommitmentStatus::Pending,
        }
    }
}

/// The window during which a commitment should be delivered, as epoch
/// milliseconds. `earliest_ms` is when it first becomes due; `latest_ms` is the
/// hard deadline after which it expires undelivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DueWindow {
    pub earliest_ms: i64,
    pub latest_ms: i64,
}

impl DueWindow {
    /// A window opening `from_now` and closing `+slack` after that.
    pub fn around(earliest: DateTime<Utc>, latest: DateTime<Utc>) -> Self {
        Self {
            earliest_ms: earliest.timestamp_millis(),
            latest_ms: latest.timestamp_millis(),
        }
    }

    /// True when `now` falls within `[earliest, latest]`.
    pub fn is_due_at(&self, now_ms: i64) -> bool {
        now_ms >= self.earliest_ms && now_ms <= self.latest_ms
    }

    /// True when `now` is past the hard deadline.
    pub fn is_expired_at(&self, now_ms: i64) -> bool {
        now_ms > self.latest_ms
    }
}

/// A single inferred follow-up the agent promised to honor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commitment {
    pub id: String,
    /// The promise, in the agent's own words ("check the build is green").
    pub text: String,
    /// Where to deliver it back — channel/thread id. Used for dedup so the
    /// agent doesn't nag the same place twice.
    pub channel: String,
    /// When it's due and when it expires.
    pub due_window: DueWindow,
    pub status: CommitmentStatus,
    pub created_at: DateTime<Utc>,
    /// The session that inferred this commitment (provenance + audit).
    pub source_session: String,
}

impl Commitment {
    pub fn new(
        text: impl Into<String>,
        channel: impl Into<String>,
        due_window: DueWindow,
        source_session: impl Into<String>,
    ) -> Self {
        Self {
            id: ulid::Ulid::new().to_string(),
            text: text.into(),
            channel: channel.into(),
            due_window,
            status: CommitmentStatus::Pending,
            created_at: Utc::now(),
            source_session: source_session.into(),
        }
    }
}

// ============================================================================
// Store
// ============================================================================

const COMMITMENT_MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE IF NOT EXISTS commitments (
        id              TEXT PRIMARY KEY,
        text            TEXT NOT NULL,
        channel         TEXT NOT NULL,
        earliest_ms     INTEGER NOT NULL,
        latest_ms       INTEGER NOT NULL,
        status          TEXT NOT NULL,
        created_at      TEXT NOT NULL,
        source_session  TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_commitments_status
        ON commitments(status);
    CREATE INDEX IF NOT EXISTS idx_commitments_channel
        ON commitments(channel);
    "#,
];

/// SQLite-backed store for commitments. Same skeleton as
/// [`crate::standing_orders::StandingOrderStore`].
pub struct CommitmentStore {
    path: PathBuf,
}

impl CommitmentStore {
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

    /// Default location: `~/.zeus/commitments.db`.
    pub fn default_path() -> Result<Self> {
        let path = dirs::home_dir()
            .ok_or_else(|| zeus_core::Error::Internal("no home directory".into()))?
            .join(".zeus/commitments.db");
        Self::new(path)
    }

    fn init(&self) -> Result<()> {
        let conn = self.conn()?;
        crate::db::run_migrations(&conn, COMMITMENT_MIGRATIONS)?;
        Ok(())
    }

    fn conn(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path).map_err(|e| {
            zeus_core::Error::Database(format!("Failed to open commitments db: {}", e))
        })?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| {
                zeus_core::Error::Database(format!("Failed to set commitments db pragmas: {}", e))
            })?;
        Ok(conn)
    }

    /// Insert a new commitment.
    pub fn add(&self, c: &Commitment) -> Result<String> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO commitments
             (id, text, channel, earliest_ms, latest_ms, status, created_at, source_session)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                c.id,
                c.text,
                c.channel,
                c.due_window.earliest_ms,
                c.due_window.latest_ms,
                c.status.as_str(),
                c.created_at.to_rfc3339(),
                c.source_session,
            ],
        )
        .map_err(|e| zeus_core::Error::Database(format!("Failed to add commitment: {}", e)))?;
        debug!(id = %c.id, text = %c.text, "Commitment added");
        Ok(c.id.clone())
    }

    /// Insert a commitment **only if** fewer than `max_per_day` have been
    /// created today (UTC). Returns `Ok(Some(id))` on insert, `Ok(None)` when
    /// the daily cap is already hit. This is the [`CommitmentsConfig::max_per_day`]
    /// guard, enforced at the store boundary so no caller can bypass it.
    pub fn add_capped(&self, c: &Commitment, max_per_day: u32) -> Result<Option<String>> {
        if self.count_created_today()? >= max_per_day as i64 {
            debug!(text = %c.text, max_per_day, "Commitment dropped — daily cap reached");
            return Ok(None);
        }
        Ok(Some(self.add(c)?))
    }

    /// Count commitments created since UTC midnight today.
    pub fn count_created_today(&self) -> Result<i64> {
        let conn = self.conn()?;
        let start = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|d| DateTime::<Utc>::from_naive_utc_and_offset(d, Utc))
            .unwrap_or_else(Utc::now);
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM commitments WHERE created_at >= ?1",
                params![start.to_rfc3339()],
                |row| row.get(0),
            )
            .map_err(|e| zeus_core::Error::Database(e.to_string()))?;
        Ok(n)
    }

    /// List all commitments (optionally filtered by status).
    pub fn list(&self, status: Option<CommitmentStatus>) -> Result<Vec<Commitment>> {
        let conn = self.conn()?;
        let (sql, filter): (&str, Option<String>) = match status {
            Some(s) => (
                "SELECT id, text, channel, earliest_ms, latest_ms, status, created_at, source_session
                 FROM commitments WHERE status = ?1
                 ORDER BY earliest_ms ASC, created_at ASC",
                Some(s.as_str().to_string()),
            ),
            None => (
                "SELECT id, text, channel, earliest_ms, latest_ms, status, created_at, source_session
                 FROM commitments
                 ORDER BY earliest_ms ASC, created_at ASC",
                None,
            ),
        };
        let conn_ref = &conn;
        let mut stmt = conn_ref
            .prepare(sql)
            .map_err(|e| zeus_core::Error::Database(e.to_string()))?;

        let mapper = |row: &rusqlite::Row<'_>| -> rusqlite::Result<Commitment> {
            let created: String = row.get(6)?;
            Ok(Commitment {
                id: row.get(0)?,
                text: row.get(1)?,
                channel: row.get(2)?,
                due_window: DueWindow {
                    earliest_ms: row.get(3)?,
                    latest_ms: row.get(4)?,
                },
                status: {
                    let s: String = row.get(5)?;
                    CommitmentStatus::from_str(&s)
                },
                created_at: DateTime::parse_from_rfc3339(&created)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                source_session: row.get(7)?,
            })
        };

        let rows = match filter {
            Some(s) => stmt.query_map(params![s], mapper),
            None => stmt.query_map([], mapper),
        }
        .map_err(|e| zeus_core::Error::Database(e.to_string()))?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| zeus_core::Error::Database(e.to_string()))?);
        }
        Ok(out)
    }

    /// Commitments that are due *now* and not yet attempted/done/expired.
    /// This is the delivery query the heartbeat will call (`listDue` in
    /// OpenClaw). Dedup is by status: once `Attempted`, it won't resurface.
    pub fn list_due(&self, now_ms: i64) -> Result<Vec<Commitment>> {
        Ok(self
            .list(Some(CommitmentStatus::Pending))?
            .into_iter()
            .filter(|c| c.due_window.is_due_at(now_ms))
            .collect())
    }

    /// Mark a commitment as delivered into a heartbeat prompt (the dedup
    /// transition — OpenClaw's `markCommitmentsAttempted`).
    pub fn mark_attempted(&self, id: &str) -> Result<()> {
        self.set_status(id, CommitmentStatus::Attempted)
    }

    /// Mark a commitment as honored.
    pub fn mark_done(&self, id: &str) -> Result<()> {
        self.set_status(id, CommitmentStatus::Done)
    }

    fn set_status(&self, id: &str, status: CommitmentStatus) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE commitments SET status = ?1 WHERE id = ?2",
            params![status.as_str(), id],
        )
        .map_err(|e| zeus_core::Error::Database(e.to_string()))?;
        Ok(())
    }

    /// Sweep any `Pending` commitments whose window has closed into `Expired`.
    /// Returns the number swept. Cheap to call on each heartbeat tick.
    pub fn expire_stale(&self, now_ms: i64) -> Result<usize> {
        let conn = self.conn()?;
        let n = conn
            .execute(
                "UPDATE commitments SET status = ?1
                 WHERE status = ?2 AND latest_ms < ?3",
                params![
                    CommitmentStatus::Expired.as_str(),
                    CommitmentStatus::Pending.as_str(),
                    now_ms
                ],
            )
            .map_err(|e| zeus_core::Error::Database(e.to_string()))?;
        Ok(n)
    }

    /// Remove a commitment outright.
    pub fn remove(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM commitments WHERE id = ?1", params![id])
            .map_err(|e| zeus_core::Error::Database(e.to_string()))?;
        Ok(())
    }
}

impl CommitmentsConfig {
    /// Cadence registration for the shared scheduler.
    ///
    /// Returns an empty `Vec` while the subsystem is disabled — so a
    /// `SubsystemTick { kind: "commitments" }` is *never* registered, and the
    /// delivery loop never fires, until [`enabled`](Self::enabled) is flipped
    /// on. Off-by-default holds here, at the same `enabled` gate dreaming uses.
    /// No second scheduler is forked: this hands one [`TaskConfig`] to the
    /// shared `CronScheduler`, exactly like [`crate::dreaming`].
    pub fn scheduler_tasks(&self) -> Vec<crate::scheduler::TaskConfig> {
        if !self.enabled {
            return Vec::new();
        }
        vec![crate::scheduler::TaskConfig {
            name: "commitments:deliver".to_string(),
            cron: self.delivery_cron.clone(),
            task_type: crate::scheduler::TaskType::SubsystemTick {
                kind: "commitments".to_string(),
            },
            enabled: true,
            run_at: None,
            run_once: false,
            wake_mode: crate::scheduler::WakeMode::Now,
        delivery_mode: crate::scheduler::DeliveryMode::Channel,
        }]
    }
}

/// Drain due commitments into a workspace note and mark them attempted.
///
/// This is the delivery loop the scheduler's `SubsystemTick { kind:
/// "commitments" }` arm calls. The sequence mirrors OpenClaw's
/// `listDueCommitmentsForSession` → render → `markCommitmentsAttempted`:
///
/// 1. **Sweep stale** — expire any commitments whose window has fully closed.
/// 2. **List due** — `Pending` commitments inside their window (dedup is by
///    status: an already-`Attempted` one never reappears here).
/// 3. **Deliver** — render them into today's workspace note so they surface in
///    the next heartbeat context.
/// 4. **Latch** — flip each delivered commitment to `Attempted` so it won't be
///    redelivered on the next tick.
///
/// Returns the number delivered. The daily cap is enforced upstream at
/// *creation* (`add_capped`), so delivery itself is uncapped — it only ever
/// surfaces commitments that already passed the cap.
pub async fn deliver_due(
    store: &CommitmentStore,
    workspace: &zeus_memory::Workspace,
) -> Result<usize> {
    let now_ms = Utc::now().timestamp_millis();

    // 1. Sweep windows that have fully closed.
    let expired = store.expire_stale(now_ms)?;
    if expired > 0 {
        debug!(expired, "commitments: swept stale before delivery");
    }

    // 2. List what's due and not yet attempted.
    let due = store.list_due(now_ms)?;
    if due.is_empty() {
        return Ok(0);
    }

    // 3. Render into a workspace note.
    let mut lines = Vec::with_capacity(due.len() + 1);
    lines.push(format!(
        "🔔 {} commitment(s) due — follow up:",
        due.len()
    ));
    for c in &due {
        lines.push(format!("- {}", c.text));
    }
    workspace.note(&lines.join("\n")).await?;

    // 4. Latch each as attempted (dedup transition).
    for c in &due {
        store.mark_attempted(&c.id)?;
    }

    debug!(delivered = due.len(), "commitments: delivered due batch");
    Ok(due.len())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use tempfile::tempdir;

    fn window_now(slack_secs: i64) -> DueWindow {
        let now = Utc::now();
        DueWindow::around(now - Duration::seconds(1), now + Duration::seconds(slack_secs))
    }

    #[test]
    fn config_default_is_off_and_capped_at_three() {
        let cfg = CommitmentsConfig::default();
        assert!(!cfg.enabled, "commitments must default OFF — no surprise spend");
        assert_eq!(cfg.max_per_day, 3);
    }

    #[test]
    fn disabled_config_registers_no_scheduler_task() {
        // Off-by-default must hold at the scheduler boundary: a disabled
        // subsystem emits zero TaskConfigs, so no SubsystemTick ever fires.
        let cfg = CommitmentsConfig::default();
        assert!(!cfg.enabled);
        assert!(
            cfg.scheduler_tasks().is_empty(),
            "disabled commitments must register zero cron jobs"
        );
    }

    #[test]
    fn enabled_config_registers_one_subsystem_tick() {
        let cfg = CommitmentsConfig {
            enabled: true,
            ..Default::default()
        };
        let tasks = cfg.scheduler_tasks();
        assert_eq!(tasks.len(), 1, "exactly one delivery task — no second scheduler");
        assert_eq!(tasks[0].name, "commitments:deliver");
        match &tasks[0].task_type {
            crate::scheduler::TaskType::SubsystemTick { kind } => {
                assert_eq!(kind, "commitments");
            }
            other => panic!("expected SubsystemTick, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn deliver_due_drains_latches_and_dedups() {
        let dir = tempdir().unwrap();
        let store = CommitmentStore::new(dir.path().join("c.db")).unwrap();
        let workspace = zeus_memory::Workspace::new(dir.path().join("ws"));

        // One due, one not-yet-due.
        store
            .add(&Commitment::new("ping the deploy", "ch", window_now(600), "s"))
            .unwrap();
        let future = DueWindow::around(
            Utc::now() + Duration::hours(1),
            Utc::now() + Duration::hours(2),
        );
        store
            .add(&Commitment::new("later thing", "ch", future, "s"))
            .unwrap();

        // First delivery: only the due one goes out.
        let n = deliver_due(&store, &workspace).await.unwrap();
        assert_eq!(n, 1, "only the in-window commitment delivers");

        // It's now latched to Attempted — dedup means a second tick delivers 0.
        assert_eq!(
            store.list(Some(CommitmentStatus::Attempted)).unwrap().len(),
            1
        );
        let n2 = deliver_due(&store, &workspace).await.unwrap();
        assert_eq!(n2, 0, "attempted commitments must not resurface");
    }

    #[test]
    fn add_list_and_status_transitions() {
        let dir = tempdir().unwrap();
        let store = CommitmentStore::new(dir.path().join("c.db")).unwrap();

        let c = Commitment::new("check the build is green", "discord:123", window_now(600), "sess-a");
        let id = store.add(&c).unwrap();
        assert_eq!(store.list(None).unwrap().len(), 1);
        assert_eq!(store.list(Some(CommitmentStatus::Pending)).unwrap().len(), 1);

        store.mark_attempted(&id).unwrap();
        assert_eq!(store.list(Some(CommitmentStatus::Pending)).unwrap().len(), 0);
        assert_eq!(store.list(Some(CommitmentStatus::Attempted)).unwrap().len(), 1);

        store.mark_done(&id).unwrap();
        assert_eq!(store.list(Some(CommitmentStatus::Done)).unwrap().len(), 1);

        store.remove(&id).unwrap();
        assert_eq!(store.list(None).unwrap().len(), 0);
    }

    #[test]
    fn list_due_respects_window_and_dedup() {
        let dir = tempdir().unwrap();
        let store = CommitmentStore::new(dir.path().join("c.db")).unwrap();
        let now = Utc::now().timestamp_millis();

        // Due now.
        let due = Commitment::new("ping deploy", "ch", window_now(600), "s");
        let due_id = store.add(&due).unwrap();
        // Not due until far future.
        let future_win = DueWindow::around(
            Utc::now() + Duration::hours(5),
            Utc::now() + Duration::hours(6),
        );
        store
            .add(&Commitment::new("later", "ch", future_win, "s"))
            .unwrap();

        let due_list = store.list_due(now).unwrap();
        assert_eq!(due_list.len(), 1, "only the in-window commitment is due");
        assert_eq!(due_list[0].id, due_id);

        // Once attempted, it no longer surfaces (dedup).
        store.mark_attempted(&due_id).unwrap();
        assert_eq!(store.list_due(now).unwrap().len(), 0);
    }

    #[test]
    fn expire_stale_sweeps_closed_windows() {
        let dir = tempdir().unwrap();
        let store = CommitmentStore::new(dir.path().join("c.db")).unwrap();

        let past = DueWindow::around(
            Utc::now() - Duration::hours(2),
            Utc::now() - Duration::hours(1),
        );
        store
            .add(&Commitment::new("stale promise", "ch", past, "s"))
            .unwrap();
        store
            .add(&Commitment::new("fresh promise", "ch", window_now(600), "s"))
            .unwrap();

        let swept = store.expire_stale(Utc::now().timestamp_millis()).unwrap();
        assert_eq!(swept, 1);
        assert_eq!(store.list(Some(CommitmentStatus::Expired)).unwrap().len(), 1);
        assert_eq!(store.list(Some(CommitmentStatus::Pending)).unwrap().len(), 1);
    }

    #[test]
    fn add_capped_enforces_daily_limit() {
        let dir = tempdir().unwrap();
        let store = CommitmentStore::new(dir.path().join("c.db")).unwrap();
        let cap = 3;

        for i in 0..cap {
            let c = Commitment::new(format!("promise {i}"), "ch", window_now(600), "s");
            assert!(store.add_capped(&c, cap).unwrap().is_some());
        }
        // 4th in the same day is dropped.
        let over = Commitment::new("one too many", "ch", window_now(600), "s");
        assert!(
            store.add_capped(&over, cap).unwrap().is_none(),
            "daily cap must reject the (cap+1)th commitment"
        );
        assert_eq!(store.count_created_today().unwrap(), cap as i64);
    }
}
