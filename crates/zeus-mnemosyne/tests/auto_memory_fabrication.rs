//! Integration tests for the auto-memory-fabrication fence (#53).
//!
//! Validates the trilogy of protections shipped across #53.1 (v8 schema),
//! #2a (read-path filter), and #2b-i (UDF + AFTER INSERT trigger + mark_retracted):
//!
//! 1. `looks_like_unverified_sha_content` regex behavior (positive + negative cases)
//! 2. UDF callable via SQL (`SELECT looks_like_unverified_sha(?)`)
//! 3. Trigger FIRES on clean content — verified flips 0→1 post-INSERT
//! 4. Trigger NO-OP on SHA-shaped content — verified stays 0
//! 5. `mark_retracted` atomic + idempotent — UPDATE + INSERT OR IGNORE
//! 6. Full integration — ingest mixed content, export_memory_summary respects verified=0
//!
//! Each test uses an isolated tempdir-backed SQLite file via `MemoryStore::new`.

use rusqlite::params;
use std::path::PathBuf;
use tempfile::tempdir;
use zeus_mnemosyne::{looks_like_unverified_sha_content, MemoryStore};

fn fresh_store() -> (tempfile::TempDir, MemoryStore) {
    let dir = tempdir().expect("tempdir");
    let db_path: PathBuf = dir.path().join("mem.db");
    let store = MemoryStore::new(&db_path, false, false).expect("MemoryStore::new");
    (dir, store)
}

// ─── Test 1: helper-fn regex behavior ────────────────────────────────────────

#[test]
fn test_looks_like_unverified_sha_content_regex_behavior() {
    // Positive cases: 10–40 char hex runs with non-hex boundaries
    assert!(
        looks_like_unverified_sha_content("commit 5ceefad1b55 landed"),
        "11-char hex SHA should match"
    );
    assert!(
        looks_like_unverified_sha_content("SHA 263d05c59525ed71ed88b3349dc0a29c204eea1c"),
        "40-char full SHA should match"
    );
    assert!(
        looks_like_unverified_sha_content("ref=2fd928774c48961bd6e881d85258f419ccee0153 done"),
        "embedded 40-char SHA should match"
    );
    assert!(
        looks_like_unverified_sha_content("0a44e717abc"),
        "exactly 11-char hex at string bounds should match"
    );

    // Negative cases: too-short, non-hex, no boundary, or empty
    assert!(
        !looks_like_unverified_sha_content("commit abc123 landed"),
        "6-char hex (below 10-char floor) should NOT match"
    );
    assert!(
        !looks_like_unverified_sha_content("hello world no hex here"),
        "plain prose with no hex should NOT match"
    );
    assert!(
        !looks_like_unverified_sha_content(""),
        "empty string should NOT match"
    );
    assert!(
        !looks_like_unverified_sha_content("xyznothex1234567890"),
        "hex-adjacent letters break the word boundary requirement on the front"
    );
}

// ─── Test 2: UDF callable via SQL ────────────────────────────────────────────

#[test]
fn test_udf_callable_via_sql() {
    let (_dir, store) = fresh_store();
    let conn = store.conn();

    let positive: i64 = conn
        .query_row(
            "SELECT looks_like_unverified_sha(?1)",
            params!["commit 5ceefad1b55 landed"],
            |r| r.get(0),
        )
        .expect("UDF positive query");
    assert_eq!(positive, 1, "UDF should return 1 for SHA-shaped content");

    let negative: i64 = conn
        .query_row(
            "SELECT looks_like_unverified_sha(?1)",
            params!["hello world no hex here"],
            |r| r.get(0),
        )
        .expect("UDF negative query");
    assert_eq!(negative, 0, "UDF should return 0 for clean content");
}

// ─── Test 3: Trigger FIRES on clean content (verified 0→1) ───────────────────

#[test]
fn test_trigger_fires_on_clean_content() {
    let (_dir, store) = fresh_store();
    let conn = store.conn();

    // Insert with explicit verified=0; trigger should flip to 1 because content is clean.
    conn.execute(
        "INSERT INTO messages (session_id, role, content, timestamp, verified)
         VALUES (?1, ?2, ?3, ?4, 0)",
        params![
            "session-clean",
            "assistant",
            "Just a normal response with no SHA-shaped content at all.",
            "2026-05-15T19:00:00Z"
        ],
    )
    .expect("INSERT clean");

    let verified: i64 = conn
        .query_row(
            "SELECT verified FROM messages WHERE session_id = ?1",
            params!["session-clean"],
            |r| r.get(0),
        )
        .expect("SELECT verified");
    assert_eq!(verified, 1, "trigger should flip clean content to verified=1");
}

// ─── Test 4: Trigger NO-OP on SHA-shaped content (stays 0) ───────────────────

#[test]
fn test_trigger_noop_on_sha_shaped_content() {
    let (_dir, store) = fresh_store();
    let conn = store.conn();

    // Insert with explicit verified=0; trigger should NOT flip because content is SHA-shaped.
    conn.execute(
        "INSERT INTO messages (session_id, role, content, timestamp, verified)
         VALUES (?1, ?2, ?3, ?4, 0)",
        params![
            "session-sha",
            "assistant",
            "shipped 263d05c59525ed71ed88b3349dc0a29c204eea1c to main",
            "2026-05-15T19:00:00Z"
        ],
    )
    .expect("INSERT SHA-shaped");

    let verified: i64 = conn
        .query_row(
            "SELECT verified FROM messages WHERE session_id = ?1",
            params!["session-sha"],
            |r| r.get(0),
        )
        .expect("SELECT verified");
    assert_eq!(
        verified, 0,
        "trigger should NOT flip SHA-shaped content; stays verified=0"
    );
}

// ─── Test 5: mark_retracted atomic + idempotent ──────────────────────────────

#[test]
fn test_mark_retracted_atomic_idempotent() {
    let (_dir, store) = fresh_store();

    // Seed a verified=1 message
    {
        let conn = store.conn();
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, timestamp, verified)
             VALUES (?1, ?2, ?3, ?4, ?5, 1)",
            params![
                42i64,
                "session-retract",
                "assistant",
                "Initially clean content",
                "2026-05-15T19:00:00Z"
            ],
        )
        .expect("seed message");
    }

    // First retraction: flips verified to 0 + inserts retractions row
    store
        .mark_retracted(42, "fabrication-detected")
        .expect("mark_retracted first");

    {
        let conn = store.conn();
        let verified: i64 = conn
            .query_row(
                "SELECT verified FROM messages WHERE id = 42",
                [],
                |r| r.get(0),
            )
            .expect("SELECT verified");
        assert_eq!(verified, 0, "first mark_retracted should flip verified to 0");

        let retraction_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM retractions WHERE message_id = 42",
                [],
                |r| r.get(0),
            )
            .expect("count retractions");
        assert_eq!(retraction_count, 1, "first call should create 1 retraction row");
    }

    // Second call with SAME reason: idempotent via INSERT OR IGNORE
    store
        .mark_retracted(42, "fabrication-detected")
        .expect("mark_retracted idempotent");

    {
        let conn = store.conn();
        let retraction_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM retractions WHERE message_id = 42",
                [],
                |r| r.get(0),
            )
            .expect("count retractions after idempotent call");
        assert_eq!(
            retraction_count, 1,
            "idempotent second call must NOT duplicate retraction row"
        );
    }
}

// ─── Test 6: Full integration — export_memory_summary respects verified=0 ────

#[test]
fn test_export_filter_respects_verified() {
    let (_dir, store) = fresh_store();

    // Seed: 1 clean (trigger flips 1), 1 SHA-shaped (stays 0), 1 explicit retraction (flip to 0).
    {
        let conn = store.conn();

        // All rows seeded as semantic + importance=0.8 to bypass the
        // "low-importance episodic" filter in export_memory_summary —
        // we're specifically validating the `verified` filter path here.

        // Row 1: clean → trigger fires → verified=1 → INCLUDED in export
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, timestamp, memory_type, importance, verified)
             VALUES (1, 'session-export', 'assistant', 'Clean architectural note about fences.', '2026-05-15T19:00:00Z', 'semantic', 0.8, 0)",
            [],
        )
        .expect("INSERT clean");

        // Row 2: SHA-shaped → trigger no-op → verified=0 → EXCLUDED from export
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, timestamp, memory_type, importance, verified)
             VALUES (2, 'session-export', 'assistant', 'commit dfe739e2abc landed on main', '2026-05-15T19:01:00Z', 'semantic', 0.8, 0)",
            [],
        )
        .expect("INSERT sha-shaped");

        // Row 3: clean → trigger fires → verified=1, then explicit retraction → verified=0 → EXCLUDED
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, timestamp, memory_type, importance, verified)
             VALUES (3, 'session-export', 'assistant', 'Another clean note that will be retracted.', '2026-05-15T19:02:00Z', 'semantic', 0.8, 0)",
            [],
        )
        .expect("INSERT to-be-retracted");
    }

    store
        .mark_retracted(3, "user-corrected")
        .expect("mark_retracted row 3");

    // export_memory_summary should only surface row 1.
    let export = store
        .export_memory_summary(100)
        .expect("export_memory_summary");

    assert!(
        export.contains("Clean architectural note about fences."),
        "verified=1 clean content MUST appear in export. got: {}",
        export
    );
    assert!(
        !export.contains("dfe739e2abc"),
        "SHA-shaped (verified=0 via trigger no-op) MUST be filtered from export. got: {}",
        export
    );
    assert!(
        !export.contains("Another clean note that will be retracted."),
        "explicitly retracted (verified=0 via mark_retracted) MUST be filtered from export. got: {}",
        export
    );
}
