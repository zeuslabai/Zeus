//! SQLite schema migration helper shared by all zeus-prometheus stores.
//!
//! Apply pending schema migrations tracked by `PRAGMA user_version`.
//! See `zeus-api/src/db.rs` for the full migration contract.

use zeus_core::{Error, Result};

/// Apply pending schema migrations tracked by `PRAGMA user_version`.
///
/// `migrations` is a slice of SQL batches. `user_version` holds the count of
/// already-applied migrations. Each migration is applied in order; on success
/// `user_version` is incremented. `duplicate column name` errors are silently
/// skipped (safe for databases created before this versioning was added).
pub(crate) fn run_migrations(conn: &rusqlite::Connection, migrations: &[&str]) -> Result<()> {
    let current: usize = conn
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .unwrap_or(0) as usize;

    for (i, sql) in migrations[current..].iter().enumerate() {
        let next_version = (current + i + 1) as i64;
        match conn.execute_batch(sql) {
            Ok(()) => tracing::debug!("sqlite migration v{} applied", next_version),
            Err(e) if e.to_string().contains("duplicate column name") => {
                tracing::debug!(
                    "sqlite migration v{} already applied (skipped)",
                    next_version
                );
            }
            Err(e) => {
                return Err(Error::Database(format!(
                    "Migration v{} failed: {}",
                    next_version, e
                )));
            }
        }
        conn.pragma_update(None, "user_version", next_version)
            .map_err(|e| {
                Error::Database(format!("Failed to set user_version to {next_version}: {e}"))
            })?;
    }
    Ok(())
}
