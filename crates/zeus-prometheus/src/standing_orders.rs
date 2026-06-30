//! Standing Orders - Persistent multi-day goals that survive restarts.
//!
//! Unlike one-shot tasks or the richer `GoalStack`, a Standing Order is a
//! durable directive ("keep an eye on X", "every day, do Y") that is parsed
//! from `HEARTBEAT.md` and persisted to SQLite. On gateway boot, active
//! standing orders are loaded back into the heartbeat prompt so the agent
//! stays on-mission across restarts.
//!
//! Format expected in `HEARTBEAT.md`:
//!
//! ```markdown
//! ## STANDING ORDERS
//! - [P1] Monitor #alerts channel and triage anything urgent
//! - [P2] Keep MEMORY.md under 500 lines — prune weekly
//! - Check in with zeus100 every morning
//! ```
//!
//! Priority prefix `[P1]`/`[P2]`/`[P3]` is optional; omitted → Normal.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::debug;
use zeus_core::Result;

// ============================================================================
// Types
// ============================================================================

/// Priority level for a standing order (mirrors `goals::Priority` but kept
/// local so standing orders can evolve independently).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderPriority {
    Low = 1,
    Normal = 2,
    High = 3,
    Critical = 4,
}

impl OrderPriority {
    fn from_i32(v: i32) -> Self {
        match v {
            1 => OrderPriority::Low,
            3 => OrderPriority::High,
            4 => OrderPriority::Critical,
            _ => OrderPriority::Normal,
        }
    }

    /// Parse a `P1`/`P2`/`P3`/`P4` token (case-insensitive).
    fn from_tag(tag: &str) -> Option<Self> {
        match tag.trim().to_ascii_uppercase().as_str() {
            "P1" | "CRITICAL" => Some(OrderPriority::Critical),
            "P2" | "HIGH" => Some(OrderPriority::High),
            "P3" | "NORMAL" => Some(OrderPriority::Normal),
            "P4" | "LOW" => Some(OrderPriority::Low),
            _ => None,
        }
    }
}

impl std::fmt::Display for OrderPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderPriority::Low => write!(f, "low"),
            OrderPriority::Normal => write!(f, "normal"),
            OrderPriority::High => write!(f, "high"),
            OrderPriority::Critical => write!(f, "critical"),
        }
    }
}

/// Lifecycle state of a standing order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Active,
    Completed,
    Archived,
}

impl OrderStatus {
    fn as_str(&self) -> &'static str {
        match self {
            OrderStatus::Active => "active",
            OrderStatus::Completed => "completed",
            OrderStatus::Archived => "archived",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "completed" => OrderStatus::Completed,
            "archived" => OrderStatus::Archived,
            _ => OrderStatus::Active,
        }
    }
}

/// A single persistent directive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandingOrder {
    pub id: String,
    pub description: String,
    pub priority: OrderPriority,
    pub status: OrderStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// When the agent last acted on this order (None = never touched).
    pub last_acted_at: Option<DateTime<Utc>>,
    /// Short freeform progress note from the last action.
    pub notes: String,
}

impl StandingOrder {
    pub fn new(description: impl Into<String>, priority: OrderPriority) -> Self {
        let now = Utc::now();
        Self {
            id: ulid::Ulid::new().to_string(),
            description: description.into(),
            priority,
            status: OrderStatus::Active,
            created_at: now,
            updated_at: now,
            last_acted_at: None,
            notes: String::new(),
        }
    }

    /// Staleness window per priority: how long an active order may sit
    /// untouched before it surfaces a flag. P1 is tightest.
    pub fn staleness_window(&self) -> chrono::Duration {
        match self.priority {
            OrderPriority::Critical => chrono::Duration::hours(12),
            OrderPriority::High => chrono::Duration::days(1),
            OrderPriority::Normal => chrono::Duration::days(3),
            OrderPriority::Low => chrono::Duration::days(7),
        }
    }

    /// True if this active order hasn't been acted on within its cadence
    /// window. An order never acted on is measured from `created_at`.
    pub fn is_stale(&self, now: DateTime<Utc>) -> bool {
        if self.status != OrderStatus::Active {
            return false;
        }
        let anchor = self.last_acted_at.unwrap_or(self.created_at);
        now.signed_duration_since(anchor) > self.staleness_window()
    }
}

// ============================================================================
// Store
// ============================================================================

const ORDER_MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE IF NOT EXISTS standing_orders (
        id           TEXT PRIMARY KEY,
        description  TEXT NOT NULL,
        priority     INTEGER NOT NULL,
        status       TEXT NOT NULL,
        created_at   TEXT NOT NULL,
        updated_at   TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_standing_orders_status
        ON standing_orders(status);
    "#,
    // #133 deepening: progress tracking. Additive columns; existing rows get
    // NULL last_acted_at (= never touched) and empty notes.
    r#"
    ALTER TABLE standing_orders ADD COLUMN last_acted_at TEXT;
    ALTER TABLE standing_orders ADD COLUMN notes TEXT NOT NULL DEFAULT '';
    "#,
];

/// SQLite-backed store for standing orders.
pub struct StandingOrderStore {
    path: PathBuf,
}

impl StandingOrderStore {
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

    /// Default location: `~/.zeus/standing_orders.db`.
    pub fn default_path() -> Result<Self> {
        let path = dirs::home_dir()
            .ok_or_else(|| zeus_core::Error::Internal("no home directory".into()))?
            .join(".zeus/standing_orders.db");
        Self::new(path)
    }

    fn init(&self) -> Result<()> {
        let conn = self.conn()?;
        crate::db::run_migrations(&conn, ORDER_MIGRATIONS)?;
        Ok(())
    }

    fn conn(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path).map_err(|e| {
            zeus_core::Error::Database(format!("Failed to open standing_orders db: {}", e))
        })?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| {
                zeus_core::Error::Database(format!(
                    "Failed to set standing_orders db pragmas: {}",
                    e
                ))
            })?;
        Ok(conn)
    }

    /// Insert a new standing order.
    pub fn add(&self, order: &StandingOrder) -> Result<String> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO standing_orders
             (id, description, priority, status, created_at, updated_at, last_acted_at, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                order.id,
                order.description,
                order.priority as i32,
                order.status.as_str(),
                order.created_at.to_rfc3339(),
                order.updated_at.to_rfc3339(),
                order.last_acted_at.map(|d| d.to_rfc3339()),
                order.notes,
            ],
        )
        .map_err(|e| zeus_core::Error::Database(format!("Failed to add standing order: {}", e)))?;
        debug!(id = %order.id, desc = %order.description, "Standing order added");
        Ok(order.id.clone())
    }

    /// List all orders (optionally filtered by status).
    pub fn list(&self, status: Option<OrderStatus>) -> Result<Vec<StandingOrder>> {
        let conn = self.conn()?;
        let (sql, filter): (&str, Option<&'static str>) = match status {
            Some(s) => (
                "SELECT id, description, priority, status, created_at, updated_at, last_acted_at, notes
                 FROM standing_orders WHERE status = ?1
                 ORDER BY priority DESC, created_at ASC",
                Some(s.as_str()),
            ),
            None => (
                "SELECT id, description, priority, status, created_at, updated_at, last_acted_at, notes
                 FROM standing_orders
                 ORDER BY priority DESC, created_at ASC",
                None,
            ),
        };
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| zeus_core::Error::Database(e.to_string()))?;

        let mapper = |row: &rusqlite::Row<'_>| -> rusqlite::Result<StandingOrder> {
            let created: String = row.get(4)?;
            let updated: String = row.get(5)?;
            let last_acted: Option<String> = row.get(6)?;
            let notes: String = row.get(7)?;
            Ok(StandingOrder {
                id: row.get(0)?,
                description: row.get(1)?,
                priority: OrderPriority::from_i32(row.get::<_, i32>(2)?),
                status: OrderStatus::from_str(&row.get::<_, String>(3)?),
                created_at: DateTime::parse_from_rfc3339(&created)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: DateTime::parse_from_rfc3339(&updated)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                last_acted_at: last_acted.and_then(|s| {
                    DateTime::parse_from_rfc3339(&s)
                        .map(|d| d.with_timezone(&Utc))
                        .ok()
                }),
                notes,
            })
        };

        let rows: Vec<StandingOrder> = if let Some(f) = filter {
            stmt.query_map(params![f], mapper)
                .map_err(|e| zeus_core::Error::Database(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect()
        } else {
            stmt.query_map([], mapper)
                .map_err(|e| zeus_core::Error::Database(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect()
        };
        Ok(rows)
    }

    /// Convenience: return only active orders, priority-sorted.
    pub fn active(&self) -> Result<Vec<StandingOrder>> {
        self.list(Some(OrderStatus::Active))
    }

    /// Record that the agent acted on an order: stamps `last_acted_at = now`
    /// and stores a short progress `note`. Resets the staleness clock.
    pub fn record_action(&self, id: &str, note: &str) -> Result<()> {
        let conn = self.conn()?;
        let now = Utc::now().to_rfc3339();
        let n = conn
            .execute(
                "UPDATE standing_orders
                 SET last_acted_at = ?1, notes = ?2, updated_at = ?1
                 WHERE id = ?3",
                params![now, note, id],
            )
            .map_err(|e| zeus_core::Error::Database(e.to_string()))?;
        if n == 0 {
            return Err(zeus_core::Error::NotFound(format!(
                "standing order {} not found",
                id
            )));
        }
        debug!(id = %id, "Standing order action recorded");
        Ok(())
    }

    /// Return active orders that have gone stale (untouched past their
    /// priority cadence window), priority-sorted.
    pub fn stale(&self) -> Result<Vec<StandingOrder>> {
        let now = Utc::now();
        Ok(self
            .active()?
            .into_iter()
            .filter(|o| o.is_stale(now))
            .collect())
    }

    /// Mark an order as completed.
    pub fn complete(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        let now = Utc::now().to_rfc3339();
        let n = conn
            .execute(
                "UPDATE standing_orders SET status = 'completed', updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )
            .map_err(|e| zeus_core::Error::Database(e.to_string()))?;
        if n == 0 {
            return Err(zeus_core::Error::NotFound(format!(
                "standing order {} not found",
                id
            )));
        }
        Ok(())
    }

    /// Delete an order permanently.
    pub fn remove(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        let n = conn
            .execute("DELETE FROM standing_orders WHERE id = ?1", params![id])
            .map_err(|e| zeus_core::Error::Database(e.to_string()))?;
        if n == 0 {
            return Err(zeus_core::Error::NotFound(format!(
                "standing order {} not found",
                id
            )));
        }
        Ok(())
    }

    /// Idempotent: reconcile the store with the `## STANDING ORDERS` section
    /// of HEARTBEAT.md. Any description already present stays as-is; new
    /// descriptions are inserted. Does NOT delete missing ones — that's the
    /// user's call via `remove`.
    pub fn sync_from_heartbeat(&self, heartbeat_md: &str) -> Result<usize> {
        let parsed = parse_standing_orders(heartbeat_md);
        let existing = self.list(None)?;
        let mut added = 0usize;
        for (desc, prio) in parsed {
            if existing.iter().any(|o| o.description == desc) {
                continue;
            }
            let order = StandingOrder::new(desc, prio);
            self.add(&order)?;
            added += 1;
        }
        Ok(added)
    }
}

// ============================================================================
// Parsing
// ============================================================================

/// Parse the `## STANDING ORDERS` section of HEARTBEAT.md into
/// `(description, priority)` pairs. Lines outside that section are ignored.
/// Recognised bullet prefixes: `-`, `*`, `•`. Priority tag `[P1]`/`[P2]`/etc.
/// is optional and stripped from the description.
pub fn parse_standing_orders(md: &str) -> Vec<(String, OrderPriority)> {
    let mut in_section = false;
    let mut out = Vec::new();

    for raw in md.lines() {
        let line = raw.trim();

        // Section boundaries: any `## ...` or `# ...` header flips state.
        if line.starts_with('#') {
            let upper = line.to_ascii_uppercase();
            in_section = upper.contains("STANDING ORDER");
            continue;
        }
        if !in_section || line.is_empty() {
            continue;
        }

        // Accept `-`, `*`, `•` bullets.
        let body = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .or_else(|| line.strip_prefix("• "));
        let Some(body) = body else { continue };
        let body = body.trim();
        if body.is_empty() {
            continue;
        }

        // Optional `[P1]` priority tag.
        let (priority, desc) = if let Some(rest) = body.strip_prefix('[') {
            if let Some(end) = rest.find(']') {
                let tag = &rest[..end];
                let after = rest[end + 1..].trim();
                match OrderPriority::from_tag(tag) {
                    Some(p) => (p, after.to_string()),
                    None => (OrderPriority::Normal, body.to_string()),
                }
            } else {
                (OrderPriority::Normal, body.to_string())
            }
        } else {
            (OrderPriority::Normal, body.to_string())
        };

        if !desc.is_empty() {
            out.push((desc, priority));
        }
    }

    out
}

// ============================================================================
// Cadence-fire config + delivery (the 4th deepening axis, #133)
// ============================================================================

/// Operator knobs for the standing-orders **cadence-fire**.
///
/// The store + staleness logic already exist (and overdue orders already
/// render into the boot/heartbeat context). This config adds the *active*
/// half: a periodic tick through the **one** shared `CronScheduler` that
/// surfaces stale orders on cadence — so an order that goes untouched between
/// heartbeats still gets pushed into a note rather than waiting passively.
///
/// Off-by-default like every other autonomy subsystem: while `enabled` is
/// `false`, [`scheduler_tasks`](Self::scheduler_tasks) returns an empty `Vec`,
/// so no `SubsystemTick { kind: "standing-orders" }` is ever registered and the
/// surface loop never fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandingOrdersConfig {
    /// Master switch. **Default `false`** — no cadence tick, no extra notes,
    /// until explicitly enabled. The passive boot/heartbeat surfacing of stale
    /// orders is unaffected by this flag; this gates only the *scheduled* push.
    pub enabled: bool,
    /// Cron cadence on which the shared scheduler fires the staleness-surface
    /// tick. Only registered while [`enabled`](Self::enabled) is true.
    /// Default: hourly — coarse enough not to spam, fine enough that an overdue
    /// order surfaces within the hour even on a quiet day.
    #[serde(default = "default_cadence_cron")]
    pub cadence_cron: String,
}

fn default_cadence_cron() -> String {
    "0 * * * *".to_string()
}

impl Default for StandingOrdersConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cadence_cron: default_cadence_cron(),
        }
    }
}

impl StandingOrdersConfig {
    /// Cadence registration for the shared scheduler.
    ///
    /// Returns an empty `Vec` while the subsystem is disabled — so a
    /// `SubsystemTick { kind: "standing-orders" }` is *never* registered and
    /// [`surface_stale`] never fires until [`enabled`](Self::enabled) is
    /// flipped on. Off-by-default holds here, at the same `enabled` gate
    /// `commitments`/`dreaming` use. No second scheduler is forked: this hands
    /// one [`TaskConfig`](crate::scheduler::TaskConfig) to the shared
    /// `CronScheduler`, exactly like the sibling subsystems.
    pub fn scheduler_tasks(&self) -> Vec<crate::scheduler::TaskConfig> {
        if !self.enabled {
            return Vec::new();
        }
        vec![crate::scheduler::TaskConfig {
            name: "standing-orders:surface-stale".to_string(),
            cron: self.cadence_cron.clone(),
            task_type: crate::scheduler::TaskType::SubsystemTick {
                kind: "standing-orders".to_string(),
            },
            enabled: true,
            run_at: None,
            run_once: false,
            wake_mode: crate::scheduler::WakeMode::Now,
        delivery_mode: crate::scheduler::DeliveryMode::Channel,
        }]
    }
}

/// Surface stale standing orders into a workspace note on cadence.
///
/// This is the delivery loop the scheduler's `SubsystemTick { kind:
/// "standing-orders" }` arm calls. It consumes the existing
/// [`StandingOrderStore::stale`] filter (priority-windowed overdue actives) and
/// renders them into today's workspace note, mirroring `commitments::deliver_due`:
///
/// 1. **List stale** — active orders past their per-priority staleness window.
/// 2. **Render** — one note line per overdue order, flagged `⚠️ STALE`, with
///    the last-acted timestamp (or "never") so the agent sees what's been
///    languishing.
///
/// Unlike commitments there is **no attempted-latch transition**: a standing
/// order is durable and re-surfaces every cadence until the agent *acts* on it
/// (`record_action`), which resets `last_acted_at` and drops it out of `stale()`
/// naturally. Returns the count surfaced (0 ⇒ nothing overdue, no note written).
pub async fn surface_stale(
    store: &StandingOrderStore,
    workspace: &zeus_memory::Workspace,
) -> Result<usize> {
    let stale = store.stale()?;
    if stale.is_empty() {
        return Ok(0);
    }

    let now = Utc::now();
    let mut lines = Vec::with_capacity(stale.len() + 1);
    lines.push(format!(
        "⚠️ {} standing order(s) overdue — act or update:",
        stale.len()
    ));
    for o in &stale {
        let last = o
            .last_acted_at
            .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| "never".to_string());
        lines.push(format!(
            "- [{}] {} (⚠️ STALE — last acted: {})",
            o.priority, o.description, last
        ));
    }
    workspace.note(&lines.join("\n")).await?;

    debug!(
        surfaced = stale.len(),
        now = %now.to_rfc3339(),
        "standing-orders: surfaced stale batch on cadence"
    );
    Ok(stale.len())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_extracts_orders_with_priorities() {
        let md = "\
# HEARTBEAT

## CURRENT TASK
Do the thing.

## STANDING ORDERS
- [P1] Watch the alerts channel
- [P2] Prune MEMORY.md weekly
- Check in with zeus100 daily
* Keep commits small

## Other Section
- ignored bullet
";
        let parsed = parse_standing_orders(md);
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].1, OrderPriority::Critical);
        assert_eq!(parsed[0].0, "Watch the alerts channel");
        assert_eq!(parsed[1].1, OrderPriority::High);
        assert_eq!(parsed[2].1, OrderPriority::Normal);
        assert_eq!(parsed[2].0, "Check in with zeus100 daily");
        assert_eq!(parsed[3].0, "Keep commits small");
    }

    #[test]
    fn store_add_list_complete_remove() {
        let dir = tempdir().unwrap();
        let store = StandingOrderStore::new(dir.path().join("so.db")).unwrap();

        let a = StandingOrder::new("first", OrderPriority::High);
        let b = StandingOrder::new("second", OrderPriority::Normal);
        store.add(&a).unwrap();
        store.add(&b).unwrap();

        let active = store.active().unwrap();
        assert_eq!(active.len(), 2);
        // High priority should come first.
        assert_eq!(active[0].description, "first");

        store.complete(&a.id).unwrap();
        let active = store.active().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].description, "second");

        let done = store.list(Some(OrderStatus::Completed)).unwrap();
        assert_eq!(done.len(), 1);

        store.remove(&b.id).unwrap();
        assert_eq!(store.active().unwrap().len(), 0);
    }

    #[test]
    fn sync_from_heartbeat_is_idempotent() {
        let dir = tempdir().unwrap();
        let store = StandingOrderStore::new(dir.path().join("so.db")).unwrap();

        let md = "## STANDING ORDERS\n- [P1] alpha\n- beta\n";
        assert_eq!(store.sync_from_heartbeat(md).unwrap(), 2);
        // Second sync adds nothing.
        assert_eq!(store.sync_from_heartbeat(md).unwrap(), 0);
        assert_eq!(store.active().unwrap().len(), 2);
    }

    #[test]
    fn record_action_stamps_last_acted_and_notes() {
        let dir = tempdir().unwrap();
        let store = StandingOrderStore::new(dir.path().join("so.db")).unwrap();

        let o = StandingOrder::new("watch alerts", OrderPriority::High);
        let id = store.add(&o).unwrap();

        // Before acting: never touched.
        let before = &store.active().unwrap()[0];
        assert!(before.last_acted_at.is_none());
        assert_eq!(before.notes, "");

        store.record_action(&id, "checked, all clear").unwrap();

        let after = &store.active().unwrap()[0];
        assert!(after.last_acted_at.is_some());
        assert_eq!(after.notes, "checked, all clear");

        // Acting on a missing id errors.
        assert!(store.record_action("nope", "x").is_err());
    }

    #[test]
    fn staleness_respects_priority_window_and_resets_on_action() {
        // A High-priority order created long ago is stale; acting clears it.
        let mut o = StandingOrder::new("ship the thing", OrderPriority::High);
        // Force creation far in the past.
        o.created_at = Utc::now() - chrono::Duration::days(3);
        o.updated_at = o.created_at;

        let now = Utc::now();
        assert!(
            o.is_stale(now),
            "a 3-day-untouched High order must read stale"
        );

        // A fresh action within the window clears staleness.
        o.last_acted_at = Some(now);
        assert!(
            !o.is_stale(now),
            "an order acted on just now must not be stale"
        );

        // Critical window is tighter than High.
        let crit = StandingOrder::new("p1", OrderPriority::Critical);
        let high = StandingOrder::new("p2", OrderPriority::High);
        assert!(
            crit.staleness_window() < high.staleness_window(),
            "Critical must surface staleness sooner than High"
        );
    }

    #[test]
    fn stale_filter_returns_only_overdue_active_orders() {
        let dir = tempdir().unwrap();
        let store = StandingOrderStore::new(dir.path().join("so.db")).unwrap();

        // Fresh order — not stale.
        store
            .add(&StandingOrder::new("fresh", OrderPriority::High))
            .unwrap();

        // Old order — stale (created in the past, never acted).
        let mut old = StandingOrder::new("overdue", OrderPriority::Critical);
        old.created_at = Utc::now() - chrono::Duration::days(2);
        old.updated_at = old.created_at;
        store.add(&old).unwrap();

        let stale = store.stale().unwrap();
        assert_eq!(stale.len(), 1, "only the overdue order should surface");
        assert_eq!(stale[0].description, "overdue");
    }

    // --- cadence-fire (4th axis, #133) ---

    #[test]
    fn disabled_config_registers_no_scheduler_task() {
        // Off-by-default must hold at the scheduler boundary: a disabled
        // subsystem emits zero TaskConfigs, so no SubsystemTick ever fires.
        let cfg = StandingOrdersConfig::default();
        assert!(!cfg.enabled, "standing-orders cadence must default OFF");
        assert!(
            cfg.scheduler_tasks().is_empty(),
            "disabled standing-orders must register zero cron jobs"
        );
    }

    #[test]
    fn enabled_config_registers_one_subsystem_tick() {
        let cfg = StandingOrdersConfig {
            enabled: true,
            ..Default::default()
        };
        let tasks = cfg.scheduler_tasks();
        assert_eq!(
            tasks.len(),
            1,
            "exactly one surface task — no second scheduler"
        );
        assert_eq!(tasks[0].name, "standing-orders:surface-stale");
        match &tasks[0].task_type {
            crate::scheduler::TaskType::SubsystemTick { kind } => {
                assert_eq!(kind, "standing-orders");
            }
            other => panic!("expected SubsystemTick, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn surface_stale_pushes_overdue_and_skips_when_clean() {
        let dir = tempdir().unwrap();
        let store = StandingOrderStore::new(dir.path().join("so.db")).unwrap();
        let workspace = zeus_memory::Workspace::new(dir.path().join("ws"));

        // Fresh order — not stale, must not surface.
        store
            .add(&StandingOrder::new("fresh", OrderPriority::High))
            .unwrap();

        // Clean run: nothing overdue ⇒ 0, no note.
        let n = surface_stale(&store, &workspace).await.unwrap();
        assert_eq!(n, 0, "no overdue orders ⇒ nothing surfaced");

        // Add an overdue order.
        let mut old = StandingOrder::new("overdue", OrderPriority::Critical);
        old.created_at = Utc::now() - chrono::Duration::days(2);
        old.updated_at = old.created_at;
        store.add(&old).unwrap();

        // Now exactly the overdue one surfaces.
        let n = surface_stale(&store, &workspace).await.unwrap();
        assert_eq!(n, 1, "exactly the overdue order surfaces on cadence");

        // No latch: it re-surfaces until acted on.
        let n = surface_stale(&store, &workspace).await.unwrap();
        assert_eq!(n, 1, "standing order re-surfaces — no attempted-latch");

        // Acting on it resets staleness ⇒ drops out of the next surface.
        store.record_action(&old.id, "handled").unwrap();
        let n = surface_stale(&store, &workspace).await.unwrap();
        assert_eq!(n, 0, "record_action resets last_acted_at ⇒ no longer stale");
    }
}
