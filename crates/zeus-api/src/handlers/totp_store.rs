//! SQLite-backed persistence for TOTP 2FA (S23 Track H).
//!
//! Mirrors the `DeployStore` / `PantheonStore` pattern:
//! `Arc<Mutex<Connection>>` with WAL mode.
//!
//! Tables:
//!  - `totp_users`          — TOTP secrets (single-admin model)
//!  - `totp_recovery_codes` — hashed one-time recovery codes
//!  - `totp_sessions`       — JWT session tracking (hash → expiry)

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;

/// Versioned schema migrations for the TOTP SQLite database.
const TOTP_MIGRATIONS: &[&str] = &[
    // v1 — initial schema
    "CREATE TABLE IF NOT EXISTS totp_users (
        id TEXT PRIMARY KEY,
        secret_base32 TEXT NOT NULL,
        enabled INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL,
        enabled_at TEXT
    );
    CREATE TABLE IF NOT EXISTS totp_recovery_codes (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id TEXT NOT NULL REFERENCES totp_users(id),
        code_hash TEXT NOT NULL,
        used INTEGER NOT NULL DEFAULT 0,
        used_at TEXT
    );
    CREATE TABLE IF NOT EXISTS totp_sessions (
        token_hash TEXT PRIMARY KEY,
        user_id TEXT NOT NULL,
        created_at TEXT NOT NULL,
        expires_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_recovery_user ON totp_recovery_codes(user_id);
    CREATE INDEX IF NOT EXISTS idx_sessions_expires ON totp_sessions(expires_at);",
];

// ============================================================================
// Data types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotpUser {
    pub id: String,
    pub secret_base32: String,
    pub enabled: bool,
    pub created_at: String,
    pub enabled_at: Option<String>,
}

// ============================================================================
// TotpStore
// ============================================================================

#[derive(Clone)]
pub struct TotpStore {
    db: Arc<Mutex<Connection>>,
}

impl TotpStore {
    /// Open (or create) the TOTP SQLite database.
    pub fn new(db_path: &PathBuf) -> Result<Self, String> {
        let conn =
            Connection::open(db_path).map_err(|e| format!("Failed to open totp db: {e}"))?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=ON;",
        )
        .map_err(|e| format!("Failed to set pragmas: {e}"))?;

        crate::db::run_migrations(&conn, TOTP_MIGRATIONS)
            .map_err(|e| format!("TOTP schema migration failed: {e}"))?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory TOTP store (for tests / fallback).
    pub fn in_memory() -> Result<Self, String> {
        let path = PathBuf::from(":memory:");
        Self::new(&path)
    }

    // ── Users ────────────────────────────────────────────────────

    /// Store a new TOTP secret for a user (enabled=false until verified).
    pub async fn create_user(&self, id: &str, secret_base32: &str) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        match db.execute(
            "INSERT OR REPLACE INTO totp_users (id, secret_base32, enabled, created_at, enabled_at)
             VALUES (?1, ?2, 0, ?3, NULL)",
            params![id, secret_base32, now],
        ) {
            Ok(_) => true,
            Err(e) => {
                warn!("Failed to create TOTP user {id}: {e}");
                false
            }
        }
    }

    /// Fetch a TOTP user by ID.
    pub async fn get_user(&self, id: &str) -> Option<TotpUser> {
        let db = self.db.lock().await;
        db.query_row(
            "SELECT id, secret_base32, enabled, created_at, enabled_at
             FROM totp_users WHERE id = ?1",
            params![id],
            |row| {
                Ok(TotpUser {
                    id: row.get(0)?,
                    secret_base32: row.get(1)?,
                    enabled: row.get::<_, i32>(2)? != 0,
                    created_at: row.get(3)?,
                    enabled_at: row.get(4)?,
                })
            },
        )
        .ok()
    }

    /// Mark a user's TOTP as enabled (after first successful code verification).
    pub async fn enable_user(&self, id: &str) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        match db.execute(
            "UPDATE totp_users SET enabled = 1, enabled_at = ?1 WHERE id = ?2",
            params![now, id],
        ) {
            Ok(n) => n > 0,
            Err(e) => {
                warn!("Failed to enable TOTP for {id}: {e}");
                false
            }
        }
    }

    /// Remove a user's TOTP setup (disable 2FA). Also removes recovery codes.
    pub async fn disable_user(&self, id: &str) -> bool {
        let db = self.db.lock().await;
        let _ = db.execute(
            "DELETE FROM totp_recovery_codes WHERE user_id = ?1",
            params![id],
        );
        let _ = db.execute(
            "DELETE FROM totp_sessions WHERE user_id = ?1",
            params![id],
        );
        match db.execute("DELETE FROM totp_users WHERE id = ?1", params![id]) {
            Ok(n) => n > 0,
            Err(e) => {
                warn!("Failed to disable TOTP for {id}: {e}");
                false
            }
        }
    }

    // ── Recovery codes ───────────────────────────────────────────

    /// Store hashed recovery codes for a user (bulk insert).
    pub async fn store_recovery_codes(&self, user_id: &str, code_hashes: &[String]) -> bool {
        let db = self.db.lock().await;
        // Clear existing unused codes first
        let _ = db.execute(
            "DELETE FROM totp_recovery_codes WHERE user_id = ?1 AND used = 0",
            params![user_id],
        );
        for hash in code_hashes {
            if let Err(e) = db.execute(
                "INSERT INTO totp_recovery_codes (user_id, code_hash, used) VALUES (?1, ?2, 0)",
                params![user_id, hash],
            ) {
                warn!("Failed to store recovery code for {user_id}: {e}");
                return false;
            }
        }
        true
    }

    /// Attempt to use a recovery code. Returns true if the code was valid and unused.
    pub async fn use_recovery_code(&self, user_id: &str, code_hash: &str) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        match db.execute(
            "UPDATE totp_recovery_codes SET used = 1, used_at = ?1
             WHERE user_id = ?2 AND code_hash = ?3 AND used = 0",
            params![now, user_id, code_hash],
        ) {
            Ok(n) => n > 0,
            Err(e) => {
                warn!("Failed to use recovery code for {user_id}: {e}");
                false
            }
        }
    }

    /// Count remaining (unused) recovery codes for a user.
    pub async fn remaining_recovery_codes(&self, user_id: &str) -> usize {
        let db = self.db.lock().await;
        db.query_row(
            "SELECT COUNT(*) FROM totp_recovery_codes WHERE user_id = ?1 AND used = 0",
            params![user_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    }

    // ── Sessions ─────────────────────────────────────────────────

    /// Create a session entry (token hash → user, expiry).
    pub async fn create_session(
        &self,
        token_hash: &str,
        user_id: &str,
        expires_at: &str,
    ) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        match db.execute(
            "INSERT OR REPLACE INTO totp_sessions (token_hash, user_id, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![token_hash, user_id, now, expires_at],
        ) {
            Ok(_) => true,
            Err(e) => {
                warn!("Failed to create TOTP session: {e}");
                false
            }
        }
    }

    /// Validate a session: exists and not expired.
    pub async fn validate_session(&self, token_hash: &str) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        db.query_row(
            "SELECT 1 FROM totp_sessions WHERE token_hash = ?1 AND expires_at > ?2",
            params![token_hash, now],
            |_| Ok(()),
        )
        .is_ok()
    }

    /// Remove expired sessions.
    pub async fn cleanup_expired_sessions(&self) -> usize {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        db.execute(
            "DELETE FROM totp_sessions WHERE expires_at <= ?1",
            params![now],
        )
        .unwrap_or(0)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> TotpStore {
        TotpStore::in_memory().expect("in-memory store should work")
    }

    #[tokio::test]
    async fn test_create_and_get_user() {
        let s = store();
        assert!(s.create_user("admin", "JBSWY3DPEHPK3PXP").await);
        let user = s.get_user("admin").await.expect("user should exist");
        assert_eq!(user.id, "admin");
        assert_eq!(user.secret_base32, "JBSWY3DPEHPK3PXP");
        assert!(!user.enabled);
        assert!(user.enabled_at.is_none());
    }

    #[tokio::test]
    async fn test_enable_disable_user() {
        let s = store();
        assert!(s.create_user("admin", "SECRET123").await);

        // Enable
        assert!(s.enable_user("admin").await);
        let user = s.get_user("admin").await.unwrap();
        assert!(user.enabled);
        assert!(user.enabled_at.is_some());

        // Disable (removes user entirely)
        assert!(s.disable_user("admin").await);
        assert!(s.get_user("admin").await.is_none());
    }

    #[tokio::test]
    async fn test_recovery_codes() {
        let s = store();
        assert!(s.create_user("admin", "SECRET").await);

        let hashes: Vec<String> = (0..8).map(|i| format!("hash_{i}")).collect();
        assert!(s.store_recovery_codes("admin", &hashes).await);
        assert_eq!(s.remaining_recovery_codes("admin").await, 8);

        // Use one code
        assert!(s.use_recovery_code("admin", "hash_0").await);
        assert_eq!(s.remaining_recovery_codes("admin").await, 7);

        // Cannot reuse
        assert!(!s.use_recovery_code("admin", "hash_0").await);
        assert_eq!(s.remaining_recovery_codes("admin").await, 7);

        // Wrong hash fails
        assert!(!s.use_recovery_code("admin", "nonexistent").await);
    }

    #[tokio::test]
    async fn test_session_lifecycle() {
        let s = store();

        let future = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::hours(24))
            .unwrap()
            .to_rfc3339();
        assert!(s.create_session("tok_hash_1", "admin", &future).await);
        assert!(s.validate_session("tok_hash_1").await);

        // Expired session
        let past = "2020-01-01T00:00:00+00:00";
        assert!(s.create_session("tok_hash_expired", "admin", past).await);
        assert!(!s.validate_session("tok_hash_expired").await);

        // Cleanup removes expired
        let cleaned = s.cleanup_expired_sessions().await;
        assert!(cleaned >= 1);
        assert!(!s.validate_session("tok_hash_expired").await);
        // Valid session still works
        assert!(s.validate_session("tok_hash_1").await);
    }
}
