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
        }
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
        Connection::open(&self.path).map_err(|e| {
            zeus_core::Error::Database(format!("Failed to open standing_orders db: {}", e))
        })
    }

    /// Insert a new standing order.
    pub fn add(&self, order: &StandingOrder) -> Result<String> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO standing_orders
             (id, description, priority, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                order.id,
                order.description,
                order.priority as i32,
                order.status.as_str(),
                order.created_at.to_rfc3339(),
                order.updated_at.to_rfc3339(),
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
                "SELECT id, description, priority, status, created_at, updated_at
                 FROM standing_orders WHERE status = ?1
                 ORDER BY priority DESC, created_at ASC",
                Some(s.as_str()),
            ),
            None => (
                "SELECT id, description, priority, status, created_at, updated_at
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
}
