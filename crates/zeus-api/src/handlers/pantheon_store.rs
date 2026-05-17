// ═══════════════════════════════════════════════════════════
// ZEUS — Pantheon SQLite Store — Persistent mission storage
// ═══════════════════════════════════════════════════════════
//
// Replaces the in-memory DashMap<String, Mission> with SQLite.
// Missions survive server restarts. Same API surface as before.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json;
use tokio::sync::{Mutex, broadcast};
use tracing::{info, warn};

use zeus_prometheus::MissionCheckpointer;

use super::pantheon::*;

/// Versioned schema migrations for the Pantheon SQLite database.
///
/// v1: Initial schema — all tables + indexes.
/// v2: Add `reply_to` column to room_messages; normalise room_type values.
const PANTHEON_MIGRATIONS: &[&str] = &[
    // v1 — initial schema
    "CREATE TABLE IF NOT EXISTS missions (
        id TEXT PRIMARY KEY,
        goal TEXT NOT NULL,
        status TEXT NOT NULL DEFAULT 'created',
        progress_pct REAL NOT NULL DEFAULT 0.0,
        tasks_done INTEGER NOT NULL DEFAULT 0,
        tasks_total INTEGER NOT NULL DEFAULT 0,
        tokens_used INTEGER NOT NULL DEFAULT 0,
        budget_tokens INTEGER,
        timeout_seconds INTEGER,
        max_agents INTEGER,
        require_review INTEGER NOT NULL DEFAULT 0,
        summary TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        completed_at TEXT
    );
    CREATE TABLE IF NOT EXISTS mission_team (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        mission_id TEXT NOT NULL REFERENCES missions(id) ON DELETE CASCADE,
        agent_id TEXT NOT NULL,
        name TEXT NOT NULL,
        role TEXT NOT NULL,
        status TEXT NOT NULL DEFAULT 'idle',
        model TEXT
    );
    CREATE TABLE IF NOT EXISTS mission_tasks (
        id TEXT PRIMARY KEY,
        mission_id TEXT NOT NULL REFERENCES missions(id) ON DELETE CASCADE,
        description TEXT NOT NULL,
        assigned_to TEXT,
        status TEXT NOT NULL DEFAULT 'pending',
        result TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS mission_feed (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        mission_id TEXT NOT NULL REFERENCES missions(id) ON DELETE CASCADE,
        agent_id TEXT NOT NULL,
        agent_name TEXT NOT NULL,
        activity TEXT NOT NULL,
        detail TEXT NOT NULL DEFAULT '{}',
        timestamp TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS mission_artifacts (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        mission_id TEXT NOT NULL REFERENCES missions(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        path TEXT NOT NULL,
        artifact_type TEXT NOT NULL,
        created_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS rooms (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        description TEXT,
        room_type TEXT NOT NULL DEFAULT 'public',
        mission_id TEXT,
        created_by TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS room_members (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
        agent_id TEXT NOT NULL,
        agent_name TEXT NOT NULL,
        role TEXT NOT NULL DEFAULT 'member',
        joined_at TEXT NOT NULL,
        UNIQUE(room_id, agent_id)
    );
    CREATE TABLE IF NOT EXISTS room_messages (
        id TEXT PRIMARY KEY,
        room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
        sender_id TEXT NOT NULL,
        sender_name TEXT NOT NULL,
        content TEXT NOT NULL,
        message_type TEXT NOT NULL DEFAULT 'chat',
        metadata TEXT,
        reply_to TEXT,
        timestamp TEXT NOT NULL,
        edited BOOLEAN NOT NULL DEFAULT 0,
        attachments TEXT
    );
    CREATE TABLE IF NOT EXISTS room_reactions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        message_id TEXT NOT NULL REFERENCES room_messages(id) ON DELETE CASCADE,
        room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
        agent_id TEXT NOT NULL,
        agent_name TEXT NOT NULL,
        emoji TEXT NOT NULL,
        created_at TEXT NOT NULL,
        UNIQUE(message_id, agent_id, emoji)
    );
    CREATE TABLE IF NOT EXISTS identities (
        agent_id TEXT PRIMARY KEY,
        display_name TEXT NOT NULL,
        nickname TEXT,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS pending_approvals (
        plan_id TEXT PRIMARY KEY,
        room_id TEXT NOT NULL,
        requested_by TEXT NOT NULL,
        requested_by_name TEXT NOT NULL,
        goal TEXT NOT NULL,
        complexity TEXT NOT NULL,
        risk TEXT NOT NULL,
        steps TEXT NOT NULL,
        status TEXT NOT NULL DEFAULT 'awaiting_approval',
        revision INTEGER NOT NULL DEFAULT 1,
        resolved_by TEXT,
        resolved_by_name TEXT,
        reject_reason TEXT,
        spawn_task TEXT NOT NULL,
        created_at TEXT NOT NULL,
        resolved_at TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_missions_status ON missions(status);
    CREATE INDEX IF NOT EXISTS idx_missions_created ON missions(created_at);
    CREATE INDEX IF NOT EXISTS idx_team_mission ON mission_team(mission_id);
    CREATE INDEX IF NOT EXISTS idx_tasks_mission ON mission_tasks(mission_id);
    CREATE INDEX IF NOT EXISTS idx_feed_mission ON mission_feed(mission_id);
    CREATE INDEX IF NOT EXISTS idx_artifacts_mission ON mission_artifacts(mission_id);
    CREATE INDEX IF NOT EXISTS idx_rooms_type ON rooms(room_type);
    CREATE INDEX IF NOT EXISTS idx_room_members_room ON room_members(room_id);
    CREATE INDEX IF NOT EXISTS idx_room_messages_room ON room_messages(room_id);
    CREATE INDEX IF NOT EXISTS idx_room_messages_ts ON room_messages(timestamp);
    CREATE INDEX IF NOT EXISTS idx_room_reactions_msg ON room_reactions(message_id);
    CREATE INDEX IF NOT EXISTS idx_room_reactions_room ON room_reactions(room_id);
    CREATE INDEX IF NOT EXISTS idx_pending_approvals_room ON pending_approvals(room_id);
    CREATE INDEX IF NOT EXISTS idx_pending_approvals_status ON pending_approvals(status);",
    // v2 — add reply_to column (no-op on fresh DBs; duplicate-column error skipped on old ones)
    "ALTER TABLE room_messages ADD COLUMN reply_to TEXT;
     UPDATE rooms SET room_type = 'public' WHERE room_type = 'mission';",
    // v3 — add edited flag (no-op on fresh DBs; duplicate-column error skipped on old ones)
    "ALTER TABLE room_messages ADD COLUMN edited INTEGER NOT NULL DEFAULT 0;",
    // v4 — link pending approvals to missions for supervisor pipeline
    "ALTER TABLE pending_approvals ADD COLUMN mission_id TEXT;",
    // v5 — file attachments on room messages (JSON array)
    "ALTER TABLE room_messages ADD COLUMN attachments TEXT;",
    // v6 — agent zone persistence (S66-P4B)
    "CREATE TABLE IF NOT EXISTS agent_zones (
        agent_id TEXT PRIMARY KEY,
        zone TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );",
];

/// SQLite-backed store for Pantheon missions.
/// Thread-safe via `Arc<Mutex<Connection>>` + broadcast for events.
#[derive(Clone)]
pub struct PantheonStore {
    db: Arc<Mutex<Connection>>,
    pub broadcast: Arc<broadcast::Sender<PantheonEvent>>,
}

impl PantheonStore {
    /// Open (or create) the Pantheon SQLite database at the given path.
    pub fn new(db_path: &PathBuf) -> Result<Self, String> {
        let conn =
            Connection::open(db_path).map_err(|e| format!("Failed to open pantheon db: {}", e))?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=ON;",
        )
        .map_err(|e| format!("Failed to set pragmas: {}", e))?;

        crate::db::run_migrations(&conn, PANTHEON_MIGRATIONS)
            .map_err(|e| format!("Pantheon schema migration failed: {e}"))?;

        let (tx, _) = broadcast::channel(256);

        info!("Pantheon SQLite store initialized at {:?}", db_path);

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
            broadcast: Arc::new(tx),
        })
    }

    /// In-memory fallback (for tests or when no path configured).
    pub fn in_memory() -> Result<Self, String> {
        let path = PathBuf::from(":memory:");
        Self::new(&path)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PantheonEvent> {
        self.broadcast.subscribe()
    }

    pub fn emit(&self, event: PantheonEvent) {
        let _ = self.broadcast.send(event);
    }

    /// Insert a new mission with its team, tasks, feed, and artifacts.
    pub async fn insert(&self, mission: Mission) {
        let db = self.db.lock().await;
        if let Err(e) = Self::insert_mission_inner(&db, &mission) {
            warn!("Failed to insert mission {}: {}", mission.id, e);
        }
    }

    fn insert_mission_inner(conn: &Connection, m: &Mission) -> Result<(), String> {
        conn.execute(
            "INSERT OR REPLACE INTO missions
             (id, goal, status, progress_pct, tasks_done, tasks_total, tokens_used,
              budget_tokens, timeout_seconds, max_agents, require_review,
              summary, created_at, updated_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                m.id,
                m.goal,
                serde_json::to_string(&m.status)
                    .unwrap_or_else(|_| "\"created\"".to_string())
                    .trim_matches('"'),
                m.progress_pct,
                m.tasks_done as i64,
                m.tasks_total as i64,
                m.tokens_used as i64,
                m.constraints.budget_tokens.map(|v| v as i64),
                m.constraints.timeout_seconds.map(|v| v as i64),
                m.constraints.max_agents.map(|v| v as i64),
                m.constraints.require_review.unwrap_or(false) as i32,
                m.summary,
                m.created_at.to_rfc3339(),
                m.updated_at.to_rfc3339(),
                m.completed_at.map(|t| t.to_rfc3339()),
            ],
        )
        .map_err(|e| format!("Insert mission: {}", e))?;

        // Delete existing child rows (for REPLACE case)
        conn.execute(
            "DELETE FROM mission_team WHERE mission_id = ?1",
            params![m.id],
        )
        .map_err(|e| format!("Delete team: {}", e))?;
        conn.execute(
            "DELETE FROM mission_tasks WHERE mission_id = ?1",
            params![m.id],
        )
        .map_err(|e| format!("Delete tasks: {}", e))?;
        conn.execute(
            "DELETE FROM mission_feed WHERE mission_id = ?1",
            params![m.id],
        )
        .map_err(|e| format!("Delete feed: {}", e))?;
        conn.execute(
            "DELETE FROM mission_artifacts WHERE mission_id = ?1",
            params![m.id],
        )
        .map_err(|e| format!("Delete artifacts: {}", e))?;

        // Insert team members
        for t in &m.team {
            conn.execute(
                "INSERT INTO mission_team (mission_id, agent_id, name, role, status, model)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    m.id,
                    t.agent_id,
                    t.name,
                    serde_json::to_string(&t.role)
                        .unwrap_or_else(|_| "\"worker\"".to_string())
                        .trim_matches('"'),
                    serde_json::to_string(&t.status)
                        .unwrap_or_else(|_| "\"idle\"".to_string())
                        .trim_matches('"'),
                    t.model,
                ],
            )
            .map_err(|e| format!("Insert team member: {}", e))?;
        }

        // Insert tasks
        for task in &m.tasks {
            conn.execute(
                "INSERT INTO mission_tasks (id, mission_id, description, assigned_to, status, result, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    task.id, m.id, task.description, task.assigned_to,
                    serde_json::to_string(&task.status).unwrap_or_else(|_| "\"pending\"".to_string()).trim_matches('"'),
                    task.result,
                    task.created_at.to_rfc3339(),
                    task.updated_at.to_rfc3339(),
                ],
            ).map_err(|e| format!("Insert task: {}", e))?;
        }

        // Insert feed entries
        for f in &m.feed {
            conn.execute(
                "INSERT INTO mission_feed (mission_id, agent_id, agent_name, activity, detail, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    m.id, f.agent_id, f.agent_name, f.activity,
                    serde_json::to_string(&f.detail).unwrap_or_else(|_| "{}".to_string()),
                    f.timestamp.to_rfc3339(),
                ],
            ).map_err(|e| format!("Insert feed entry: {}", e))?;
        }

        // Insert artifacts
        for a in &m.artifacts {
            conn.execute(
                "INSERT INTO mission_artifacts (mission_id, name, path, artifact_type, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    m.id,
                    a.name,
                    a.path,
                    a.artifact_type,
                    a.created_at.to_rfc3339()
                ],
            )
            .map_err(|e| format!("Insert artifact: {}", e))?;
        }

        Ok(())
    }

    /// Get a mission by ID with all child data.
    pub async fn get(&self, id: &str) -> Option<Mission> {
        let db = self.db.lock().await;
        Self::load_mission(&db, id).ok()
    }

    /// List all missions ordered by created_at DESC.
    pub async fn list(&self) -> Vec<Mission> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare("SELECT id FROM missions ORDER BY created_at DESC") {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to list missions: {}", e);
                return Vec::new();
            }
        };

        let ids: Vec<String> = match stmt.query_map([], |row| row.get(0)) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(e) => {
                warn!("Failed to query mission ids: {}", e);
                return Vec::new();
            }
        };

        ids.iter()
            .filter_map(|id| Self::load_mission(&db, id).ok())
            .collect()
    }

    /// Update a mission in-place. Loads from DB, applies closure, writes back.
    pub async fn update<F>(&self, id: &str, f: F) -> bool
    where
        F: FnOnce(&mut Mission),
    {
        let db = self.db.lock().await;
        match Self::load_mission(&db, id) {
            Ok(mut mission) => {
                f(&mut mission);
                mission.updated_at = Utc::now();
                if let Err(e) = Self::insert_mission_inner(&db, &mission) {
                    warn!("Failed to update mission {}: {}", id, e);
                    return false;
                }
                true
            }
            Err(_) => false,
        }
    }

    /// Load a full Mission from the database (mission + team + tasks + feed + artifacts).
    fn load_mission(conn: &Connection, id: &str) -> Result<Mission, String> {
        let mut stmt = conn
            .prepare(
                "SELECT id, goal, status, progress_pct, tasks_done, tasks_total, tokens_used,
                    budget_tokens, timeout_seconds, max_agents, require_review,
                    summary, created_at, updated_at, completed_at
             FROM missions WHERE id = ?1",
            )
            .map_err(|e| format!("Prepare mission query: {}", e))?;

        let mission_row = stmt
            .query_row(params![id], |row| {
                Ok((
                    row.get::<_, String>(0)?,          // id
                    row.get::<_, String>(1)?,          // goal
                    row.get::<_, String>(2)?,          // status
                    row.get::<_, f64>(3)?,             // progress_pct
                    row.get::<_, i64>(4)?,             // tasks_done
                    row.get::<_, i64>(5)?,             // tasks_total
                    row.get::<_, i64>(6)?,             // tokens_used
                    row.get::<_, Option<i64>>(7)?,     // budget_tokens
                    row.get::<_, Option<i64>>(8)?,     // timeout_seconds
                    row.get::<_, Option<i64>>(9)?,     // max_agents
                    row.get::<_, i32>(10)?,            // require_review
                    row.get::<_, Option<String>>(11)?, // summary
                    row.get::<_, String>(12)?,         // created_at
                    row.get::<_, String>(13)?,         // updated_at
                    row.get::<_, Option<String>>(14)?, // completed_at
                ))
            })
            .map_err(|_| format!("Mission {} not found", id))?;

        let status = parse_mission_status(&mission_row.2);
        let created_at = chrono::DateTime::parse_from_rfc3339(&mission_row.12)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let updated_at = chrono::DateTime::parse_from_rfc3339(&mission_row.13)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let completed_at = mission_row.14.as_ref().and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        });

        // Load team
        let team = Self::load_team(conn, id)?;

        // Load tasks
        let tasks = Self::load_tasks(conn, id)?;

        // Load feed
        let feed = Self::load_feed(conn, id)?;

        // Load artifacts
        let artifacts = Self::load_artifacts(conn, id)?;

        Ok(Mission {
            id: mission_row.0,
            goal: mission_row.1,
            status,
            team,
            tasks,
            progress_pct: mission_row.3,
            tasks_done: mission_row.4 as usize,
            tasks_total: mission_row.5 as usize,
            tokens_used: mission_row.6 as u64,
            constraints: MissionConstraints {
                budget_tokens: mission_row.7.map(|v| v as u64),
                timeout_seconds: mission_row.8.map(|v| v as u64),
                max_agents: mission_row.9.map(|v| v as usize),
                require_review: Some(mission_row.10 != 0),
            },
            feed,
            artifacts,
            created_at,
            updated_at,
            completed_at,
            summary: mission_row.11,
        })
    }

    fn load_team(conn: &Connection, mission_id: &str) -> Result<Vec<TeamMember>, String> {
        let mut stmt = conn.prepare(
            "SELECT agent_id, name, role, status, model FROM mission_team WHERE mission_id = ?1"
        ).map_err(|e| format!("Prepare team query: {}", e))?;

        let rows = stmt
            .query_map(params![mission_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|e| format!("Query team: {}", e))?;

        Ok(rows
            .filter_map(|r| r.ok())
            .map(|(agent_id, name, role, status, model)| TeamMember {
                agent_id,
                name,
                role: parse_agent_role(&role),
                status: parse_agent_status(&status),
                model,
            })
            .collect())
    }

    fn load_tasks(conn: &Connection, mission_id: &str) -> Result<Vec<MissionTask>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT id, description, assigned_to, status, result, created_at, updated_at
             FROM mission_tasks WHERE mission_id = ?1",
            )
            .map_err(|e| format!("Prepare tasks query: {}", e))?;

        let rows = stmt
            .query_map(params![mission_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(|e| format!("Query tasks: {}", e))?;

        Ok(rows
            .filter_map(|r| r.ok())
            .map(
                |(id, desc, assigned, status, result, created, updated)| MissionTask {
                    id,
                    description: desc,
                    assigned_to: assigned,
                    status: parse_task_status(&status),
                    result,
                    created_at: chrono::DateTime::parse_from_rfc3339(&created)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: chrono::DateTime::parse_from_rfc3339(&updated)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                },
            )
            .collect())
    }

    fn load_feed(conn: &Connection, mission_id: &str) -> Result<Vec<ActivityEntry>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT agent_id, agent_name, activity, detail, timestamp
             FROM mission_feed WHERE mission_id = ?1 ORDER BY id ASC",
            )
            .map_err(|e| format!("Prepare feed query: {}", e))?;

        let rows = stmt
            .query_map(params![mission_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| format!("Query feed: {}", e))?;

        Ok(rows
            .filter_map(|r| r.ok())
            .map(
                |(agent_id, agent_name, activity, detail, ts)| ActivityEntry {
                    agent_id,
                    agent_name,
                    activity,
                    detail: serde_json::from_str(&detail).unwrap_or(serde_json::Value::Null),
                    timestamp: chrono::DateTime::parse_from_rfc3339(&ts)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                },
            )
            .collect())
    }

    fn load_artifacts(conn: &Connection, mission_id: &str) -> Result<Vec<Artifact>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT name, path, artifact_type, created_at
             FROM mission_artifacts WHERE mission_id = ?1 ORDER BY id ASC",
            )
            .map_err(|e| format!("Prepare artifacts query: {}", e))?;

        let rows = stmt
            .query_map(params![mission_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| format!("Query artifacts: {}", e))?;

        Ok(rows
            .filter_map(|r| r.ok())
            .map(|(name, path, atype, created)| Artifact {
                name,
                path,
                artifact_type: atype,
                created_at: chrono::DateTime::parse_from_rfc3339(&created)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
            .collect())
    }

    /// Count missions by status.
    pub async fn count_by_status(&self) -> Vec<(String, usize)> {
        let db = self.db.lock().await;
        let mut stmt = match db
            .prepare("SELECT status, COUNT(*) FROM missions GROUP BY status ORDER BY COUNT(*) DESC")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        match stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// List missions filtered by status.
    pub async fn list_by_status(&self, status: &str) -> Vec<Mission> {
        let db = self.db.lock().await;
        let mut stmt = match db
            .prepare("SELECT id FROM missions WHERE status = ?1 ORDER BY created_at DESC")
        {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to list missions by status: {}", e);
                return Vec::new();
            }
        };

        let ids: Vec<String> = match stmt.query_map(params![status], |row| row.get(0)) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(e) => {
                warn!("Failed to query missions by status: {}", e);
                return Vec::new();
            }
        };

        ids.iter()
            .filter_map(|id| Self::load_mission(&db, id).ok())
            .collect()
    }

    /// Recover stale missions on gateway startup.
    ///
    /// Finds all missions in `Executing` or `Assembling` state whose `updated_at`
    /// is older than `stale_threshold`. Marks them as `Failed`, fails their
    /// in-progress tasks, adds a recovery activity entry, and emits `MissionFailed`.
    ///
    /// Returns the list of recovered mission IDs for logging.
    pub async fn recover_stale_missions(
        &self,
        stale_threshold: std::time::Duration,
    ) -> Vec<String> {
        let db = self.db.lock().await;
        let cutoff = Utc::now() - chrono::Duration::from_std(stale_threshold).unwrap_or_default();
        let cutoff_str = cutoff.to_rfc3339();

        // Find stale executing/assembling missions
        let mut stmt = match db.prepare(
            "SELECT id FROM missions WHERE status IN ('executing', 'assembling') AND updated_at < ?1",
        ) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to query stale missions: {}", e);
                return Vec::new();
            }
        };

        let stale_ids: Vec<String> = match stmt.query_map(params![cutoff_str], |row| row.get(0)) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(e) => {
                warn!("Failed to fetch stale mission ids: {}", e);
                return Vec::new();
            }
        };
        drop(stmt);

        let mut recovered = Vec::new();
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        for id in &stale_ids {
            // Mark mission as failed
            let _ = db.execute(
                "UPDATE missions SET status = 'failed', updated_at = ?1, completed_at = ?2,
                 summary = 'Gateway restarted during execution' WHERE id = ?3",
                params![now_str, now_str, id],
            );

            // Fail all in-progress tasks
            let _ = db.execute(
                "UPDATE mission_tasks SET status = 'failed', result = 'Gateway restarted',
                 updated_at = ?1 WHERE mission_id = ?2 AND status IN ('in_progress', 'pending')",
                params![now_str, id],
            );

            // Add recovery activity entry
            let _ = db.execute(
                "INSERT INTO mission_feed (mission_id, agent_id, agent_name, activity, detail, timestamp)
                 VALUES (?1, 'system', 'System', 'recovery', ?2, ?3)",
                params![
                    id,
                    serde_json::json!({
                        "reason": "Gateway restarted during execution",
                        "type": "stale_mission_recovery"
                    }).to_string(),
                    now_str,
                ],
            );

            // Emit MissionFailed event
            self.emit(PantheonEvent::MissionFailed {
                mission_id: id.clone(),
                reason: "Gateway restarted during execution".to_string(),
            });

            info!(mission_id = %id, "Recovered stale mission → marked Failed");
            recovered.push(id.clone());
        }

        recovered
    }

    /// Check for missions that have exceeded their timeout and auto-fail them.
    ///
    /// Looks at `Executing` missions whose elapsed time exceeds either their
    /// configured `timeout_seconds` or the provided `default_timeout`.
    /// Returns the list of timed-out mission IDs.
    pub async fn check_mission_timeouts(
        &self,
        default_timeout: std::time::Duration,
    ) -> Vec<String> {
        let db = self.db.lock().await;
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Get all executing missions
        let mut stmt = match db.prepare(
            "SELECT id, timeout_seconds, created_at FROM missions WHERE status = 'executing'",
        ) {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    "Failed to query executing missions for timeout check: {}",
                    e
                );
                return Vec::new();
            }
        };

        let missions: Vec<(String, Option<i64>, String)> = match stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
            ))
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => return Vec::new(),
        };
        drop(stmt);

        let mut timed_out = Vec::new();

        for (id, timeout_secs, created_str) in &missions {
            let created = chrono::DateTime::parse_from_rfc3339(created_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or(now);

            let timeout = timeout_secs
                .map(|s| std::time::Duration::from_secs(s as u64))
                .unwrap_or(default_timeout);

            let elapsed = (now - created).to_std().unwrap_or_default();
            if elapsed <= timeout {
                continue;
            }

            // Timed out — mark as failed
            let _ = db.execute(
                "UPDATE missions SET status = 'failed', updated_at = ?1, completed_at = ?2,
                 summary = 'Mission exceeded timeout' WHERE id = ?3",
                params![now_str, now_str, id],
            );

            let _ = db.execute(
                "UPDATE mission_tasks SET status = 'failed', result = 'Mission timed out',
                 updated_at = ?1 WHERE mission_id = ?2 AND status IN ('in_progress', 'pending')",
                params![now_str, id],
            );

            let _ = db.execute(
                "INSERT INTO mission_feed (mission_id, agent_id, agent_name, activity, detail, timestamp)
                 VALUES (?1, 'system', 'System', 'timeout', ?2, ?3)",
                params![
                    id,
                    serde_json::json!({
                        "reason": "Mission exceeded timeout",
                        "timeout_seconds": timeout.as_secs(),
                        "elapsed_seconds": elapsed.as_secs(),
                    }).to_string(),
                    now_str,
                ],
            );

            self.emit(PantheonEvent::MissionFailed {
                mission_id: id.clone(),
                reason: format!(
                    "Mission exceeded timeout ({}s elapsed, {}s limit)",
                    elapsed.as_secs(),
                    timeout.as_secs()
                ),
            });

            info!(mission_id = %id, elapsed_secs = elapsed.as_secs(), "Mission timed out → marked Failed");
            timed_out.push(id.clone());
        }

        timed_out
    }

    /// Spawn a background task that checks for timed-out missions every 60 seconds.
    /// Returns a `JoinHandle` that lives as long as the gateway.
    pub fn start_timeout_check_task(
        store: PantheonStore,
        default_timeout: std::time::Duration,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                let timed_out = store.check_mission_timeouts(default_timeout).await;
                if !timed_out.is_empty() {
                    info!(
                        "Mission timeout check: {} missions auto-failed",
                        timed_out.len()
                    );
                }
            }
        })
    }

    /// Total mission count.
    pub async fn count(&self) -> usize {
        let db = self.db.lock().await;
        db.query_row("SELECT COUNT(*) FROM missions", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap_or(0) as usize
    }

    // ─── Room operations ─────────────────────────────────────

    /// Insert a new room.
    pub async fn insert_room(&self, room: &super::pantheon::Room) {
        let db = self.db.lock().await;
        if let Err(e) = db.execute(
            "INSERT INTO rooms (id, name, description, room_type, mission_id, created_by, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                room.id,
                room.name,
                room.description,
                room.room_type.to_string(),
                room.mission_id,
                room.created_by,
                room.created_at.to_rfc3339(),
                room.updated_at.to_rfc3339(),
            ],
        ) {
            warn!("Failed to insert room {}: {}", room.id, e);
        }
    }

    /// Get a room by ID.
    pub async fn get_room(&self, id: &str) -> Option<super::pantheon::Room> {
        let db = self.db.lock().await;
        Self::load_room(&db, id).ok()
    }

    // ── Agent Zone Persistence (S66-P4B) ─────────────────────────────────

    /// Save an agent's zone assignment to SQLite.
    pub async fn save_agent_zone(&self, agent_id: &str, zone: &str) {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        if let Err(e) = db.execute(
            "INSERT OR REPLACE INTO agent_zones (agent_id, zone, updated_at) VALUES (?1, ?2, ?3)",
            params![agent_id, zone, now],
        ) {
            warn!("Failed to persist agent zone: {}", e);
        }
    }

    /// Load all persisted agent zone assignments.
    pub async fn load_agent_zones(&self) -> Vec<(String, String)> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare("SELECT agent_id, zone FROM agent_zones") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// List all rooms (public rooms first, then by created_at DESC).
    pub async fn list_rooms(&self) -> Vec<super::pantheon::Room> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id FROM rooms ORDER BY
             CASE room_type WHEN 'public' THEN 0 ELSE 1 END,
             created_at DESC",
        ) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to list rooms: {}", e);
                return Vec::new();
            }
        };

        let ids: Vec<String> = match stmt.query_map([], |row| row.get(0)) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => return Vec::new(),
        };

        ids.iter()
            .filter_map(|id| Self::load_room(&db, id).ok())
            .collect()
    }

    fn load_room(conn: &Connection, id: &str) -> Result<super::pantheon::Room, String> {
        conn.query_row(
            "SELECT id, name, description, room_type, mission_id, created_by, created_at, updated_at
             FROM rooms WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .map(|(id, name, desc, rtype, mid, created_by, created, updated)| {
            super::pantheon::Room {
                id,
                name,
                description: desc,
                room_type: match rtype.as_str() {
                    "public" | "mission" => super::pantheon::RoomType::Public,
                    "private" => super::pantheon::RoomType::Private,
                    "dm" => super::pantheon::RoomType::Dm,
                    _ => super::pantheon::RoomType::Public,
                },
                mission_id: mid,
                created_by,
                created_at: chrono::DateTime::parse_from_rfc3339(&created)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: chrono::DateTime::parse_from_rfc3339(&updated)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            }
        })
        .map_err(|_| format!("Room {} not found", id))
    }

    /// Find an existing DM room between two agents (order-independent).
    pub async fn find_dm_room(&self, agent1: &str, agent2: &str) -> Option<super::pantheon::Room> {
        let db = self.db.lock().await;
        // Find a room of type "dm" that has both agents as members
        let result = db.query_row(
            "SELECT r.id FROM rooms r
             JOIN room_members m1 ON m1.room_id = r.id AND m1.agent_id = ?1
             JOIN room_members m2 ON m2.room_id = r.id AND m2.agent_id = ?2
             WHERE r.room_type = 'dm'
             LIMIT 1",
            params![agent1, agent2],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(id) => Self::load_room(&db, &id).ok(),
            Err(_) => None,
        }
    }

    /// List all DM rooms where the given agent is a member, ordered by most recent activity.
    pub async fn list_dms_for_agent(&self, agent_id: &str) -> Vec<super::pantheon::Room> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT r.id FROM rooms r
             JOIN room_members m ON m.room_id = r.id AND m.agent_id = ?1
             WHERE r.room_type = 'dm'
             ORDER BY r.updated_at DESC",
        ) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to list DMs for {}: {}", agent_id, e);
                return Vec::new();
            }
        };

        let ids: Vec<String> = match stmt.query_map(params![agent_id], |row| row.get(0)) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => return Vec::new(),
        };

        ids.iter()
            .filter_map(|id| Self::load_room(&db, id).ok())
            .collect()
    }

    /// Add a member to a room.
    pub async fn join_room(&self, room_id: &str, member: &super::pantheon::RoomMember) {
        let db = self.db.lock().await;
        if let Err(e) = db.execute(
            "INSERT OR IGNORE INTO room_members (room_id, agent_id, agent_name, role, joined_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                room_id,
                member.agent_id,
                member.agent_name,
                member.role,
                member.joined_at.to_rfc3339(),
            ],
        ) {
            warn!("Failed to join room {}: {}", room_id, e);
        }
    }

    /// Remove a member from a room.
    pub async fn leave_room(&self, room_id: &str, agent_id: &str) {
        let db = self.db.lock().await;
        let _ = db.execute(
            "DELETE FROM room_members WHERE room_id = ?1 AND agent_id = ?2",
            params![room_id, agent_id],
        );
    }

    /// List members of a room.
    pub async fn list_room_members(&self, room_id: &str) -> Vec<super::pantheon::RoomMember> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT agent_id, agent_name, role, joined_at FROM room_members WHERE room_id = ?1 ORDER BY joined_at ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        match stmt.query_map(params![room_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        }) {
            Ok(rows) => rows
                .filter_map(|r| r.ok())
                .map(
                    |(agent_id, agent_name, role, joined)| super::pantheon::RoomMember {
                        agent_id,
                        agent_name,
                        role,
                        joined_at: chrono::DateTime::parse_from_rfc3339(&joined)
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now()),
                    },
                )
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Insert a message into a room.
    pub async fn insert_room_message(&self, msg: &super::pantheon::RoomMessage) {
        let db = self.db.lock().await;
        let metadata_str = msg
            .metadata
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_default());
        let attachments_str = if msg.attachments.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&msg.attachments).unwrap_or_default())
        };
        if let Err(e) = db.execute(
            "INSERT INTO room_messages (id, room_id, sender_id, sender_name, content, message_type, metadata, reply_to, timestamp, edited, attachments)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                msg.id,
                msg.room_id,
                msg.sender_id,
                msg.sender_name,
                msg.content,
                msg.message_type,
                metadata_str,
                msg.reply_to,
                msg.timestamp.to_rfc3339(),
                msg.edited,
                attachments_str,
            ],
        ) {
            warn!("Failed to insert room message {}: {}", msg.id, e);
        }
    }

    /// Get messages from a room (paginated, newest first).
    pub async fn get_room_messages(
        &self,
        room_id: &str,
        limit: usize,
        before: Option<&str>,
    ) -> Vec<super::pantheon::RoomMessage> {
        let db = self.db.lock().await;
        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(
            before_ts,
        ) = before
        {
            (
                "SELECT id, room_id, sender_id, sender_name, content, message_type, metadata, timestamp, reply_to, edited, attachments
                 FROM room_messages WHERE room_id = ?1 AND timestamp < ?2
                 ORDER BY timestamp DESC LIMIT ?3".to_string(),
                vec![
                    Box::new(room_id.to_string()) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(before_ts.to_string()),
                    Box::new(limit as i64),
                ],
            )
        } else {
            (
                "SELECT id, room_id, sender_id, sender_name, content, message_type, metadata, timestamp, reply_to, edited, attachments
                 FROM room_messages WHERE room_id = ?1
                 ORDER BY timestamp DESC LIMIT ?2".to_string(),
                vec![
                    Box::new(room_id.to_string()) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(limit as i64),
                ],
            )
        };

        let mut stmt = match db.prepare(&sql) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let mut messages: Vec<super::pantheon::RoomMessage> =
            match stmt.query_map(params_refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, bool>(9).unwrap_or(false),
                    row.get::<_, Option<String>>(10).unwrap_or(None),
                ))
            }) {
                Ok(rows) => rows
                    .filter_map(|r| r.ok())
                    .map(
                        |(
                            id,
                            room_id,
                            sender_id,
                            sender_name,
                            content,
                            msg_type,
                            metadata,
                            ts,
                            reply_to,
                            edited,
                            attachments_json,
                        )| {
                            let attachments: Vec<super::pantheon::MessageAttachment> = attachments_json
                                .and_then(|s| serde_json::from_str(&s).ok())
                                .unwrap_or_default();
                            super::pantheon::RoomMessage {
                                id,
                                room_id,
                                sender_id,
                                sender_name,
                                content,
                                message_type: msg_type,
                                metadata: metadata.and_then(|s| serde_json::from_str(&s).ok()),
                                reply_to,
                                edited,
                                attachments,
                                timestamp: chrono::DateTime::parse_from_rfc3339(&ts)
                                    .map(|dt| dt.with_timezone(&Utc))
                                    .unwrap_or_else(|_| Utc::now()),
                            }
                        },
                    )
                    .collect(),
                Err(_) => Vec::new(),
            };

        // Reverse to return oldest-first (chat order)
        messages.reverse();
        messages
    }

    /// Create a room for a mission. Room is Public — missions don't gate room access.
    pub async fn create_mission_room(&self, mission_id: &str, goal: &str) -> String {
        let room_id = format!("{}-room", mission_id);
        let room = super::pantheon::Room {
            id: room_id.clone(),
            name: format!("Mission: {}", &goal[..goal.len().min(50)]),
            description: Some(goal.to_string()),
            room_type: super::pantheon::RoomType::Public,
            mission_id: Some(mission_id.to_string()),
            created_by: "system".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.insert_room(&room).await;
        room_id
    }

    /// Delete a room message by ID. Returns true if a row was deleted.
    pub async fn delete_room_message(&self, message_id: &str) -> bool {
        let db = self.db.lock().await;
        match db.execute(
            "DELETE FROM room_messages WHERE id = ?1",
            params![message_id],
        ) {
            Ok(n) => n > 0,
            Err(e) => {
                warn!("Failed to delete room message {}: {}", message_id, e);
                false
            }
        }
    }

    /// Edit a room message's content. Returns true if updated.
    pub async fn edit_room_message(&self, message_id: &str, new_content: &str) -> bool {
        let db = self.db.lock().await;
        match db.execute(
            "UPDATE room_messages SET content = ?1, edited = 1 WHERE id = ?2",
            params![new_content, message_id],
        ) {
            Ok(n) => n > 0,
            Err(e) => {
                warn!("Failed to edit room message {}: {}", message_id, e);
                false
            }
        }
    }

    /// Add a reaction to a message. Returns true if inserted (false if duplicate).
    pub async fn add_reaction(
        &self,
        message_id: &str,
        room_id: &str,
        agent_id: &str,
        agent_name: &str,
        emoji: &str,
    ) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        match db.execute(
            "INSERT OR IGNORE INTO room_reactions (message_id, room_id, agent_id, agent_name, emoji, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![message_id, room_id, agent_id, agent_name, emoji, now],
        ) {
            Ok(n) => n > 0,
            Err(e) => {
                warn!("Failed to add reaction to {}: {}", message_id, e);
                false
            }
        }
    }

    /// Remove a reaction from a message. Returns true if removed.
    pub async fn remove_reaction(&self, message_id: &str, agent_id: &str, emoji: &str) -> bool {
        let db = self.db.lock().await;
        match db.execute(
            "DELETE FROM room_reactions WHERE message_id = ?1 AND agent_id = ?2 AND emoji = ?3",
            params![message_id, agent_id, emoji],
        ) {
            Ok(n) => n > 0,
            Err(e) => {
                warn!("Failed to remove reaction from {}: {}", message_id, e);
                false
            }
        }
    }

    /// Get all reactions for a message, grouped by emoji.
    pub async fn get_reactions(&self, message_id: &str) -> Vec<(String, Vec<(String, String)>)> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT emoji, agent_id, agent_name FROM room_reactions WHERE message_id = ?1 ORDER BY emoji, created_at",
        ) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to query reactions for {}: {}", message_id, e);
                return vec![];
            }
        };

        let rows: Vec<(String, String, String)> = match stmt.query_map(params![message_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => return vec![],
        };

        // Group by emoji
        let mut grouped: std::collections::BTreeMap<String, Vec<(String, String)>> =
            std::collections::BTreeMap::new();
        for (emoji, agent_id, agent_name) in rows {
            grouped
                .entry(emoji)
                .or_default()
                .push((agent_id, agent_name));
        }
        grouped.into_iter().collect()
    }

    // ─── Identity operations ─────────────────────────────────

    /// Set or update an identity (display_name + optional nickname).
    pub async fn set_identity(&self, agent_id: &str, display_name: &str, nickname: Option<&str>) {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        if let Err(e) = db.execute(
            "INSERT INTO identities (agent_id, display_name, nickname, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent_id) DO UPDATE SET display_name = ?2, nickname = ?3, updated_at = ?4",
            params![agent_id, display_name, nickname, now],
        ) {
            warn!("Failed to set identity for {}: {}", agent_id, e);
        }
    }

    /// Set just the nickname for an existing identity.
    pub async fn set_nickname(&self, agent_id: &str, sender_name: &str, nickname: &str) {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        // Upsert — if identity doesn't exist yet, create it with sender_name as display_name
        if let Err(e) = db.execute(
            "INSERT INTO identities (agent_id, display_name, nickname, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent_id) DO UPDATE SET nickname = ?3, updated_at = ?4",
            params![agent_id, sender_name, nickname, now],
        ) {
            warn!("Failed to set nickname for {}: {}", agent_id, e);
        }
    }

    /// Get identity for an agent/user.
    pub async fn get_identity(&self, agent_id: &str) -> Option<(String, Option<String>)> {
        let db = self.db.lock().await;
        db.query_row(
            "SELECT display_name, nickname FROM identities WHERE agent_id = ?1",
            params![agent_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .ok()
    }

    /// List all identities.
    pub async fn list_identities(&self) -> Vec<(String, String, Option<String>)> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT agent_id, display_name, nickname FROM identities ORDER BY display_name",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    // ── Pending Approvals (Plan Cards) ──────────────────────────

    /// Insert a pending approval (plan card awaiting human decision).
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_pending_approval(
        &self,
        plan_id: &str,
        room_id: &str,
        requested_by: &str,
        requested_by_name: &str,
        goal: &str,
        complexity: &str,
        risk: &str,
        steps_json: &str,
        spawn_task: &str,
    ) {
        let db = self.db.lock().await;
        if let Err(e) = db.execute(
            "INSERT INTO pending_approvals (plan_id, room_id, requested_by, requested_by_name, goal, complexity, risk, steps, spawn_task, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                plan_id, room_id, requested_by, requested_by_name,
                goal, complexity, risk, steps_json, spawn_task,
                Utc::now().to_rfc3339(),
            ],
        ) {
            warn!("Failed to insert pending approval {}: {}", plan_id, e);
        }
    }

    /// Get a pending approval by plan_id.
    pub async fn get_pending_approval(&self, plan_id: &str) -> Option<PendingApproval> {
        let db = self.db.lock().await;
        let mut stmt = db.prepare(
            "SELECT plan_id, room_id, requested_by, requested_by_name, goal, complexity, risk, steps, status, revision, resolved_by, resolved_by_name, reject_reason, spawn_task, created_at, resolved_at, mission_id
             FROM pending_approvals WHERE plan_id = ?1"
        ).ok()?;
        stmt.query_row(params![plan_id], |row| {
            Ok(PendingApproval {
                plan_id: row.get(0)?,
                room_id: row.get(1)?,
                requested_by: row.get(2)?,
                requested_by_name: row.get(3)?,
                goal: row.get(4)?,
                complexity: row.get(5)?,
                risk: row.get(6)?,
                steps_json: row.get(7)?,
                status: row.get(8)?,
                revision: row.get(9)?,
                resolved_by: row.get(10)?,
                resolved_by_name: row.get(11)?,
                reject_reason: row.get(12)?,
                spawn_task: row.get(13)?,
                created_at: row.get(14)?,
                resolved_at: row.get(15)?,
                mission_id: row.get(16)?,
            })
        })
        .ok()
    }

    /// Approve a pending plan — marks it approved and returns the approval record.
    pub async fn approve_plan(
        &self,
        plan_id: &str,
        approved_by: &str,
        approved_by_name: &str,
    ) -> Option<PendingApproval> {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        match db.execute(
            "UPDATE pending_approvals SET status = 'approved', resolved_by = ?1, resolved_by_name = ?2, resolved_at = ?3
             WHERE plan_id = ?4 AND status = 'awaiting_approval'",
            params![approved_by, approved_by_name, now, plan_id],
        ) {
            Ok(0) => return None, // not found or already resolved
            Ok(_) => {}
            Err(e) => {
                warn!("Failed to approve plan {}: {}", plan_id, e);
                return None;
            }
        }
        drop(db);
        self.get_pending_approval(plan_id).await
    }

    /// Reject a pending plan with an optional reason.
    pub async fn reject_plan(
        &self,
        plan_id: &str,
        rejected_by: &str,
        rejected_by_name: &str,
        reason: Option<&str>,
    ) -> Option<PendingApproval> {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        match db.execute(
            "UPDATE pending_approvals SET status = 'rejected', resolved_by = ?1, resolved_by_name = ?2, reject_reason = ?3, resolved_at = ?4
             WHERE plan_id = ?5 AND status = 'awaiting_approval'",
            params![rejected_by, rejected_by_name, reason, now, plan_id],
        ) {
            Ok(0) => return None,
            Ok(_) => {}
            Err(e) => {
                warn!("Failed to reject plan {}: {}", plan_id, e);
                return None;
            }
        }
        drop(db);
        self.get_pending_approval(plan_id).await
    }

    /// List all pending approvals in a specific room (status = awaiting_approval).
    pub async fn list_pending_approvals(&self, room_id: &str) -> Vec<PendingApproval> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT plan_id, room_id, requested_by, requested_by_name, goal, complexity, risk, steps, status, revision, resolved_by, resolved_by_name, reject_reason, spawn_task, created_at, resolved_at, mission_id
             FROM pending_approvals WHERE room_id = ?1 AND status = 'awaiting_approval'
             ORDER BY created_at DESC"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map(params![room_id], |row| {
            Ok(PendingApproval {
                plan_id: row.get(0)?,
                room_id: row.get(1)?,
                requested_by: row.get(2)?,
                requested_by_name: row.get(3)?,
                goal: row.get(4)?,
                complexity: row.get(5)?,
                risk: row.get(6)?,
                steps_json: row.get(7)?,
                status: row.get(8)?,
                revision: row.get(9)?,
                resolved_by: row.get(10)?,
                resolved_by_name: row.get(11)?,
                reject_reason: row.get(12)?,
                spawn_task: row.get(13)?,
                created_at: row.get(14)?,
                resolved_at: row.get(15)?,
                mission_id: row.get(16)?,
            })
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// Link a mission_id to an approved plan — tracks which mission was spawned.
    pub async fn link_mission_to_approval(&self, plan_id: &str, mission_id: &str) {
        let db = self.db.lock().await;
        if let Err(e) = db.execute(
            "UPDATE pending_approvals SET mission_id = ?1 WHERE plan_id = ?2",
            params![mission_id, plan_id],
        ) {
            warn!(
                "Failed to link mission {} to approval {}: {}",
                mission_id, plan_id, e
            );
        }
    }

    /// Reopen a rejected plan for a new revision with updated steps.
    /// Resets status to `awaiting_approval`, increments revision, clears resolved fields.
    /// Returns None if plan not found or revision limit (3) exceeded.
    pub async fn reopen_plan_for_revision(
        &self,
        plan_id: &str,
        new_steps_json: &str,
        new_goal: &str,
    ) -> Option<PendingApproval> {
        let db = self.db.lock().await;
        // Only reopen rejected plans with revision < 3
        match db.execute(
            "UPDATE pending_approvals
             SET status = 'awaiting_approval',
                 steps = ?1,
                 goal = ?2,
                 revision = revision + 1,
                 resolved_by = NULL,
                 resolved_by_name = NULL,
                 reject_reason = NULL,
                 resolved_at = NULL
             WHERE plan_id = ?3 AND status = 'rejected' AND revision < 3",
            params![new_steps_json, new_goal, plan_id],
        ) {
            Ok(0) => return None,
            Ok(_) => {}
            Err(e) => {
                warn!("Failed to reopen plan {} for revision: {}", plan_id, e);
                return None;
            }
        }
        drop(db);
        self.get_pending_approval(plan_id).await
    }
}

/// A pending plan card approval record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    pub plan_id: String,
    pub room_id: String,
    pub requested_by: String,
    pub requested_by_name: String,
    pub goal: String,
    pub complexity: String,
    pub risk: String,
    pub steps_json: String,
    pub status: String, // "awaiting_approval" | "approved" | "rejected"
    pub revision: i64,
    pub resolved_by: Option<String>,
    pub resolved_by_name: Option<String>,
    pub reject_reason: Option<String>,
    pub spawn_task: String,
    pub created_at: String,
    pub resolved_at: Option<String>,
    /// Linked mission ID — populated when approval triggers execution.
    #[serde(default)]
    pub mission_id: Option<String>,
}

// ─── MissionCheckpointer trait impl ──────────────────────────

impl MissionCheckpointer for PantheonStore {
    fn checkpoint(
        &self,
        mission_id: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        let mid = mission_id.to_string();
        Box::pin(async move {
            // Touch updated_at to keep the mission fresh — prevents recover_stale_missions()
            // from falsely marking an actively-executing mission as stale.
            // Full state sync happens when drive_mission() returns.
            let db = self.db.lock().await;
            let now = Utc::now().to_rfc3339();
            if let Err(e) = db.execute(
                "UPDATE missions SET updated_at = ?1 WHERE id = ?2",
                params![now, mid],
            ) {
                warn!("Checkpoint touch failed for mission {}: {}", mid, e);
            }
        })
    }
}

// ─── Enum parsing helpers ─────────────────────────────────

fn parse_mission_status(s: &str) -> MissionStatus {
    match s {
        "created" => MissionStatus::Created,
        "assembling" => MissionStatus::Assembling,
        "awaiting_approval" => MissionStatus::AwaitingApproval,
        "executing" => MissionStatus::Executing,
        "reviewing" => MissionStatus::Reviewing,
        "complete" => MissionStatus::Complete,
        "failed" => MissionStatus::Failed,
        "cancelled" => MissionStatus::Cancelled,
        _ => MissionStatus::Created,
    }
}

fn parse_agent_role(s: &str) -> AgentRole {
    match s {
        "coordinator" => AgentRole::Coordinator,
        "manager" => AgentRole::Manager,
        "worker" => AgentRole::Worker,
        "reviewer" => AgentRole::Reviewer,
        _ => AgentRole::Worker,
    }
}

fn parse_agent_status(s: &str) -> AgentStatus {
    match s {
        "idle" => AgentStatus::Idle,
        "working" => AgentStatus::Working,
        "blocked" => AgentStatus::Blocked,
        "done" => AgentStatus::Done,
        _ => AgentStatus::Idle,
    }
}

fn parse_task_status(s: &str) -> TaskStatus {
    match s {
        "pending" => TaskStatus::Pending,
        "in_progress" => TaskStatus::InProgress,
        "awaiting_review" => TaskStatus::AwaitingReview,
        "approved" => TaskStatus::Approved,
        "rejected" => TaskStatus::Rejected,
        "complete" => TaskStatus::Complete,
        "failed" => TaskStatus::Failed,
        _ => TaskStatus::Pending,
    }
}

// ═══════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_test_store() -> PantheonStore {
        PantheonStore::in_memory().expect("in-memory store should work")
    }

    fn make_mission(id: &str, goal: &str) -> Mission {
        let now = Utc::now();
        Mission {
            id: id.to_string(),
            goal: goal.to_string(),
            status: MissionStatus::Created,
            team: vec![
                TeamMember {
                    agent_id: "zeus-112".to_string(),
                    name: "Zeus-Coordinator".to_string(),
                    role: AgentRole::Coordinator,
                    status: AgentStatus::Working,
                    model: None,
                },
                TeamMember {
                    agent_id: "zeus-102".to_string(),
                    name: "Zeus-Worker-1".to_string(),
                    role: AgentRole::Worker,
                    status: AgentStatus::Idle,
                    model: Some("sonnet-4.5".to_string()),
                },
            ],
            tasks: vec![MissionTask {
                id: "t-1".to_string(),
                description: "Implement feature".to_string(),
                assigned_to: Some("zeus-102".to_string()),
                status: TaskStatus::Pending,
                result: None,
                created_at: now,
                updated_at: now,
            }],
            progress_pct: 0.0,
            tasks_done: 0,
            tasks_total: 1,
            tokens_used: 0,
            constraints: MissionConstraints {
                budget_tokens: Some(50_000),
                timeout_seconds: Some(600),
                max_agents: Some(4),
                require_review: Some(false),
            },
            feed: vec![ActivityEntry {
                agent_id: "system".to_string(),
                agent_name: "System".to_string(),
                activity: "mission_created".to_string(),
                detail: serde_json::json!({"source": "test"}),
                timestamp: now,
            }],
            artifacts: Vec::new(),
            created_at: now,
            updated_at: now,
            completed_at: None,
            summary: None,
        }
    }

    #[tokio::test]
    async fn test_insert_and_get() {
        let store = make_test_store();
        let mission = make_mission("m-test-1", "Build the thing");
        store.insert(mission.clone()).await;

        let loaded = store.get("m-test-1").await.expect("should find mission");
        assert_eq!(loaded.id, "m-test-1");
        assert_eq!(loaded.goal, "Build the thing");
        assert_eq!(loaded.team.len(), 2);
        assert_eq!(loaded.tasks.len(), 1);
        assert_eq!(loaded.feed.len(), 1);
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let store = make_test_store();
        assert!(store.get("nope").await.is_none());
    }

    #[tokio::test]
    async fn test_list_ordered_by_created() {
        let store = make_test_store();
        store.insert(make_mission("m-a", "First")).await;
        // Small delay to ensure ordering
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        store.insert(make_mission("m-b", "Second")).await;

        let missions = store.list().await;
        assert_eq!(missions.len(), 2);
        assert_eq!(missions[0].id, "m-b"); // newest first
        assert_eq!(missions[1].id, "m-a");
    }

    #[tokio::test]
    async fn test_update() {
        let store = make_test_store();
        store.insert(make_mission("m-upd", "Update me")).await;

        let updated = store
            .update("m-upd", |m| {
                m.status = MissionStatus::Executing;
                m.progress_pct = 50.0;
                m.tasks_done = 1;
            })
            .await;
        assert!(updated);

        let loaded = store.get("m-upd").await.unwrap();
        assert_eq!(loaded.status, MissionStatus::Executing);
        assert_eq!(loaded.progress_pct, 50.0);
        assert_eq!(loaded.tasks_done, 1);
    }

    #[tokio::test]
    async fn test_update_nonexistent() {
        let store = make_test_store();
        let updated = store
            .update("nope", |m| m.status = MissionStatus::Failed)
            .await;
        assert!(!updated);
    }

    #[tokio::test]
    async fn test_team_roundtrip() {
        let store = make_test_store();
        store.insert(make_mission("m-team", "Team test")).await;

        let loaded = store.get("m-team").await.unwrap();
        assert_eq!(loaded.team.len(), 2);
        assert_eq!(loaded.team[0].role, AgentRole::Coordinator);
        assert_eq!(loaded.team[1].role, AgentRole::Worker);
        assert_eq!(loaded.team[1].model, Some("sonnet-4.5".to_string()));
    }

    #[tokio::test]
    async fn test_tasks_roundtrip() {
        let store = make_test_store();
        store.insert(make_mission("m-tasks", "Task test")).await;

        let loaded = store.get("m-tasks").await.unwrap();
        assert_eq!(loaded.tasks.len(), 1);
        assert_eq!(loaded.tasks[0].description, "Implement feature");
        assert_eq!(loaded.tasks[0].assigned_to, Some("zeus-102".to_string()));
    }

    #[tokio::test]
    async fn test_feed_roundtrip() {
        let store = make_test_store();
        store.insert(make_mission("m-feed", "Feed test")).await;

        let loaded = store.get("m-feed").await.unwrap();
        assert_eq!(loaded.feed.len(), 1);
        assert_eq!(loaded.feed[0].activity, "mission_created");
    }

    #[tokio::test]
    async fn test_constraints_roundtrip() {
        let store = make_test_store();
        store.insert(make_mission("m-con", "Constraints")).await;

        let loaded = store.get("m-con").await.unwrap();
        assert_eq!(loaded.constraints.budget_tokens, Some(50_000));
        assert_eq!(loaded.constraints.timeout_seconds, Some(600));
        assert_eq!(loaded.constraints.max_agents, Some(4));
        assert_eq!(loaded.constraints.require_review, Some(false));
    }

    #[tokio::test]
    async fn test_count_and_count_by_status() {
        let store = make_test_store();
        store.insert(make_mission("m-c1", "One")).await;
        store.insert(make_mission("m-c2", "Two")).await;

        assert_eq!(store.count().await, 2);

        let by_status = store.count_by_status().await;
        assert_eq!(by_status.len(), 1); // both "created"
        assert_eq!(by_status[0], ("created".to_string(), 2));
    }

    #[tokio::test]
    async fn test_list_by_status() {
        let store = make_test_store();
        store.insert(make_mission("m-s1", "Created one")).await;

        let mut m2 = make_mission("m-s2", "Executing one");
        m2.status = MissionStatus::Executing;
        store.insert(m2).await;

        let created = store.list_by_status("created").await;
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].id, "m-s1");

        let executing = store.list_by_status("executing").await;
        assert_eq!(executing.len(), 1);
        assert_eq!(executing[0].id, "m-s2");
    }

    #[tokio::test]
    async fn test_complete_lifecycle() {
        let store = make_test_store();

        // Create
        store.insert(make_mission("m-life", "Full lifecycle")).await;

        // Assemble → Execute
        store
            .update("m-life", |m| {
                m.status = MissionStatus::Executing;
                m.team.push(TeamMember {
                    agent_id: "zeus-107".to_string(),
                    name: "Zeus-Reviewer".to_string(),
                    role: AgentRole::Reviewer,
                    status: AgentStatus::Idle,
                    model: None,
                });
            })
            .await;

        // Progress
        store
            .update("m-life", |m| {
                m.progress_pct = 100.0;
                m.tasks_done = 1;
                m.status = MissionStatus::Complete;
                m.completed_at = Some(Utc::now());
                m.summary = Some("All tasks done".to_string());
            })
            .await;

        let loaded = store.get("m-life").await.unwrap();
        assert_eq!(loaded.status, MissionStatus::Complete);
        assert_eq!(loaded.team.len(), 3);
        assert!(loaded.completed_at.is_some());
        assert_eq!(loaded.summary, Some("All tasks done".to_string()));
    }

    #[tokio::test]
    async fn test_recover_stale_missions_marks_failed() {
        let store = make_test_store();

        // Insert a mission in Executing state with old updated_at
        let mut m = make_mission("m-stale", "Stale mission");
        m.status = MissionStatus::Executing;
        m.tasks[0].status = TaskStatus::InProgress;
        store.insert(m).await;

        // Force updated_at to 10 minutes ago
        {
            let db = store.db.lock().await;
            let old = (Utc::now() - chrono::Duration::minutes(10)).to_rfc3339();
            db.execute(
                "UPDATE missions SET updated_at = ?1 WHERE id = 'm-stale'",
                params![old],
            )
            .unwrap();
        }

        // Also insert a fresh executing mission (should NOT be recovered)
        let mut fresh = make_mission("m-fresh", "Fresh mission");
        fresh.status = MissionStatus::Executing;
        store.insert(fresh).await;

        let recovered = store
            .recover_stale_missions(std::time::Duration::from_secs(300))
            .await;

        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0], "m-stale");

        // Verify the stale mission is now Failed
        let loaded = store.get("m-stale").await.unwrap();
        assert_eq!(loaded.status, MissionStatus::Failed);
        assert!(loaded.summary.as_deref() == Some("Gateway restarted during execution"));

        // Verify recovery activity was added
        assert!(loaded.feed.iter().any(|f| f.activity == "recovery"));

        // Verify fresh mission is untouched
        let fresh_loaded = store.get("m-fresh").await.unwrap();
        assert_eq!(fresh_loaded.status, MissionStatus::Executing);
    }

    #[tokio::test]
    async fn test_recover_stale_missions_assembling() {
        let store = make_test_store();

        let mut m = make_mission("m-asm", "Assembling mission");
        m.status = MissionStatus::Assembling;
        store.insert(m).await;

        // Force old timestamp
        {
            let db = store.db.lock().await;
            let old = (Utc::now() - chrono::Duration::minutes(10)).to_rfc3339();
            db.execute(
                "UPDATE missions SET updated_at = ?1 WHERE id = 'm-asm'",
                params![old],
            )
            .unwrap();
        }

        let recovered = store
            .recover_stale_missions(std::time::Duration::from_secs(300))
            .await;
        assert_eq!(recovered, vec!["m-asm"]);

        let loaded = store.get("m-asm").await.unwrap();
        assert_eq!(loaded.status, MissionStatus::Failed);
    }

    #[tokio::test]
    async fn test_recover_stale_missions_none_stale() {
        let store = make_test_store();

        let mut m = make_mission("m-ok", "Recent mission");
        m.status = MissionStatus::Executing;
        store.insert(m).await;

        let recovered = store
            .recover_stale_missions(std::time::Duration::from_secs(300))
            .await;
        assert!(recovered.is_empty());
    }

    #[tokio::test]
    async fn test_recover_emits_mission_failed_event() {
        let store = make_test_store();
        let mut rx = store.subscribe();

        let mut m = make_mission("m-evt", "Event test");
        m.status = MissionStatus::Executing;
        store.insert(m).await;

        {
            let db = store.db.lock().await;
            let old = (Utc::now() - chrono::Duration::minutes(10)).to_rfc3339();
            db.execute(
                "UPDATE missions SET updated_at = ?1 WHERE id = 'm-evt'",
                params![old],
            )
            .unwrap();
        }

        store
            .recover_stale_missions(std::time::Duration::from_secs(300))
            .await;

        let event = rx.try_recv().expect("should receive MissionFailed event");
        match event {
            PantheonEvent::MissionFailed { mission_id, reason } => {
                assert_eq!(mission_id, "m-evt");
                assert!(reason.contains("Gateway restarted"));
            }
            _ => panic!("Expected MissionFailed event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_check_mission_timeouts() {
        let store = make_test_store();

        let mut m = make_mission("m-timeout", "Timeout test");
        m.status = MissionStatus::Executing;
        m.constraints.timeout_seconds = Some(1); // 1 second timeout
        store.insert(m).await;

        // Force created_at to 10 seconds ago
        {
            let db = store.db.lock().await;
            let old = (Utc::now() - chrono::Duration::seconds(10)).to_rfc3339();
            db.execute(
                "UPDATE missions SET created_at = ?1 WHERE id = 'm-timeout'",
                params![old],
            )
            .unwrap();
        }

        let timed_out = store
            .check_mission_timeouts(std::time::Duration::from_secs(1800))
            .await;
        assert_eq!(timed_out, vec!["m-timeout"]);

        let loaded = store.get("m-timeout").await.unwrap();
        assert_eq!(loaded.status, MissionStatus::Failed);
        assert!(loaded.feed.iter().any(|f| f.activity == "timeout"));
    }

    #[tokio::test]
    async fn test_check_mission_timeouts_uses_default() {
        let store = make_test_store();

        let mut m = make_mission("m-def-to", "Default timeout test");
        m.status = MissionStatus::Executing;
        m.constraints.timeout_seconds = None; // no per-mission timeout
        store.insert(m).await;

        // Force created_at to 10 seconds ago
        {
            let db = store.db.lock().await;
            let old = (Utc::now() - chrono::Duration::seconds(10)).to_rfc3339();
            db.execute(
                "UPDATE missions SET created_at = ?1 WHERE id = 'm-def-to'",
                params![old],
            )
            .unwrap();
        }

        // Default timeout of 5 seconds — should trigger
        let timed_out = store
            .check_mission_timeouts(std::time::Duration::from_secs(5))
            .await;
        assert_eq!(timed_out, vec!["m-def-to"]);
    }

    #[tokio::test]
    async fn test_checkpoint_touches_updated_at() {
        use zeus_prometheus::MissionCheckpointer;

        let store = make_test_store();
        let mut m = make_mission("m-cp", "Checkpoint test");
        m.status = MissionStatus::Executing;
        store.insert(m).await;

        let before = store.get("m-cp").await.unwrap().updated_at;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Call the trait method (takes mission_id: &str)
        MissionCheckpointer::checkpoint(&store, "m-cp").await;

        let after = store.get("m-cp").await.unwrap().updated_at;
        assert!(after > before, "updated_at should advance after checkpoint");
    }

    // ─── Room tests ────────────────────────────────────────

    #[tokio::test]
    async fn test_room_create_and_get() {
        let store = make_test_store();
        let room = super::super::pantheon::Room {
            id: "r-test-1".to_string(),
            name: "General".to_string(),
            description: Some("General chat".to_string()),
            room_type: super::super::pantheon::RoomType::Public,
            mission_id: None,
            created_by: "zeus-112".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.insert_room(&room).await;

        let loaded = store.get_room("r-test-1").await.expect("should find room");
        assert_eq!(loaded.id, "r-test-1");
        assert_eq!(loaded.name, "General");
        assert_eq!(loaded.room_type, super::super::pantheon::RoomType::Public);
    }

    #[tokio::test]
    async fn test_room_list() {
        let store = make_test_store();

        // Insert public and private rooms
        let public = super::super::pantheon::Room {
            id: "r-pub".to_string(),
            name: "Public Room".to_string(),
            description: None,
            room_type: super::super::pantheon::RoomType::Public,
            mission_id: None,
            created_by: "zeus-112".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let private = super::super::pantheon::Room {
            id: "r-priv".to_string(),
            name: "Private Room".to_string(),
            description: None,
            room_type: super::super::pantheon::RoomType::Private,
            mission_id: None,
            created_by: "zeus-112".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.insert_room(&public).await;
        store.insert_room(&private).await;

        let rooms = store.list_rooms().await;
        assert_eq!(rooms.len(), 2);
        // Public rooms should come first
        assert_eq!(rooms[0].room_type, super::super::pantheon::RoomType::Public);
    }

    #[tokio::test]
    async fn test_room_join_and_leave() {
        let store = make_test_store();
        let room = super::super::pantheon::Room {
            id: "r-join".to_string(),
            name: "Join Test".to_string(),
            description: None,
            room_type: super::super::pantheon::RoomType::Public,
            mission_id: None,
            created_by: "zeus-112".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.insert_room(&room).await;

        let member = super::super::pantheon::RoomMember {
            agent_id: "zeus-100".to_string(),
            agent_name: "Zeus100".to_string(),
            role: "member".to_string(),
            joined_at: Utc::now(),
        };
        store.join_room("r-join", &member).await;

        let members = store.list_room_members("r-join").await;
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].agent_id, "zeus-100");

        // Leave
        store.leave_room("r-join", "zeus-100").await;
        let members = store.list_room_members("r-join").await;
        assert!(members.is_empty());
    }

    #[tokio::test]
    async fn test_room_join_duplicate_ignored() {
        let store = make_test_store();
        let room = super::super::pantheon::Room {
            id: "r-dup".to_string(),
            name: "Dup Test".to_string(),
            description: None,
            room_type: super::super::pantheon::RoomType::Public,
            mission_id: None,
            created_by: "zeus-112".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.insert_room(&room).await;

        let member = super::super::pantheon::RoomMember {
            agent_id: "zeus-100".to_string(),
            agent_name: "Zeus100".to_string(),
            role: "member".to_string(),
            joined_at: Utc::now(),
        };
        store.join_room("r-dup", &member).await;
        store.join_room("r-dup", &member).await; // duplicate — should be ignored

        let members = store.list_room_members("r-dup").await;
        assert_eq!(members.len(), 1);
    }

    #[tokio::test]
    async fn test_room_messages() {
        let store = make_test_store();
        let room = super::super::pantheon::Room {
            id: "r-msg".to_string(),
            name: "Messages Test".to_string(),
            description: None,
            room_type: super::super::pantheon::RoomType::Public,
            mission_id: None,
            created_by: "zeus-112".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.insert_room(&room).await;

        // Send 3 messages
        for i in 0..3 {
            let msg = super::super::pantheon::RoomMessage {
                id: format!("msg-{}", i),
                room_id: "r-msg".to_string(),
                sender_id: "zeus-112".to_string(),
                sender_name: "Zeus112".to_string(),
                content: format!("Message {}", i),
                message_type: "chat".to_string(),
                metadata: None,
                reply_to: None,
                edited: false,
                attachments: vec![],
                timestamp: Utc::now() + chrono::Duration::seconds(i as i64),
            };
            store.insert_room_message(&msg).await;
        }

        let messages = store.get_room_messages("r-msg", 50, None).await;
        assert_eq!(messages.len(), 3);
        // Should be in chronological order (oldest first)
        assert_eq!(messages[0].content, "Message 0");
        assert_eq!(messages[2].content, "Message 2");
    }

    #[tokio::test]
    async fn test_room_messages_pagination() {
        let store = make_test_store();
        let room = super::super::pantheon::Room {
            id: "r-page".to_string(),
            name: "Page Test".to_string(),
            description: None,
            room_type: super::super::pantheon::RoomType::Public,
            mission_id: None,
            created_by: "zeus-112".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.insert_room(&room).await;

        for i in 0..5 {
            let msg = super::super::pantheon::RoomMessage {
                id: format!("msg-p-{}", i),
                room_id: "r-page".to_string(),
                sender_id: "zeus-112".to_string(),
                sender_name: "Zeus112".to_string(),
                content: format!("Message {}", i),
                message_type: "chat".to_string(),
                metadata: None,
                reply_to: None,
                edited: false,
                attachments: vec![],
                timestamp: Utc::now() + chrono::Duration::seconds(i as i64),
            };
            store.insert_room_message(&msg).await;
        }

        // Limit to 2
        let messages = store.get_room_messages("r-page", 2, None).await;
        assert_eq!(messages.len(), 2);
    }

    #[tokio::test]
    async fn test_create_mission_room() {
        let store = make_test_store();
        let room_id = store
            .create_mission_room("m-test-123", "Build a trading bot")
            .await;

        assert_eq!(room_id, "m-test-123-room");
        let room = store
            .get_room(&room_id)
            .await
            .expect("mission room should exist");
        assert_eq!(room.room_type, super::super::pantheon::RoomType::Public);
        assert_eq!(room.mission_id, Some("m-test-123".to_string()));
        assert!(room.name.contains("Build a trading bot"));
    }

    #[tokio::test]
    async fn test_emit_and_subscribe() {
        let store = make_test_store();
        let mut rx = store.subscribe();

        store.emit(PantheonEvent::MissionCreated {
            mission_id: "m-evt".to_string(),
            goal: "Test event".to_string(),
            status: "created".to_string(),
        });

        let event = rx.try_recv().expect("should receive event");
        if let PantheonEvent::MissionCreated { mission_id, .. } = event {
            assert_eq!(mission_id, "m-evt");
        } else {
            panic!("wrong event type");
        }
    }

    // ── Pending Approval Tests ──

    #[tokio::test]
    async fn test_insert_and_get_pending_approval() {
        let store = make_test_store();
        store
            .insert_pending_approval(
                "spawn-abc",
                "r-warroom",
                "user-1",
                "Miguel",
                "scan Polymarket for trending markets",
                "complex",
                "High",
                r#"[{"description":"scan Polymarket","agent_type":"spawn","status":"pending"}]"#,
                "scan Polymarket for trending markets",
            )
            .await;

        let approval = store
            .get_pending_approval("spawn-abc")
            .await
            .expect("should exist");
        assert_eq!(approval.plan_id, "spawn-abc");
        assert_eq!(approval.room_id, "r-warroom");
        assert_eq!(approval.goal, "scan Polymarket for trending markets");
        assert_eq!(approval.complexity, "complex");
        assert_eq!(approval.risk, "High");
        assert_eq!(approval.status, "awaiting_approval");
        assert_eq!(approval.revision, 1);
        assert!(approval.resolved_by.is_none());
    }

    #[tokio::test]
    async fn test_approve_plan() {
        let store = make_test_store();
        store
            .insert_pending_approval(
                "spawn-def",
                "r-warroom",
                "user-1",
                "Miguel",
                "deploy app",
                "complex",
                "High",
                "[]",
                "deploy app",
            )
            .await;

        let result = store.approve_plan("spawn-def", "user-2", "Zeus112").await;
        assert!(result.is_some());
        let approval = result.unwrap();
        assert_eq!(approval.status, "approved");
        assert_eq!(approval.resolved_by, Some("user-2".to_string()));
        assert_eq!(approval.resolved_by_name, Some("Zeus112".to_string()));
    }

    #[tokio::test]
    async fn test_reject_plan_with_reason() {
        let store = make_test_store();
        store
            .insert_pending_approval(
                "spawn-ghi",
                "r-warroom",
                "user-1",
                "Miguel",
                "nuke production",
                "complex",
                "High",
                "[]",
                "nuke production",
            )
            .await;

        let result = store
            .reject_plan("spawn-ghi", "user-2", "Zeus112", Some("too risky"))
            .await;
        assert!(result.is_some());
        let approval = result.unwrap();
        assert_eq!(approval.status, "rejected");
        assert_eq!(approval.reject_reason, Some("too risky".to_string()));
    }

    #[tokio::test]
    async fn test_approve_nonexistent_plan() {
        let store = make_test_store();
        let result = store.approve_plan("nonexistent", "user-1", "Test").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_double_approve_fails() {
        let store = make_test_store();
        store
            .insert_pending_approval(
                "spawn-dbl",
                "r-warroom",
                "user-1",
                "Miguel",
                "task",
                "moderate",
                "Medium",
                "[]",
                "task",
            )
            .await;

        let first = store.approve_plan("spawn-dbl", "user-2", "Zeus112").await;
        assert!(first.is_some());

        // Second approve should fail (no longer awaiting_approval)
        let second = store.approve_plan("spawn-dbl", "user-3", "Zeus100").await;
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn test_list_pending_approvals() {
        let store = make_test_store();
        // Insert 3 approvals, approve 1
        for i in 1..=3 {
            store
                .insert_pending_approval(
                    &format!("spawn-{}", i),
                    "r-warroom",
                    "user-1",
                    "Miguel",
                    &format!("task {}", i),
                    "complex",
                    "High",
                    "[]",
                    &format!("task {}", i),
                )
                .await;
        }
        store.approve_plan("spawn-2", "user-2", "Zeus112").await;

        let pending = store.list_pending_approvals("r-warroom").await;
        assert_eq!(pending.len(), 2); // spawn-1 and spawn-3
        assert!(pending.iter().all(|p| p.status == "awaiting_approval"));
        assert!(pending.iter().all(|p| p.plan_id != "spawn-2"));
    }

    #[tokio::test]
    async fn test_reopen_plan_increments_revision() {
        let store = make_test_store();
        store
            .insert_pending_approval(
                "spawn-replan",
                "r-warroom",
                "user-1",
                "Miguel",
                "original goal",
                "complex",
                "High",
                "[{\"description\":\"step 1\"}]",
                "original goal",
            )
            .await;

        // Reject first
        store
            .reject_plan("spawn-replan", "user-2", "Reviewer", Some("too broad"))
            .await;
        let rejected = store.get_pending_approval("spawn-replan").await.unwrap();
        assert_eq!(rejected.status, "rejected");
        assert_eq!(rejected.revision, 1);

        // Reopen with new steps
        let reopened = store
            .reopen_plan_for_revision(
                "spawn-replan",
                "[{\"description\":\"revised step 1\"},{\"description\":\"revised step 2\"}]",
                "revised goal",
            )
            .await;
        assert!(reopened.is_some());
        let updated = reopened.unwrap();
        assert_eq!(updated.status, "awaiting_approval");
        assert_eq!(updated.revision, 2);
        assert_eq!(updated.goal, "revised goal");
        assert!(updated.resolved_by.is_none());
        assert!(updated.reject_reason.is_none());
    }

    #[tokio::test]
    async fn test_reopen_plan_max_revisions() {
        let store = make_test_store();
        store
            .insert_pending_approval(
                "spawn-maxrev",
                "r-warroom",
                "user-1",
                "Miguel",
                "goal",
                "complex",
                "High",
                "[]",
                "goal",
            )
            .await;

        // Cycle 1: reject → reopen (revision 1→2)
        store
            .reject_plan("spawn-maxrev", "u", "U", Some("r1"))
            .await;
        let r1 = store
            .reopen_plan_for_revision("spawn-maxrev", "[]", "goal v2")
            .await;
        assert!(r1.is_some());
        assert_eq!(r1.unwrap().revision, 2);

        // Cycle 2: reject → reopen (revision 2→3)
        store
            .reject_plan("spawn-maxrev", "u", "U", Some("r2"))
            .await;
        let r2 = store
            .reopen_plan_for_revision("spawn-maxrev", "[]", "goal v3")
            .await;
        assert!(r2.is_some());
        assert_eq!(r2.unwrap().revision, 3);

        // Cycle 3: reject → reopen should fail (revision 3 >= 3)
        store
            .reject_plan("spawn-maxrev", "u", "U", Some("r3"))
            .await;
        let r3 = store
            .reopen_plan_for_revision("spawn-maxrev", "[]", "goal v4")
            .await;
        assert!(r3.is_none(), "Should not reopen past revision 3");
    }

    #[tokio::test]
    async fn test_reopen_only_rejected_plans() {
        let store = make_test_store();
        store
            .insert_pending_approval(
                "spawn-notrej",
                "r-warroom",
                "user-1",
                "Miguel",
                "goal",
                "simple",
                "Low",
                "[]",
                "goal",
            )
            .await;

        // Try to reopen a plan that's still awaiting_approval (not rejected)
        let result = store
            .reopen_plan_for_revision("spawn-notrej", "[]", "new goal")
            .await;
        assert!(result.is_none(), "Should not reopen non-rejected plans");
    }
}
