//! SQLite-backed persistence for One-Click Deploy (Phase 4).
//!
//! Mirrors the `PantheonStore` / `MarketplaceStore` pattern:
//! `Arc<Mutex<Connection>>` with WAL mode.
//!
//! Tables:
//!  - `deploy_targets`    — configured deployment providers (Vercel, Netlify, Docker, etc.)
//!  - `deployments`       — deployment history with status, URL, timestamps
//!  - `deployment_logs`   — per-deployment step log entries
//!  - `rollback_snapshots`— versioned snapshots for rollback

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;

/// Versioned schema migrations for the Deploy SQLite database.
const DEPLOY_MIGRATIONS: &[&str] = &[
    // v1 — initial schema
    "CREATE TABLE IF NOT EXISTS deploy_targets (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        provider TEXT NOT NULL,
        environment TEXT NOT NULL DEFAULT 'production',
        config TEXT NOT NULL DEFAULT '{}',
        credentials_ref TEXT NOT NULL DEFAULT '',
        project_path TEXT NOT NULL DEFAULT '',
        build_command TEXT NOT NULL DEFAULT '',
        output_dir TEXT NOT NULL DEFAULT '',
        url TEXT NOT NULL DEFAULT '',
        active INTEGER NOT NULL DEFAULT 1,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS deployments (
        id TEXT PRIMARY KEY,
        target_id TEXT NOT NULL,
        version TEXT NOT NULL DEFAULT '0.0.0',
        status TEXT NOT NULL DEFAULT 'pending',
        trigger TEXT NOT NULL DEFAULT 'manual',
        commit_hash TEXT NOT NULL DEFAULT '',
        commit_message TEXT NOT NULL DEFAULT '',
        build_log TEXT NOT NULL DEFAULT '',
        deploy_url TEXT NOT NULL DEFAULT '',
        preview_url TEXT NOT NULL DEFAULT '',
        duration_secs INTEGER NOT NULL DEFAULT 0,
        error_message TEXT NOT NULL DEFAULT '',
        initiated_by TEXT NOT NULL DEFAULT 'system',
        metadata TEXT NOT NULL DEFAULT '{}',
        created_at TEXT NOT NULL,
        started_at TEXT,
        completed_at TEXT
    );
    CREATE TABLE IF NOT EXISTS deployment_logs (
        id TEXT PRIMARY KEY,
        deployment_id TEXT NOT NULL,
        step TEXT NOT NULL,
        status TEXT NOT NULL DEFAULT 'running',
        message TEXT NOT NULL DEFAULT '',
        duration_ms INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS rollback_snapshots (
        id TEXT PRIMARY KEY,
        deployment_id TEXT NOT NULL,
        target_id TEXT NOT NULL,
        version TEXT NOT NULL,
        snapshot_ref TEXT NOT NULL DEFAULT '',
        deploy_url TEXT NOT NULL DEFAULT '',
        is_current INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_deployments_target ON deployments(target_id);
    CREATE INDEX IF NOT EXISTS idx_deployments_status ON deployments(status);
    CREATE INDEX IF NOT EXISTS idx_deployments_created ON deployments(created_at);
    CREATE INDEX IF NOT EXISTS idx_logs_deployment ON deployment_logs(deployment_id);
    CREATE INDEX IF NOT EXISTS idx_snapshots_target ON rollback_snapshots(target_id);
    CREATE INDEX IF NOT EXISTS idx_snapshots_deployment ON rollback_snapshots(deployment_id);
    CREATE INDEX IF NOT EXISTS idx_targets_provider ON deploy_targets(provider);
    CREATE INDEX IF NOT EXISTS idx_targets_active ON deploy_targets(active);",
];

// ============================================================================
// DeployStore
// ============================================================================

#[derive(Clone)]
pub struct DeployStore {
    db: Arc<Mutex<Connection>>,
}

impl DeployStore {
    /// Open (or create) the deploy SQLite database.
    pub fn new(db_path: &PathBuf) -> Result<Self, String> {
        let conn =
            Connection::open(db_path).map_err(|e| format!("Failed to open deploy db: {}", e))?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=ON;",
        )
        .map_err(|e| format!("Failed to set pragmas: {}", e))?;

        crate::db::run_migrations(&conn, DEPLOY_MIGRATIONS)
            .map_err(|e| format!("Deploy schema migration failed: {e}"))?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory deploy store (for fallback / tests).
    pub fn in_memory() -> Result<Self, String> {
        let path = PathBuf::from(":memory:");
        Self::new(&path)
    }

    // ── Deploy Targets ──────────────────────────────────────────

    /// Register a new deploy target.
    pub async fn create_target(&self, target: &DeployTargetRow) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        match db.execute(
            "INSERT OR REPLACE INTO deploy_targets (id, name, provider, environment, config, credentials_ref, project_path, build_command, output_dir, url, active, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                target.id, target.name, target.provider, target.environment,
                target.config_json, target.credentials_ref, target.project_path,
                target.build_command, target.output_dir, target.url,
                target.active as i32, now, now,
            ],
        ) {
            Ok(_) => true,
            Err(e) => { warn!("Failed to create target {}: {}", target.id, e); false }
        }
    }

    /// Get a deploy target by ID.
    pub async fn get_target(&self, id: &str) -> Option<DeployTargetRow> {
        let db = self.db.lock().await;
        db.query_row(
            "SELECT id, name, provider, environment, config, credentials_ref, project_path, build_command, output_dir, url, active, created_at, updated_at
             FROM deploy_targets WHERE id = ?1",
            params![id],
            |row| Ok(row_to_target(row)),
        ).ok()
    }

    /// List all active deploy targets.
    pub async fn list_targets(&self) -> Vec<DeployTargetRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, name, provider, environment, config, credentials_ref, project_path, build_command, output_dir, url, active, created_at, updated_at
             FROM deploy_targets WHERE active = 1 ORDER BY name ASC"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map([], |row| Ok(row_to_target(row))) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// Update a deploy target's config.
    pub async fn update_target(
        &self,
        id: &str,
        name: Option<&str>,
        config_json: Option<&str>,
        build_command: Option<&str>,
        output_dir: Option<&str>,
    ) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        // Build dynamic SET clause
        let mut sets = vec!["updated_at = ?1"];
        let mut param_idx = 2u32;
        let mut values: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(now)];

        if let Some(n) = name {
            sets.push("name = ?2");
            values.push(Box::new(n.to_string()));
            param_idx = 3;
        }
        if let Some(c) = config_json {
            sets.push(if param_idx == 2 {
                "config = ?2"
            } else {
                "config = ?3"
            });
            values.push(Box::new(c.to_string()));
            param_idx += 1;
        }
        if let Some(b) = build_command {
            let placeholder = format!("build_command = ?{}", param_idx);
            // We'll use a simpler approach below
            let _ = (b, placeholder);
        }
        if let Some(o) = output_dir {
            let _ = o;
        }

        // Simpler: just update all fields with current values if not provided
        drop(sets);
        drop(values);
        let _ = param_idx;

        let target = match db.query_row(
            "SELECT id, name, provider, environment, config, credentials_ref, project_path, build_command, output_dir, url, active, created_at, updated_at
             FROM deploy_targets WHERE id = ?1",
            params![id],
            |row| Ok(row_to_target(row)),
        ) {
            Ok(t) => t,
            Err(_) => return false,
        };

        let new_name = name.unwrap_or(&target.name);
        let new_config = config_json.unwrap_or(&target.config_json);
        let new_build = build_command.unwrap_or(&target.build_command);
        let new_output = output_dir.unwrap_or(&target.output_dir);

        match db.execute(
            "UPDATE deploy_targets SET name = ?1, config = ?2, build_command = ?3, output_dir = ?4, updated_at = ?5 WHERE id = ?6",
            params![new_name, new_config, new_build, new_output, Utc::now().to_rfc3339(), id],
        ) {
            Ok(n) => n > 0,
            Err(e) => { warn!("Failed to update target {}: {}", id, e); false }
        }
    }

    /// Deactivate (soft-delete) a deploy target.
    pub async fn deactivate_target(&self, id: &str) -> bool {
        let db = self.db.lock().await;
        match db.execute(
            "UPDATE deploy_targets SET active = 0, updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id],
        ) {
            Ok(n) => n > 0,
            Err(_) => false,
        }
    }

    // ── Deployments ─────────────────────────────────────────────

    /// Create a new deployment record.
    pub async fn create_deployment(&self, deployment: &DeploymentRow) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        match db.execute(
            "INSERT INTO deployments (id, target_id, version, status, trigger, commit_hash, commit_message, build_log, deploy_url, preview_url, duration_secs, error_message, initiated_by, metadata, created_at, started_at, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                deployment.id, deployment.target_id, deployment.version,
                deployment.status, deployment.trigger, deployment.commit_hash,
                deployment.commit_message, deployment.build_log, deployment.deploy_url,
                deployment.preview_url, deployment.duration_secs, deployment.error_message,
                deployment.initiated_by, deployment.metadata_json, now,
                deployment.started_at, deployment.completed_at,
            ],
        ) {
            Ok(_) => true,
            Err(e) => { warn!("Failed to create deployment {}: {}", deployment.id, e); false }
        }
    }

    /// Get a deployment by ID.
    pub async fn get_deployment(&self, id: &str) -> Option<DeploymentRow> {
        let db = self.db.lock().await;
        db.query_row(
            "SELECT id, target_id, version, status, trigger, commit_hash, commit_message, build_log, deploy_url, preview_url, duration_secs, error_message, initiated_by, metadata, created_at, started_at, completed_at
             FROM deployments WHERE id = ?1",
            params![id],
            |row| Ok(row_to_deployment(row)),
        ).ok()
    }

    /// Update deployment status (and optionally set timestamps, URLs, errors).
    pub async fn update_deployment_status(
        &self,
        id: &str,
        status: &str,
        deploy_url: Option<&str>,
        preview_url: Option<&str>,
        error_message: Option<&str>,
        duration_secs: Option<u64>,
    ) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();

        // Determine timestamps based on status transition
        let (started, completed) = match status {
            "building" | "deploying" => (Some(now.clone()), None),
            "live" | "failed" | "cancelled" => (None, Some(now)),
            _ => (None, None),
        };

        let url = deploy_url.unwrap_or("");
        let preview = preview_url.unwrap_or("");
        let error = error_message.unwrap_or("");
        let dur = duration_secs.unwrap_or(0) as i64;

        // Use COALESCE to not overwrite existing values with empty strings
        match db.execute(
            "UPDATE deployments SET
                status = ?1,
                deploy_url = CASE WHEN ?2 = '' THEN deploy_url ELSE ?2 END,
                preview_url = CASE WHEN ?3 = '' THEN preview_url ELSE ?3 END,
                error_message = CASE WHEN ?4 = '' THEN error_message ELSE ?4 END,
                duration_secs = CASE WHEN ?5 = 0 THEN duration_secs ELSE ?5 END,
                started_at = COALESCE(?6, started_at),
                completed_at = COALESCE(?7, completed_at)
             WHERE id = ?8",
            params![status, url, preview, error, dur, started, completed, id],
        ) {
            Ok(n) => n > 0,
            Err(e) => {
                warn!("Failed to update deployment {}: {}", id, e);
                false
            }
        }
    }

    /// Append to build log.
    pub async fn append_build_log(&self, id: &str, line: &str) {
        let db = self.db.lock().await;
        let _ = db.execute(
            "UPDATE deployments SET build_log = build_log || ?1 || char(10) WHERE id = ?2",
            params![line, id],
        );
    }

    /// List deployments for a target (most recent first).
    pub async fn list_deployments(&self, target_id: &str, limit: u32) -> Vec<DeploymentRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, target_id, version, status, trigger, commit_hash, commit_message, build_log, deploy_url, preview_url, duration_secs, error_message, initiated_by, metadata, created_at, started_at, completed_at
             FROM deployments WHERE target_id = ?1 ORDER BY created_at DESC LIMIT ?2"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map(params![target_id, limit], |row| Ok(row_to_deployment(row))) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// List recent deployments across all targets.
    pub async fn list_recent_deployments(&self, limit: u32) -> Vec<DeploymentRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, target_id, version, status, trigger, commit_hash, commit_message, build_log, deploy_url, preview_url, duration_secs, error_message, initiated_by, metadata, created_at, started_at, completed_at
             FROM deployments ORDER BY created_at DESC LIMIT ?1"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map(params![limit], |row| Ok(row_to_deployment(row))) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// Get the latest successful deployment for a target.
    pub async fn latest_live_deployment(&self, target_id: &str) -> Option<DeploymentRow> {
        let db = self.db.lock().await;
        db.query_row(
            "SELECT id, target_id, version, status, trigger, commit_hash, commit_message, build_log, deploy_url, preview_url, duration_secs, error_message, initiated_by, metadata, created_at, started_at, completed_at
             FROM deployments WHERE target_id = ?1 AND status = 'live' ORDER BY created_at DESC LIMIT 1",
            params![target_id],
            |row| Ok(row_to_deployment(row)),
        ).ok()
    }

    // ── Deployment Logs ─────────────────────────────────────────

    /// Add a step log entry to a deployment.
    pub async fn add_log_entry(
        &self,
        deployment_id: &str,
        step: &str,
        status: &str,
        message: &str,
        duration_ms: u64,
    ) {
        let db = self.db.lock().await;
        let _ = db.execute(
            "INSERT INTO deployment_logs (id, deployment_id, step, status, message, duration_ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                uuid::Uuid::new_v4().to_string(),
                deployment_id, step, status, message,
                duration_ms as i64,
                Utc::now().to_rfc3339(),
            ],
        );
    }

    /// Get log entries for a deployment (chronological).
    pub async fn get_logs(&self, deployment_id: &str) -> Vec<DeployLogRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, deployment_id, step, status, message, duration_ms, created_at
             FROM deployment_logs WHERE deployment_id = ?1 ORDER BY created_at ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map(params![deployment_id], |row| {
            Ok(DeployLogRow {
                id: row.get(0)?,
                deployment_id: row.get(1)?,
                step: row.get(2)?,
                status: row.get(3)?,
                message: row.get(4)?,
                duration_ms: row.get(5)?,
                created_at: row.get(6)?,
            })
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    // ── Rollback Snapshots ──────────────────────────────────────

    /// Save a rollback snapshot for a deployment.
    pub async fn save_snapshot(&self, snapshot: &RollbackSnapshotRow) -> bool {
        let db = self.db.lock().await;
        // Clear existing current flag for this target
        let _ = db.execute(
            "UPDATE rollback_snapshots SET is_current = 0 WHERE target_id = ?1",
            params![snapshot.target_id],
        );
        match db.execute(
            "INSERT INTO rollback_snapshots (id, deployment_id, target_id, version, snapshot_ref, deploy_url, is_current, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                snapshot.id, snapshot.deployment_id, snapshot.target_id,
                snapshot.version, snapshot.snapshot_ref, snapshot.deploy_url,
                snapshot.is_current as i32, Utc::now().to_rfc3339(),
            ],
        ) {
            Ok(_) => true,
            Err(e) => { warn!("Failed to save snapshot {}: {}", snapshot.id, e); false }
        }
    }

    /// Get the current (latest live) snapshot for a target.
    pub async fn current_snapshot(&self, target_id: &str) -> Option<RollbackSnapshotRow> {
        let db = self.db.lock().await;
        db.query_row(
            "SELECT id, deployment_id, target_id, version, snapshot_ref, deploy_url, is_current, created_at
             FROM rollback_snapshots WHERE target_id = ?1 AND is_current = 1",
            params![target_id],
            |row| Ok(row_to_snapshot(row)),
        ).ok()
    }

    /// List all snapshots for a target (most recent first).
    pub async fn list_snapshots(&self, target_id: &str, limit: u32) -> Vec<RollbackSnapshotRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, deployment_id, target_id, version, snapshot_ref, deploy_url, is_current, created_at
             FROM rollback_snapshots WHERE target_id = ?1 ORDER BY created_at DESC LIMIT ?2"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map(params![target_id, limit], |row| Ok(row_to_snapshot(row))) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// Rollback to a specific snapshot: mark it as current, return its deployment URL.
    pub async fn rollback_to(&self, snapshot_id: &str) -> Option<String> {
        let db = self.db.lock().await;
        // Get the snapshot
        let snapshot = match db.query_row(
            "SELECT id, deployment_id, target_id, version, snapshot_ref, deploy_url, is_current, created_at
             FROM rollback_snapshots WHERE id = ?1",
            params![snapshot_id],
            |row| Ok(row_to_snapshot(row)),
        ) {
            Ok(s) => s,
            Err(_) => return None,
        };

        // Clear current flag for this target, then set this one
        let _ = db.execute(
            "UPDATE rollback_snapshots SET is_current = 0 WHERE target_id = ?1",
            params![snapshot.target_id],
        );
        let _ = db.execute(
            "UPDATE rollback_snapshots SET is_current = 1 WHERE id = ?1",
            params![snapshot_id],
        );

        Some(snapshot.deploy_url)
    }

    // ── Stats ───────────────────────────────────────────────────

    /// Get overall deploy stats.
    pub async fn stats(&self) -> DeployStats {
        let db = self.db.lock().await;
        let total_targets: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM deploy_targets WHERE active = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let total_deployments: u64 = db
            .query_row("SELECT COUNT(*) FROM deployments", [], |row| row.get(0))
            .unwrap_or(0);
        let live_deployments: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM deployments WHERE status = 'live'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let failed_deployments: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM deployments WHERE status = 'failed'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let in_progress: u64 = db.query_row(
            "SELECT COUNT(*) FROM deployments WHERE status IN ('pending', 'building', 'deploying')", [], |row| row.get(0)
        ).unwrap_or(0);
        let avg_duration: f64 = db
            .query_row(
                "SELECT COALESCE(AVG(duration_secs), 0) FROM deployments WHERE status = 'live'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0.0);
        let total_snapshots: u64 = db
            .query_row("SELECT COUNT(*) FROM rollback_snapshots", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);

        DeployStats {
            total_targets,
            total_deployments,
            live_deployments,
            failed_deployments,
            in_progress,
            avg_duration_secs: avg_duration,
            total_snapshots,
        }
    }
}

// ============================================================================
// Row types
// ============================================================================

fn row_to_target(row: &rusqlite::Row) -> DeployTargetRow {
    DeployTargetRow {
        id: row.get(0).unwrap_or_default(),
        name: row.get(1).unwrap_or_default(),
        provider: row.get(2).unwrap_or_default(),
        environment: row.get(3).unwrap_or_default(),
        config_json: row.get(4).unwrap_or_default(),
        credentials_ref: row.get(5).unwrap_or_default(),
        project_path: row.get(6).unwrap_or_default(),
        build_command: row.get(7).unwrap_or_default(),
        output_dir: row.get(8).unwrap_or_default(),
        url: row.get(9).unwrap_or_default(),
        active: row.get::<_, i32>(10).unwrap_or(1) != 0,
        created_at: row.get(11).unwrap_or_default(),
        updated_at: row.get(12).unwrap_or_default(),
    }
}

fn row_to_deployment(row: &rusqlite::Row) -> DeploymentRow {
    DeploymentRow {
        id: row.get(0).unwrap_or_default(),
        target_id: row.get(1).unwrap_or_default(),
        version: row.get(2).unwrap_or_default(),
        status: row.get(3).unwrap_or_default(),
        trigger: row.get(4).unwrap_or_default(),
        commit_hash: row.get(5).unwrap_or_default(),
        commit_message: row.get(6).unwrap_or_default(),
        build_log: row.get(7).unwrap_or_default(),
        deploy_url: row.get(8).unwrap_or_default(),
        preview_url: row.get(9).unwrap_or_default(),
        duration_secs: row.get(10).unwrap_or(0),
        error_message: row.get(11).unwrap_or_default(),
        initiated_by: row.get(12).unwrap_or_default(),
        metadata_json: row.get(13).unwrap_or_default(),
        created_at: row.get(14).unwrap_or_default(),
        started_at: row.get(15).ok(),
        completed_at: row.get(16).ok(),
    }
}

fn row_to_snapshot(row: &rusqlite::Row) -> RollbackSnapshotRow {
    RollbackSnapshotRow {
        id: row.get(0).unwrap_or_default(),
        deployment_id: row.get(1).unwrap_or_default(),
        target_id: row.get(2).unwrap_or_default(),
        version: row.get(3).unwrap_or_default(),
        snapshot_ref: row.get(4).unwrap_or_default(),
        deploy_url: row.get(5).unwrap_or_default(),
        is_current: row.get::<_, i32>(6).unwrap_or(0) != 0,
        created_at: row.get(7).unwrap_or_default(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployTargetRow {
    pub id: String,
    pub name: String,
    /// Provider type: "vercel", "netlify", "docker", "ssh", "freebsd", "s3", "custom"
    pub provider: String,
    /// Environment: "production", "staging", "preview"
    pub environment: String,
    /// JSON config blob (provider-specific settings)
    pub config_json: String,
    /// Reference to credential vault key (never stores secrets directly)
    pub credentials_ref: String,
    /// Path to the project source (local or git URL)
    pub project_path: String,
    /// Build command (e.g. "trunk build --release", "cargo build --release")
    pub build_command: String,
    /// Output directory for built artifacts
    pub output_dir: String,
    /// Production URL for this target
    pub url: String,
    pub active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentRow {
    pub id: String,
    pub target_id: String,
    pub version: String,
    /// Status: "pending", "building", "deploying", "live", "failed", "cancelled", "rolled_back"
    pub status: String,
    /// Trigger: "manual", "push", "webhook", "cron", "rollback"
    pub trigger: String,
    pub commit_hash: String,
    pub commit_message: String,
    pub build_log: String,
    pub deploy_url: String,
    pub preview_url: String,
    pub duration_secs: u64,
    pub error_message: String,
    pub initiated_by: String,
    pub metadata_json: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployLogRow {
    pub id: String,
    pub deployment_id: String,
    /// Step name: "checkout", "install", "build", "deploy", "verify", "screenshot"
    pub step: String,
    /// Status: "running", "completed", "failed", "skipped"
    pub status: String,
    pub message: String,
    pub duration_ms: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackSnapshotRow {
    pub id: String,
    pub deployment_id: String,
    pub target_id: String,
    pub version: String,
    /// Reference to stored snapshot (could be git tag, S3 key, etc.)
    pub snapshot_ref: String,
    pub deploy_url: String,
    pub is_current: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployStats {
    pub total_targets: u64,
    pub total_deployments: u64,
    pub live_deployments: u64,
    pub failed_deployments: u64,
    pub in_progress: u64,
    pub avg_duration_secs: f64,
    pub total_snapshots: u64,
}

/// Frontend-friendly deployment response with parsed JSON fields.
#[derive(Debug, Clone, Serialize)]
pub struct DeploymentResponse {
    pub id: String,
    pub target_id: String,
    pub target_name: String,
    pub provider: String,
    pub version: String,
    pub status: String,
    pub trigger: String,
    pub commit_hash: String,
    pub commit_message: String,
    pub deploy_url: String,
    pub preview_url: String,
    pub duration_secs: u64,
    pub error_message: String,
    pub initiated_by: String,
    pub metadata: serde_json::Value,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

impl DeploymentResponse {
    /// Create from deployment row + target info.
    pub fn from_row(row: DeploymentRow, target_name: &str, provider: &str) -> Self {
        let metadata: serde_json::Value =
            serde_json::from_str(&row.metadata_json).unwrap_or(serde_json::json!({}));
        Self {
            id: row.id,
            target_id: row.target_id,
            target_name: target_name.to_string(),
            provider: provider.to_string(),
            version: row.version,
            status: row.status,
            trigger: row.trigger,
            commit_hash: row.commit_hash,
            commit_message: row.commit_message,
            deploy_url: row.deploy_url,
            preview_url: row.preview_url,
            duration_secs: row.duration_secs,
            error_message: row.error_message,
            initiated_by: row.initiated_by,
            metadata,
            created_at: row.created_at,
            started_at: row.started_at,
            completed_at: row.completed_at,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_test_store() -> DeployStore {
        DeployStore::new(&PathBuf::from(":memory:")).unwrap()
    }

    fn test_target(id: &str, name: &str, provider: &str) -> DeployTargetRow {
        DeployTargetRow {
            id: id.to_string(),
            name: name.to_string(),
            provider: provider.to_string(),
            environment: "production".to_string(),
            config_json: "{}".to_string(),
            credentials_ref: "vault:vercel-token".to_string(),
            project_path: "/tmp/sample-project".to_string(),
            build_command: "cargo build --release".to_string(),
            output_dir: "target/release".to_string(),
            url: "https://zeuslab.ai".to_string(),
            active: true,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    fn test_deployment(id: &str, target_id: &str, version: &str) -> DeploymentRow {
        DeploymentRow {
            id: id.to_string(),
            target_id: target_id.to_string(),
            version: version.to_string(),
            status: "pending".to_string(),
            trigger: "manual".to_string(),
            commit_hash: "abc123".to_string(),
            commit_message: "feat: add deploy".to_string(),
            build_log: String::new(),
            deploy_url: String::new(),
            preview_url: String::new(),
            duration_secs: 0,
            error_message: String::new(),
            initiated_by: "zeus-112".to_string(),
            metadata_json: "{}".to_string(),
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            completed_at: None,
        }
    }

    #[tokio::test]
    async fn test_create_and_get_target() {
        let store = make_test_store();
        let target = test_target("t1", "Zeus Web", "vercel");
        assert!(store.create_target(&target).await);

        let fetched = store.get_target("t1").await;
        assert!(fetched.is_some());
        let f = fetched.unwrap();
        assert_eq!(f.name, "Zeus Web");
        assert_eq!(f.provider, "vercel");
        assert!(f.active);
    }

    #[tokio::test]
    async fn test_list_targets() {
        let store = make_test_store();
        store
            .create_target(&test_target("t1", "Web", "vercel"))
            .await;
        store
            .create_target(&test_target("t2", "API", "docker"))
            .await;
        store
            .create_target(&test_target("t3", "Docs", "netlify"))
            .await;

        let targets = store.list_targets().await;
        assert_eq!(targets.len(), 3);

        store.deactivate_target("t2").await;
        let targets = store.list_targets().await;
        assert_eq!(targets.len(), 2);
    }

    #[tokio::test]
    async fn test_update_target() {
        let store = make_test_store();
        store
            .create_target(&test_target("t1", "Web", "vercel"))
            .await;

        assert!(
            store
                .update_target("t1", Some("Zeus Web v2"), None, None, None)
                .await
        );
        let t = store.get_target("t1").await.unwrap();
        assert_eq!(t.name, "Zeus Web v2");
    }

    #[tokio::test]
    async fn test_create_and_get_deployment() {
        let store = make_test_store();
        store
            .create_target(&test_target("t1", "Web", "vercel"))
            .await;
        let dep = test_deployment("d1", "t1", "1.0.0");
        assert!(store.create_deployment(&dep).await);

        let fetched = store.get_deployment("d1").await;
        assert!(fetched.is_some());
        let f = fetched.unwrap();
        assert_eq!(f.version, "1.0.0");
        assert_eq!(f.status, "pending");
    }

    #[tokio::test]
    async fn test_deployment_status_lifecycle() {
        let store = make_test_store();
        store
            .create_target(&test_target("t1", "Web", "vercel"))
            .await;
        store
            .create_deployment(&test_deployment("d1", "t1", "1.0.0"))
            .await;

        // Building
        assert!(
            store
                .update_deployment_status("d1", "building", None, None, None, None)
                .await
        );
        let d = store.get_deployment("d1").await.unwrap();
        assert_eq!(d.status, "building");
        assert!(d.started_at.is_some());

        // Deploying
        assert!(
            store
                .update_deployment_status(
                    "d1",
                    "deploying",
                    None,
                    Some("https://preview.vercel.app"),
                    None,
                    None
                )
                .await
        );
        let d = store.get_deployment("d1").await.unwrap();
        assert_eq!(d.status, "deploying");
        assert_eq!(d.preview_url, "https://preview.vercel.app");

        // Live
        assert!(
            store
                .update_deployment_status(
                    "d1",
                    "live",
                    Some("https://zeuslab.ai"),
                    None,
                    None,
                    Some(45)
                )
                .await
        );
        let d = store.get_deployment("d1").await.unwrap();
        assert_eq!(d.status, "live");
        assert_eq!(d.deploy_url, "https://zeuslab.ai");
        assert_eq!(d.duration_secs, 45);
        assert!(d.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_deployment_failure() {
        let store = make_test_store();
        store
            .create_target(&test_target("t1", "Web", "vercel"))
            .await;
        store
            .create_deployment(&test_deployment("d1", "t1", "1.0.0"))
            .await;

        store
            .update_deployment_status("d1", "building", None, None, None, None)
            .await;
        store
            .update_deployment_status(
                "d1",
                "failed",
                None,
                None,
                Some("Build error: missing dependency"),
                Some(12),
            )
            .await;

        let d = store.get_deployment("d1").await.unwrap();
        assert_eq!(d.status, "failed");
        assert_eq!(d.error_message, "Build error: missing dependency");
        assert!(d.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_list_deployments() {
        let store = make_test_store();
        store
            .create_target(&test_target("t1", "Web", "vercel"))
            .await;
        store
            .create_deployment(&test_deployment("d1", "t1", "1.0.0"))
            .await;
        store
            .create_deployment(&test_deployment("d2", "t1", "1.1.0"))
            .await;
        store
            .create_deployment(&test_deployment("d3", "t1", "1.2.0"))
            .await;

        let deps = store.list_deployments("t1", 10).await;
        assert_eq!(deps.len(), 3);

        let deps = store.list_deployments("t1", 2).await;
        assert_eq!(deps.len(), 2);
    }

    #[tokio::test]
    async fn test_latest_live_deployment() {
        let store = make_test_store();
        store
            .create_target(&test_target("t1", "Web", "vercel"))
            .await;
        store
            .create_deployment(&test_deployment("d1", "t1", "1.0.0"))
            .await;
        store
            .create_deployment(&test_deployment("d2", "t1", "1.1.0"))
            .await;

        store
            .update_deployment_status(
                "d1",
                "live",
                Some("https://v1.zeuslab.ai"),
                None,
                None,
                None,
            )
            .await;
        store
            .update_deployment_status(
                "d2",
                "live",
                Some("https://v2.zeuslab.ai"),
                None,
                None,
                None,
            )
            .await;

        let latest = store.latest_live_deployment("t1").await;
        assert!(latest.is_some());
        let l = latest.unwrap();
        assert_eq!(l.deploy_url, "https://v2.zeuslab.ai");
    }

    #[tokio::test]
    async fn test_deployment_logs() {
        let store = make_test_store();
        store
            .create_target(&test_target("t1", "Web", "vercel"))
            .await;
        store
            .create_deployment(&test_deployment("d1", "t1", "1.0.0"))
            .await;

        store
            .add_log_entry("d1", "checkout", "completed", "Cloned repo", 250)
            .await;
        store
            .add_log_entry("d1", "build", "completed", "Built successfully", 15000)
            .await;
        store
            .add_log_entry("d1", "deploy", "completed", "Deployed to Vercel", 8000)
            .await;

        let logs = store.get_logs("d1").await;
        assert_eq!(logs.len(), 3);
        assert_eq!(logs[0].step, "checkout");
        assert_eq!(logs[2].step, "deploy");
    }

    #[tokio::test]
    async fn test_snapshots_and_rollback() {
        let store = make_test_store();
        store
            .create_target(&test_target("t1", "Web", "vercel"))
            .await;

        // Deploy v1
        store
            .create_deployment(&test_deployment("d1", "t1", "1.0.0"))
            .await;
        store
            .update_deployment_status(
                "d1",
                "live",
                Some("https://v1.zeuslab.ai"),
                None,
                None,
                None,
            )
            .await;
        store
            .save_snapshot(&RollbackSnapshotRow {
                id: "snap-1".to_string(),
                deployment_id: "d1".to_string(),
                target_id: "t1".to_string(),
                version: "1.0.0".to_string(),
                snapshot_ref: "git:v1.0.0".to_string(),
                deploy_url: "https://v1.zeuslab.ai".to_string(),
                is_current: true,
                created_at: String::new(),
            })
            .await;

        // Deploy v2
        store
            .create_deployment(&test_deployment("d2", "t1", "2.0.0"))
            .await;
        store
            .update_deployment_status(
                "d2",
                "live",
                Some("https://v2.zeuslab.ai"),
                None,
                None,
                None,
            )
            .await;
        store
            .save_snapshot(&RollbackSnapshotRow {
                id: "snap-2".to_string(),
                deployment_id: "d2".to_string(),
                target_id: "t1".to_string(),
                version: "2.0.0".to_string(),
                snapshot_ref: "git:v2.0.0".to_string(),
                deploy_url: "https://v2.zeuslab.ai".to_string(),
                is_current: true,
                created_at: String::new(),
            })
            .await;

        // Current should be v2
        let current = store.current_snapshot("t1").await.unwrap();
        assert_eq!(current.version, "2.0.0");

        // Rollback to v1
        let url = store.rollback_to("snap-1").await;
        assert_eq!(url, Some("https://v1.zeuslab.ai".to_string()));

        // Current should now be v1
        let current = store.current_snapshot("t1").await.unwrap();
        assert_eq!(current.version, "1.0.0");

        // List snapshots
        let snaps = store.list_snapshots("t1", 10).await;
        assert_eq!(snaps.len(), 2);
    }

    #[tokio::test]
    async fn test_build_log_append() {
        let store = make_test_store();
        store
            .create_target(&test_target("t1", "Web", "vercel"))
            .await;
        store
            .create_deployment(&test_deployment("d1", "t1", "1.0.0"))
            .await;

        store
            .append_build_log("d1", "[00:01] Compiling zeus v0.1.0")
            .await;
        store
            .append_build_log("d1", "[00:02] Finished release target")
            .await;

        let d = store.get_deployment("d1").await.unwrap();
        assert!(d.build_log.contains("Compiling zeus"));
        assert!(d.build_log.contains("Finished release"));
    }

    #[tokio::test]
    async fn test_stats() {
        let store = make_test_store();
        store
            .create_target(&test_target("t1", "Web", "vercel"))
            .await;
        store
            .create_target(&test_target("t2", "API", "docker"))
            .await;

        store
            .create_deployment(&test_deployment("d1", "t1", "1.0.0"))
            .await;
        store
            .update_deployment_status("d1", "live", None, None, None, Some(30))
            .await;

        store
            .create_deployment(&test_deployment("d2", "t1", "1.1.0"))
            .await;
        store
            .update_deployment_status("d2", "failed", None, None, Some("error"), None)
            .await;

        store
            .create_deployment(&test_deployment("d3", "t2", "1.0.0"))
            .await;

        let stats = store.stats().await;
        assert_eq!(stats.total_targets, 2);
        assert_eq!(stats.total_deployments, 3);
        assert_eq!(stats.live_deployments, 1);
        assert_eq!(stats.failed_deployments, 1);
        assert_eq!(stats.in_progress, 1); // d3 is still pending
    }

    #[tokio::test]
    async fn test_recent_deployments_cross_target() {
        let store = make_test_store();
        store
            .create_target(&test_target("t1", "Web", "vercel"))
            .await;
        store
            .create_target(&test_target("t2", "API", "docker"))
            .await;
        store
            .create_deployment(&test_deployment("d1", "t1", "1.0.0"))
            .await;
        store
            .create_deployment(&test_deployment("d2", "t2", "1.0.0"))
            .await;

        let recent = store.list_recent_deployments(10).await;
        assert_eq!(recent.len(), 2);
    }
}
