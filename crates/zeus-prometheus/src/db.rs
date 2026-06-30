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

    // Guard against version skew: if the DB's user_version is at or beyond the
    // binary's known migration count, there is nothing to apply. This happens
    // when a store's DB was migrated by a newer binary, then opened by an older
    // one whose migration list is shorter (e.g. after a downgrade or a build
    // with a trimmed migration array). `migrations[current..]` would otherwise
    // panic with a slice-out-of-bounds. A DB ahead of the binary is a no-op,
    // not a crash. See #205.
    if current >= migrations.len() {
        return Ok(());
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// #205: a DB whose `user_version` is ahead of the binary's migration list
    /// must no-op cleanly, not panic on `migrations[current..]` slice OOB.
    #[test]
    fn run_migrations_no_op_when_db_ahead_of_binary() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // Simulate a DB migrated by a newer binary to v4...
        conn.pragma_update(None, "user_version", 4_i64).unwrap();

        // ...then opened by an older binary whose migration list is length 1.
        let migrations: &[&str] = &["CREATE TABLE t (id INTEGER);"];

        // Must not panic and must apply nothing.
        let result = run_migrations(&conn, migrations);
        assert!(result.is_ok(), "expected Ok no-op, got {:?}", result);

        // user_version is left untouched — DB stays at v4.
        let after: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(after, 4, "user_version must be unchanged");

        // The v1 migration must NOT have run (table absent).
        let table_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='t'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 0, "no migration should have been applied");
    }

    /// Exact-boundary case: user_version == migrations.len() is also a no-op.
    #[test]
    fn run_migrations_no_op_at_exact_boundary() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 1_i64).unwrap();
        let migrations: &[&str] = &["CREATE TABLE t (id INTEGER);"];
        assert!(run_migrations(&conn, migrations).is_ok());
    }

    /// Sanity: a fresh DB (v0) still applies all migrations normally.
    #[test]
    fn run_migrations_applies_pending_on_fresh_db() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let migrations: &[&str] = &["CREATE TABLE t (id INTEGER);"];
        assert!(run_migrations(&conn, migrations).is_ok());

        let after: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(after, 1, "fresh DB should advance to v1");
    }
}
