//! SQLite schema migration helper shared by all zeus-api stores.

use rusqlite::Connection;

/// Apply pending schema migrations tracked by `PRAGMA user_version`.
///
/// `migrations` is a slice of SQL batches (each may contain multiple statements
/// separated by semicolons). `PRAGMA user_version` holds the number of
/// migrations already applied. On each call the function:
///
/// 1. Reads `user_version` (0 = no migrations applied yet).
/// 2. Applies `migrations[user_version..]` in order.
/// 3. Increments `user_version` after each successful migration so a crash
///    mid-run resumes from the right point.
///
/// Idempotency rules for migration SQL:
/// - Use `CREATE TABLE IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS`.
/// - `ALTER TABLE … ADD COLUMN` duplicate-column errors are silently skipped
///   (safe for databases created before this versioning was added).
pub(crate) fn run_migrations(conn: &Connection, migrations: &[&str]) -> rusqlite::Result<()> {
    let current: usize = conn
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .unwrap_or(0) as usize;

    if current > migrations.len() {
        tracing::warn!(
            "sqlite user_version ({}) is ahead of known migrations ({}). Skipping.",
            current,
            migrations.len()
        );
        return Ok(());
    }

    for (i, sql) in migrations[current..].iter().enumerate() {
        let next_version = (current + i + 1) as i64;
        match conn.execute_batch(sql) {
            Ok(()) => tracing::debug!("sqlite migration v{} applied", next_version),
            Err(e) if e.to_string().contains("duplicate column name") => {
                tracing::debug!("sqlite migration v{} already applied", next_version);
            }
            Err(e) => return Err(e),
        }
        conn.pragma_update(None, "user_version", next_version)?;
    }
    Ok(())
}
