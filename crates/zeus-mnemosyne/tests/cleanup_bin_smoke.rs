//! Smoke test for mnemosyne-cleanup binary.
//! Builds a fixture DB matching Mnemosyne schema, seeds junk + keepers,
//! runs the binary in --apply mode, asserts only keepers survive.

use rusqlite::{Connection, params};
use std::process::Command;
use tempfile::tempdir;

fn seed_db(path: &std::path::Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            tool_calls TEXT,
            tool_results TEXT,
            timestamp TEXT NOT NULL,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .unwrap();

    let now = "2026-04-27T00:00:00Z";
    let insert = |sid: &str, role: &str, content: &str| {
        conn.execute(
            "INSERT INTO messages (session_id, role, content, timestamp) VALUES (?1, ?2, ?3, ?4)",
            params![sid, role, content, now],
        )
        .unwrap();
    };

    // Junk: system role
    insert("session-a", "system", "# AGENTS.md - scaffolding");
    // Junk: file:* synthetic session (assistant role!)
    insert("file:skills/foo.md", "assistant", "skill content dump");
    // Junk: empty
    insert("session-b", "user", "   ");
    // Junk: heartbeat noop
    insert("session-b", "user", "HEARTBEAT_OK");
    // Junk: chat ack
    insert("session-c", "user", "ok");
    insert("session-c", "user", "👍");
    // Junk: exact duplicate (keep one)
    insert("session-d", "user", "real message about a task");
    insert("session-d", "user", "real message about a task");

    // Keepers
    insert("session-d", "user", "another real message");
    insert(
        "session-e",
        "assistant",
        "I completed the task and here is the result with substantial content",
    );
}

fn count_rows(path: &std::path::Path) -> i64 {
    let conn = Connection::open(path).unwrap();
    conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap()
}

fn bin_path() -> std::path::PathBuf {
    // CARGO_BIN_EXE_<bin-name> is set by cargo for integration tests
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_mnemosyne-cleanup"))
}

#[test]
fn dry_run_does_not_modify_db() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("test.db");
    seed_db(&db);
    assert_eq!(count_rows(&db), 10);

    let out = Command::new(bin_path())
        .args(["--db", db.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));

    // No changes
    assert_eq!(count_rows(&db), 10);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("DRY-RUN"));
    assert!(stdout.contains("TOTAL queued:"));
}

#[test]
fn apply_purges_junk_keeps_real() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("test.db");
    seed_db(&db);
    assert_eq!(count_rows(&db), 10);

    let out = Command::new(bin_path())
        .args(["--db", db.to_str().unwrap(), "--apply"])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));

    let remaining = count_rows(&db);
    // Keepers: "another real message", the assistant complete reply,
    // and one of the duplicate "real message about a task" pair
    assert_eq!(remaining, 3, "expected 3 keepers, got {}", remaining);

    // Verify junk is gone
    let conn = Connection::open(&db).unwrap();
    let system_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM messages WHERE role='system'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(system_count, 0);
    let file_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM messages WHERE session_id LIKE 'file:%'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(file_count, 0);
}

#[test]
fn rejects_non_mnemosyne_db() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("empty.db");
    Connection::open(&db).unwrap(); // create empty DB

    let out = Command::new(bin_path())
        .args(["--db", db.to_str().unwrap(), "--apply"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("messages") || stderr.contains("Mnemosyne"));
}

// ============================================================================
// #55c regression-fence: subprocess-arg surface for `zeus reset` orchestrator.
// Closes Axis 4 + `cargo-check-blind-spot-runtime-subprocess-arg-strings-not-static-verifiable` bank.
//
// The #55 B-cut originally invoked `mnemosyne-cleanup --all` (a flag that does
// not exist on this binary at the ancestry-grounded base). `cargo check` cannot
// catch runtime subprocess-arg drift; these tests fence the exact bug-class.
// ============================================================================

#[test]
fn accepts_apply_vacuum_args() {
    // Positive case: the flag-pair invoked by `zeus reset` orchestrator must
    // exit success against a valid Mnemosyne-schema DB.
    let dir = tempdir().unwrap();
    let db = dir.path().join("test.db");
    seed_db(&db);

    let out = Command::new(bin_path())
        .args(["--db", db.to_str().unwrap(), "--apply", "--vacuum"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "expected --apply --vacuum to succeed; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn rejects_legacy_all_arg() {
    // Regression-fence: `--all` was the original B-cut subprocess invocation;
    // this binary's CLI never accepted it. If a future refactor reintroduces
    // `--all` to zeus reset's subprocess call without also adding it here,
    // this test catches it at `cargo test` time (where `cargo check` cannot).
    let out = Command::new(bin_path())
        .arg("--all")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "expected --all to be rejected; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
