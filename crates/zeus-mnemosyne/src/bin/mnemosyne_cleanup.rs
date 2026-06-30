//! mnemosyne-cleanup — one-shot purge of duplicate and scaffolding memories.
//!
//! Background: a Tier-0 message store filter (commit a1de13f7) prevents NEW
//! junk from being stored, but existing Mnemosyne DBs across the fleet still
//! contain large piles of scaffolding (skill docs / SOUL / AGENTS / HEARTBEAT
//! fragments stored as `role='system'`) and synthetic `file:*` sessions.
//!
//! This binary applies the SAME filter rules as `MessageStoreFilter` to the
//! existing rows, deletes junk, dedupes exact duplicates, and rebuilds FTS.
//!
//! Usage:
//!   mnemosyne-cleanup --db ~/.zeus/memory.db                  # dry run (default)
//!   mnemosyne-cleanup --db ~/.zeus/memory.db --apply          # actually delete
//!   mnemosyne-cleanup --db ~/.zeus/memory.db --apply --vacuum # also VACUUM
//!
//! Multiple --db flags can be passed to clean a fleet of DBs in one shot.

use rusqlite::{Connection, params};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Default)]
struct CleanupStats {
    system_role_rows: i64,
    file_session_rows: i64,
    empty_content_rows: i64,
    heartbeat_noop_rows: i64,
    chat_ack_rows: i64,
    duplicate_rows: i64,
    deleted: i64,
    bytes_freed: i64,
    pre_total: i64,
    post_total: i64,
}

#[derive(Debug)]
struct Args {
    dbs: Vec<PathBuf>,
    apply: bool,
    vacuum: bool,
    verbose: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut dbs = Vec::new();
    let mut apply = false;
    let mut vacuum = false;
    let mut verbose = false;
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--db" => {
                let p = iter.next().ok_or("--db requires a path")?;
                dbs.push(PathBuf::from(p));
            }
            "--apply" => apply = true,
            "--vacuum" => vacuum = true,
            "-v" | "--verbose" => verbose = true,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {}", other)),
        }
    }
    if dbs.is_empty() {
        // Default: try ~/.zeus/memory.db
        if let Some(home) = std::env::var_os("HOME") {
            let default_db = PathBuf::from(home).join(".zeus").join("memory.db");
            if default_db.exists() {
                dbs.push(default_db);
            }
        }
    }
    if dbs.is_empty() {
        return Err("no --db paths given and ~/.zeus/memory.db does not exist".into());
    }
    Ok(Args { dbs, apply, vacuum, verbose })
}

fn print_help() {
    println!(
        "mnemosyne-cleanup — purge scaffolding & duplicate memories\n\n\
         USAGE:\n\
         \tmnemosyne-cleanup [--db PATH]... [--apply] [--vacuum] [-v]\n\n\
         FLAGS:\n\
         \t--db PATH   Path to a Mnemosyne SQLite DB (repeatable). Defaults to ~/.zeus/memory.db.\n\
         \t--apply     Actually perform deletions. Without this it's a dry-run.\n\
         \t--vacuum    After --apply, run VACUUM to reclaim disk space.\n\
         \t-v, --verbose  Print per-pattern row samples.\n\
         \t-h, --help     Show this help.\n\n\
         WHAT GETS PURGED (matches Tier-0 MessageStoreFilter rules):\n\
         \t1. role='system' messages (scaffolding: SOUL/AGENTS/HEARTBEAT/IDENTITY/skill docs)\n\
         \t2. session_id LIKE 'file:%' (synthetic sessions from prompt-building doc ingest)\n\
         \t3. Empty / whitespace-only content with no tool_results\n\
         \t4. HEARTBEAT_OK / [HEARTBEAT] no-op rows\n\
         \t5. Pure chat acks (ok/nice/got it/thanks/lol/👍 …)\n\
         \t6. Exact duplicates (same session_id+role+content) — keeps lowest id\n"
    );
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {}", e);
            print_help();
            return ExitCode::from(2);
        }
    };

    let mode = if args.apply { "APPLY" } else { "DRY-RUN" };
    println!("== mnemosyne-cleanup ({}) ==", mode);
    println!("targets: {} db(s)\n", args.dbs.len());

    let mut had_error = false;
    for db_path in &args.dbs {
        match clean_one(db_path, args.apply, args.vacuum, args.verbose) {
            Ok(s) => print_stats(db_path, &s, args.apply),
            Err(e) => {
                eprintln!("[{}] ERROR: {}", db_path.display(), e);
                had_error = true;
            }
        }
    }

    if !args.apply {
        println!("\n(no changes made — re-run with --apply to commit)");
    }

    if had_error { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

fn clean_one(
    db_path: &PathBuf,
    apply: bool,
    vacuum: bool,
    verbose: bool,
) -> Result<CleanupStats, String> {
    if !db_path.exists() {
        return Err(format!("file not found: {}", db_path.display()));
    }

    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    // Ensure messages table exists
    let has_messages: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='messages'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if has_messages == 0 {
        return Err("no `messages` table — not a Mnemosyne DB".into());
    }

    let mut s = CleanupStats::default();

    s.pre_total = conn
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;

    // Build a set of message ids to delete via CTE-style selection.
    // We collect into a temp table for visibility + atomic delete.
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS _to_delete(id INTEGER PRIMARY KEY);
         DELETE FROM _to_delete;",
    )
    .map_err(|e| e.to_string())?;

    // Rule 1: role='system'  — covers SOUL/AGENTS/HEARTBEAT/IDENTITY/skill docs
    s.system_role_rows = tx
        .execute(
            "INSERT OR IGNORE INTO _to_delete(id) \
             SELECT id FROM messages WHERE role='system'",
            [],
        )
        .map_err(|e| e.to_string())? as i64;

    // Rule 2: session_id LIKE 'file:%'  — synthetic file-ingest sessions
    s.file_session_rows = tx
        .execute(
            "INSERT OR IGNORE INTO _to_delete(id) \
             SELECT id FROM messages WHERE session_id LIKE 'file:%'",
            [],
        )
        .map_err(|e| e.to_string())? as i64;

    // Rule 3: empty content & no tool_results
    s.empty_content_rows = tx
        .execute(
            "INSERT OR IGNORE INTO _to_delete(id) \
             SELECT id FROM messages \
             WHERE TRIM(content)='' \
               AND (tool_results IS NULL OR tool_results='' OR tool_results='[]')",
            [],
        )
        .map_err(|e| e.to_string())? as i64;

    // Rule 4: HEARTBEAT_OK / [HEARTBEAT] no-ops (loose match — bare ok or status-only)
    s.heartbeat_noop_rows = tx
        .execute(
            "INSERT OR IGNORE INTO _to_delete(id) \
             SELECT id FROM messages \
             WHERE TRIM(content) = 'HEARTBEAT_OK' \
                OR TRIM(content) LIKE 'HEARTBEAT_OK %' \
                OR TRIM(content) LIKE '[HEARTBEAT]%'",
            [],
        )
        .map_err(|e| e.to_string())? as i64;

    // Rule 5: pure chat acks
    let ack_pats = [
        "ok", "oke", "okay", "nice", "got it", "thanks", "thx", "sure", "yes", "no", "lol",
        "👍", "👍🏿", "😅", "🔥", "⚡",
    ];
    let mut ack_total = 0i64;
    for p in &ack_pats {
        ack_total += tx
            .execute(
                "INSERT OR IGNORE INTO _to_delete(id) \
                 SELECT id FROM messages \
                 WHERE LOWER(TRIM(content)) = ?1",
                params![*p],
            )
            .map_err(|e| e.to_string())? as i64;
    }
    s.chat_ack_rows = ack_total;

    // Rule 6: exact duplicates — keep lowest id per (session_id, role, content)
    s.duplicate_rows = tx
        .execute(
            "INSERT OR IGNORE INTO _to_delete(id) \
             SELECT id FROM messages WHERE id NOT IN (\
                 SELECT MIN(id) FROM messages GROUP BY session_id, role, content\
             )",
            [],
        )
        .map_err(|e| e.to_string())? as i64;

    // Compute final delete count + bytes (intersection-aware, since rules overlap)
    let final_count: i64 = tx
        .query_row("SELECT COUNT(*) FROM _to_delete", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let bytes: i64 = tx
        .query_row(
            "SELECT COALESCE(SUM(LENGTH(m.content)),0) \
             FROM messages m JOIN _to_delete d ON m.id=d.id",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    s.deleted = final_count;
    s.bytes_freed = bytes;

    if verbose {
        let mut stmt = tx
            .prepare(
                "SELECT m.role, m.session_id, substr(m.content,1,80) \
                 FROM messages m JOIN _to_delete d ON m.id=d.id LIMIT 5",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        println!("  [{}] sample rows queued for deletion:", db_path.display());
        for row in rows.flatten() {
            println!("    role={:?} session={} :: {}", row.0, row.1, row.2.replace('\n', " "));
        }
    }

    if apply {
        // Cascade-aware deletes — entity_mentions/relationships reference messages.id
        // Best-effort: delete dependent rows first, ignore if tables don't exist.
        let _ = tx.execute(
            "DELETE FROM entity_mentions WHERE message_id IN (SELECT id FROM _to_delete)",
            [],
        );
        let _ = tx.execute(
            "DELETE FROM messages WHERE id IN (SELECT id FROM _to_delete)",
            [],
        );

        // Rebuild FTS index if present
        let has_fts: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='messages_fts'",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if has_fts > 0 {
            let _ = tx.execute("INSERT INTO messages_fts(messages_fts) VALUES('rebuild')", []);
        }

        tx.commit().map_err(|e| e.to_string())?;

        s.post_total = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;

        if vacuum {
            conn.execute_batch("VACUUM").map_err(|e| e.to_string())?;
        }
    } else {
        // Dry run — rollback temp inserts
        tx.rollback().map_err(|e| e.to_string())?;
        s.post_total = s.pre_total - s.deleted;
    }

    Ok(s)
}

fn print_stats(path: &PathBuf, s: &CleanupStats, applied: bool) {
    println!("[{}]", path.display());
    println!("  Pre-cleanup rows:     {}", s.pre_total);
    println!("  Rule 1 system role:   {}", s.system_role_rows);
    println!("  Rule 2 file:* session:{}", s.file_session_rows);
    println!("  Rule 3 empty content: {}", s.empty_content_rows);
    println!("  Rule 4 heartbeat-nop: {}", s.heartbeat_noop_rows);
    println!("  Rule 5 chat acks:     {}", s.chat_ack_rows);
    println!("  Rule 6 duplicates:    {}", s.duplicate_rows);
    println!("  ----------------------------");
    println!("  TOTAL queued:         {} rows", s.deleted);
    println!("  Bytes freed:          {} ({:.1} KB)", s.bytes_freed, s.bytes_freed as f64 / 1024.0);
    println!("  Post-cleanup rows:    {} ({})", s.post_total, if applied { "actual" } else { "projected" });
    println!();
}
