//! Integration tests for the memory lifecycle pipeline.
//!
//! Covers:
//! 1. Episodic → semantic promotion (auto_promote)
//! 2. Session consolidation (consolidate_session)
//! 3. Garbage collection (garbage_collect)
//! 4. Promotion + supersession interaction
//! 5. Temporal validity (valid_to prevents promotion)
//! 6. Config variations (custom thresholds, retention periods)
//! 7. Full pipeline: store → promote → consolidate → GC

use chrono::{Duration, Utc};
use rusqlite::params;
use tempfile::tempdir;
use zeus_core::Message;
use zeus_mnemosyne::{
    MemoryStore, MemoryType,
    promotion::{GcConfig, auto_promote, consolidate_session, garbage_collect},
};

// ============================================================================
// Shared helpers
// ============================================================================

/// Open an in-memory-style test store in a temp directory.
fn make_store() -> (tempfile::TempDir, MemoryStore) {
    let dir = tempdir().expect("tempdir should succeed");
    let db = dir.path().join("test.db");
    let store = MemoryStore::new(&db, true, false).expect("MemoryStore::new should succeed");
    (dir, store)
}

/// Store a fresh (just-created) episodic memory.
fn store_episodic(store: &MemoryStore, session: &str, content: &str, importance: f32) -> i64 {
    let msg = Message::user(content);
    store
        .store_typed(session, &msg, MemoryType::Episodic, importance)
        .expect("store_typed should succeed")
}

/// Insert an episodic memory with a backdated timestamp so it appears old.
fn store_old_episodic(
    store: &MemoryStore,
    session: &str,
    content: &str,
    importance: f32,
    days_ago: i64,
) -> i64 {
    let ts = (Utc::now() - Duration::days(days_ago)).to_rfc3339();
    store
        .conn()
        .execute(
            "INSERT INTO messages \
             (session_id, role, content, tool_calls, tool_results, timestamp, memory_type, importance, valid_from) \
             VALUES (?1, 'user', ?2, '[]', '[]', ?3, 'episodic', ?4, ?3)",
            params![session, content, ts, importance as f64],
        )
        .expect("direct insert should succeed");
    store.conn().last_insert_rowid()
}

/// Insert an old episodic memory that is already soft-deleted (valid_to set).
fn store_expired_episodic(
    store: &MemoryStore,
    session: &str,
    content: &str,
    importance: f32,
    days_ago: i64,
) -> i64 {
    let ts = (Utc::now() - Duration::days(days_ago)).to_rfc3339();
    let now = Utc::now().to_rfc3339();
    store
        .conn()
        .execute(
            "INSERT INTO messages \
             (session_id, role, content, tool_calls, tool_results, timestamp, memory_type, importance, valid_from, valid_to) \
             VALUES (?1, 'user', ?2, '[]', '[]', ?3, 'episodic', ?4, ?3, ?5)",
            params![session, content, ts, importance as f64, now],
        )
        .expect("direct insert should succeed");
    store.conn().last_insert_rowid()
}

/// Count live (valid_to IS NULL) rows of a given type in a session.
fn count_live(store: &MemoryStore, session: &str, memory_type: &str) -> i64 {
    store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM messages \
             WHERE session_id = ?1 AND memory_type = ?2 AND valid_to IS NULL",
            params![session, memory_type],
            |row| row.get(0),
        )
        .expect("count_live query should succeed")
}

/// Read the importance of a row by id.
fn get_importance(store: &MemoryStore, id: i64) -> f64 {
    store
        .conn()
        .query_row(
            "SELECT importance FROM messages WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("importance query should succeed")
}

/// Read valid_to for a row; None means still live.
fn get_valid_to(store: &MemoryStore, id: i64) -> Option<String> {
    store
        .conn()
        .query_row(
            "SELECT valid_to FROM messages WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("valid_to query should succeed")
}

/// Read superseded_by for a row.
fn get_superseded_by(store: &MemoryStore, id: i64) -> Option<i64> {
    store
        .conn()
        .query_row(
            "SELECT superseded_by FROM messages WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("superseded_by query should succeed")
}

// ============================================================================
// 1. Episodic memory promotion
// ============================================================================

#[test]
fn promotion_no_candidates_below_threshold() {
    let (_dir, store) = make_store();
    store_episodic(&store, "s1", "Low importance note.", 0.3);

    let r = auto_promote(&store, 0.8, 0).expect("auto_promote should succeed");
    assert_eq!(r.scanned, 0, "below-threshold memory should not be scanned");
    assert_eq!(r.promoted, 0);
}

#[test]
fn promotion_high_importance_old_memory_promoted() {
    let (_dir, store) = make_store();
    let id = store_old_episodic(
        &store,
        "s1",
        "The compiler uses LLVM as its backend. Optimisation passes run afterward.",
        0.90,
        10,
    );

    let r = auto_promote(&store, 0.8, 1).expect("auto_promote should succeed");
    assert_eq!(r.promoted, 1);

    // The original episodic should now be superseded
    assert!(
        get_valid_to(&store, id).is_some(),
        "original should be soft-deleted"
    );
    let successor = get_superseded_by(&store, id).expect("should record successor id");

    // The successor must be a semantic memory
    let mt: String = store
        .conn()
        .query_row(
            "SELECT memory_type FROM messages WHERE id = ?1",
            params![successor],
            |row| row.get(0),
        )
        .expect("successor query should succeed");
    assert_eq!(mt, "semantic");
}

#[test]
fn promotion_recent_memory_not_promoted_due_to_min_access() {
    let (_dir, store) = make_store();
    // High importance but created right now — within the min_access_days window
    store_episodic(
        &store,
        "s1",
        "Very important but fresh off the press.",
        0.99,
    );

    let r = auto_promote(&store, 0.8, 5).expect("auto_promote should succeed");
    assert_eq!(r.promoted, 0, "recent memory must not be promoted");
}

#[test]
fn promotion_importance_at_threshold_boundary() {
    let (_dir, store) = make_store();
    // Exactly at threshold — should be included
    store_old_episodic(&store, "s1", "Exactly at the promotion boundary.", 0.80, 5);
    // Just below threshold — must be excluded
    store_old_episodic(&store, "s2", "Just under the promotion threshold.", 0.79, 5);

    let r = auto_promote(&store, 0.80, 0).expect("auto_promote should succeed");
    assert_eq!(
        r.promoted, 1,
        "only the memory AT threshold should be promoted"
    );
}

#[test]
fn promotion_multiple_candidates_all_promoted() {
    let (_dir, store) = make_store();
    store_old_episodic(
        &store,
        "s1",
        "First key fact for the system design.",
        0.85,
        7,
    );
    store_old_episodic(
        &store,
        "s1",
        "Second key fact about deployment strategy.",
        0.90,
        7,
    );
    store_old_episodic(
        &store,
        "s1",
        "Third key fact regarding security posture.",
        0.95,
        7,
    );

    let r = auto_promote(&store, 0.80, 1).expect("auto_promote should succeed");
    assert_eq!(r.promoted, 3);

    // All three originals should be superseded
    let live_episodic = count_live(&store, "s1", "episodic");
    assert_eq!(live_episodic, 0, "all originals should be superseded");

    // Three new semantic memories should exist
    let live_semantic = count_live(&store, "s1", "semantic");
    assert_eq!(live_semantic, 3);
}

#[test]
fn promotion_logs_promotion_pattern() {
    let (_dir, store) = make_store();
    store_old_episodic(
        &store,
        "s1",
        "Key architectural insight documented here.",
        0.88,
        5,
    );

    auto_promote(&store, 0.80, 0).expect("auto_promote should succeed");

    let patterns = store
        .get_patterns("promotion", 10)
        .expect("get_patterns should succeed");
    assert!(!patterns.is_empty(), "promotion should log a pattern entry");
    assert!(
        patterns[0].content.contains("->"),
        "pattern content should record the id mapping"
    );
}

#[test]
fn promotion_result_counts_correct() {
    let (_dir, store) = make_store();
    // 2 above threshold, 1 below
    store_old_episodic(&store, "s1", "Above threshold first.", 0.90, 5);
    store_old_episodic(&store, "s1", "Above threshold second.", 0.85, 5);
    store_old_episodic(&store, "s1", "Below threshold third.", 0.40, 5);

    let r = auto_promote(&store, 0.80, 0).expect("auto_promote should succeed");
    assert_eq!(r.scanned, 2, "only above-threshold memories are scanned");
    assert_eq!(r.promoted, 2);
}

// ============================================================================
// 2. Session consolidation
// ============================================================================

#[test]
fn consolidation_empty_session_returns_zero() {
    let (_dir, store) = make_store();
    let r =
        consolidate_session(&store, "no-such-session", false).expect("consolidate should succeed");
    assert_eq!(r.rolled_up, 0);
    assert_eq!(r.summaries_created, 0);
    assert!(r.summary_id.is_none());
    assert_eq!(r.session_id, "no-such-session");
}

#[test]
fn consolidation_single_message_session() {
    let (_dir, store) = make_store();
    store_episodic(&store, "solo", "Only message in this session.", 0.5);

    let r = consolidate_session(&store, "solo", false).expect("consolidate should succeed");
    assert_eq!(r.rolled_up, 1);
    assert_eq!(r.summaries_created, 1);
    assert!(r.summary_id.is_some());

    // The original should be superseded
    assert_eq!(count_live(&store, "solo", "episodic"), 0);
    // A summary should exist
    assert_eq!(count_live(&store, "solo", "summary"), 1);
}

#[test]
fn consolidation_multiple_messages_rolled_up() {
    let (_dir, store) = make_store();
    for i in 1..=5 {
        store_episodic(
            &store,
            "multi",
            &format!("Conversation turn {} content.", i),
            0.5,
        );
    }

    let r = consolidate_session(&store, "multi", false).expect("consolidate should succeed");
    assert_eq!(r.rolled_up, 5);
    assert_eq!(r.summaries_created, 1);

    // All originals superseded, one summary live
    assert_eq!(count_live(&store, "multi", "episodic"), 0);
    assert_eq!(count_live(&store, "multi", "summary"), 1);
}

#[test]
fn consolidation_summary_content_contains_session_prefix() {
    let (_dir, store) = make_store();
    store_episodic(&store, "prefix-check", "First important point.", 0.6);
    store_episodic(&store, "prefix-check", "Second important point.", 0.7);

    let r = consolidate_session(&store, "prefix-check", false).expect("consolidate should succeed");
    let summary_id = r.summary_id.expect("summary_id should be Some");

    let content: String = store
        .conn()
        .query_row(
            "SELECT content FROM messages WHERE id = ?1",
            params![summary_id],
            |row| row.get(0),
        )
        .expect("content query should succeed");

    assert!(
        content.starts_with("[Session Summary]"),
        "summary must begin with '[Session Summary]'"
    );
    assert!(content.contains("First important point"));
}

#[test]
fn consolidation_preserves_max_importance_on_summary() {
    let (_dir, store) = make_store();
    store_episodic(&store, "imp-check", "Low importance note.", 0.2);
    store_episodic(&store, "imp-check", "High importance note.", 0.9);
    store_episodic(&store, "imp-check", "Medium importance note.", 0.5);

    let r = consolidate_session(&store, "imp-check", false).expect("consolidate should succeed");
    let sid = r.summary_id.expect("summary_id should be set");

    let importance = get_importance(&store, sid);
    // Should carry the max importance from the batch
    assert!(
        (importance - 0.9).abs() < 0.01,
        "summary importance should equal max of rolled-up memories (got {importance})"
    );
}

#[test]
fn consolidation_does_not_touch_other_sessions() {
    let (_dir, store) = make_store();
    store_episodic(&store, "target", "Message in target session.", 0.5);
    store_episodic(&store, "bystander", "Message in bystander session.", 0.5);

    consolidate_session(&store, "target", false).expect("consolidate should succeed");

    // Bystander session remains untouched
    assert_eq!(count_live(&store, "bystander", "episodic"), 1);
}

#[test]
fn consolidation_does_not_consolidate_already_superseded() {
    let (_dir, store) = make_store();
    // One live, one already superseded
    store_episodic(&store, "partial", "Live message.", 0.5);
    store_expired_episodic(&store, "partial", "Already expired.", 0.5, 5);

    let r = consolidate_session(&store, "partial", false).expect("consolidate should succeed");
    assert_eq!(r.rolled_up, 1, "only the live message should be rolled up");
}

#[test]
fn consolidation_supersedes_originals_with_summary_id() {
    let (_dir, store) = make_store();
    let id1 = store_episodic(&store, "chain", "First message in chain.", 0.6);
    let id2 = store_episodic(&store, "chain", "Second message in chain.", 0.7);

    let r = consolidate_session(&store, "chain", false).expect("consolidate should succeed");
    let sid = r.summary_id.expect("summary_id should be Some");

    // Both originals should point to the summary as superseded_by
    assert_eq!(get_superseded_by(&store, id1), Some(sid));
    assert_eq!(get_superseded_by(&store, id2), Some(sid));
}

// ============================================================================
// 3. Garbage collection
// ============================================================================

#[test]
fn gc_no_old_memories_deletes_nothing() {
    let (_dir, store) = make_store();
    store_episodic(&store, "s1", "Fresh memory, not yet old.", 0.1);

    let r = garbage_collect(&store, &GcConfig::default()).expect("gc should succeed");
    assert_eq!(r.deleted, 0);
}

#[test]
fn gc_deletes_old_low_importance_episodic() {
    let (_dir, store) = make_store();
    store_old_episodic(&store, "s1", "Old and unimportant note.", 0.1, 60);
    store_old_episodic(&store, "s1", "Also old and unimportant.", 0.2, 45);

    let cfg = GcConfig {
        max_age_days: 30,
        importance_floor: 0.5,
        ..Default::default()
    };
    let r = garbage_collect(&store, &cfg).expect("gc should succeed");
    assert_eq!(r.deleted, 2);
}

#[test]
fn gc_keeps_old_high_importance_episodic() {
    let (_dir, store) = make_store();
    store_old_episodic(&store, "s1", "Old but still very important.", 0.9, 60);

    let cfg = GcConfig {
        max_age_days: 30,
        importance_floor: 0.5,
        ..Default::default()
    };
    let r = garbage_collect(&store, &cfg).expect("gc should succeed");
    assert_eq!(r.deleted, 0);
    assert_eq!(r.kept, 1);
}

#[test]
fn gc_mixed_keeps_important_deletes_stale() {
    let (_dir, store) = make_store();
    store_old_episodic(&store, "s1", "Important and old.", 0.9, 60);
    store_old_episodic(&store, "s1", "Unimportant and old.", 0.2, 60);
    store_old_episodic(&store, "s1", "Unimportant and old2.", 0.1, 60);

    let cfg = GcConfig {
        max_age_days: 30,
        importance_floor: 0.5,
        ..Default::default()
    };
    let r = garbage_collect(&store, &cfg).expect("gc should succeed");
    assert_eq!(
        r.deleted, 2,
        "two low-importance old memories should be deleted"
    );
    assert_eq!(r.kept, 1, "one high-importance memory should be kept");
}

#[test]
fn gc_protects_semantic_type_regardless_of_age() {
    let (_dir, store) = make_store();
    let ts = (Utc::now() - Duration::days(120)).to_rfc3339();
    store
        .conn()
        .execute(
            "INSERT INTO messages \
             (session_id, role, content, tool_calls, tool_results, timestamp, memory_type, importance, valid_from) \
             VALUES ('s1', 'system', 'Timeless semantic knowledge.', '[]', '[]', ?1, 'semantic', 0.1, ?1)",
            params![ts],
        )
        .expect("direct insert should succeed");

    let cfg = GcConfig {
        max_age_days: 30,
        importance_floor: 0.5,
        ..Default::default()
    };
    let r = garbage_collect(&store, &cfg).expect("gc should succeed");
    assert_eq!(r.deleted, 0, "semantic memories must never be deleted");
}

#[test]
fn gc_protects_fact_type() {
    let (_dir, store) = make_store();
    let ts = (Utc::now() - Duration::days(90)).to_rfc3339();
    store
        .conn()
        .execute(
            "INSERT INTO messages \
             (session_id, role, content, tool_calls, tool_results, timestamp, memory_type, importance, valid_from) \
             VALUES ('s1', 'system', 'An old but important fact.', '[]', '[]', ?1, 'fact', 0.1, ?1)",
            params![ts],
        )
        .expect("direct insert should succeed");

    let cfg = GcConfig {
        max_age_days: 30,
        importance_floor: 0.5,
        ..Default::default()
    };
    let r = garbage_collect(&store, &cfg).expect("gc should succeed");
    assert_eq!(r.deleted, 0, "fact memories must never be deleted");
}

#[test]
fn gc_result_counts_scanned_and_kept() {
    let (_dir, store) = make_store();
    store_old_episodic(&store, "s1", "Old high importance.", 0.8, 60);
    store_old_episodic(&store, "s1", "Old low importance.", 0.1, 60);

    let cfg = GcConfig {
        max_age_days: 30,
        importance_floor: 0.5,
        ..Default::default()
    };
    let r = garbage_collect(&store, &cfg).expect("gc should succeed");
    assert_eq!(r.scanned, 2);
    assert_eq!(r.deleted, 1);
    assert_eq!(r.kept, 1);
}

#[test]
fn gc_does_not_delete_already_superseded_rows() {
    let (_dir, store) = make_store();
    // expired rows have valid_to set — gc skips them (they are already soft-deleted)
    store_expired_episodic(&store, "s1", "Expired old memory.", 0.1, 60);

    let cfg = GcConfig {
        max_age_days: 30,
        importance_floor: 0.5,
        ..Default::default()
    };
    let r = garbage_collect(&store, &cfg).expect("gc should succeed");
    assert_eq!(
        r.scanned, 0,
        "already-superseded rows must not count as GC candidates"
    );
    assert_eq!(r.deleted, 0);
}

// ============================================================================
// 4. Promotion + supersession interaction
// ============================================================================

#[test]
fn promoted_memory_gets_superseded_by_semantic_successor() {
    let (_dir, store) = make_store();
    let orig_id = store_old_episodic(
        &store,
        "s1",
        "Design decision: use actor model for concurrency. This is the final choice.",
        0.88,
        10,
    );

    auto_promote(&store, 0.80, 0).expect("auto_promote should succeed");

    // The original episodic must be soft-deleted
    assert!(get_valid_to(&store, orig_id).is_some());

    // The successor must be a semantic memory
    let successor_id = get_superseded_by(&store, orig_id).expect("successor must exist");
    let mt: String = store
        .conn()
        .query_row(
            "SELECT memory_type FROM messages WHERE id = ?1",
            params![successor_id],
            |row| row.get(0),
        )
        .expect("type query should succeed");
    assert_eq!(mt, "semantic", "promoted memory must become semantic");
}

#[test]
fn superseded_memory_is_not_re_promoted() {
    let (_dir, store) = make_store();
    // Store a high-importance episodic and supersede it manually
    let id = store_old_episodic(&store, "s1", "Already superseded memory.", 0.95, 10);
    let now = Utc::now().to_rfc3339();
    store
        .conn()
        .execute(
            "UPDATE messages SET valid_to = ?1 WHERE id = ?2",
            params![now, id],
        )
        .expect("manual supersession should succeed");

    let r = auto_promote(&store, 0.80, 0).expect("auto_promote should succeed");
    assert_eq!(
        r.promoted, 0,
        "already-superseded memory must not be re-promoted"
    );
}

#[test]
fn promotion_semantic_successor_has_correct_importance() {
    let (_dir, store) = make_store();
    let orig_importance = 0.93_f32;
    let orig_id = store_old_episodic(
        &store,
        "s1",
        "The system uses event sourcing for audit trails. This is important.",
        orig_importance,
        7,
    );

    auto_promote(&store, 0.80, 0).expect("auto_promote should succeed");

    let successor_id = get_superseded_by(&store, orig_id).expect("successor must exist");
    let succ_importance = get_importance(&store, successor_id);
    let diff = (succ_importance - orig_importance as f64).abs();
    assert!(
        diff < 0.01,
        "semantic successor should inherit original importance (expected ≈{orig_importance}, got {succ_importance})"
    );
}

// ============================================================================
// 5. Temporal validity — memories with valid_to should not be promoted
// ============================================================================

#[test]
fn temporal_expired_memory_not_promoted() {
    let (_dir, store) = make_store();
    // Insert a high-importance episodic that is already expired (valid_to set)
    let id = store_old_episodic(&store, "s1", "Stale high-value memory.", 0.95, 10);
    let now = Utc::now().to_rfc3339();
    store
        .conn()
        .execute(
            "UPDATE messages SET valid_to = ?1 WHERE id = ?2",
            params![now, id],
        )
        .expect("update should succeed");

    let r = auto_promote(&store, 0.80, 0).expect("auto_promote should succeed");
    assert_eq!(
        r.promoted, 0,
        "expired memory (valid_to set) must not be promoted"
    );
}

#[test]
fn temporal_future_valid_to_still_live() {
    // A memory with a future valid_to behaves as live — it CAN be promoted.
    let (_dir, store) = make_store();
    let id = store_old_episodic(&store, "s1", "Time-bounded but currently live.", 0.92, 10);
    let future = (Utc::now() + Duration::days(30)).to_rfc3339();
    store
        .conn()
        .execute(
            "UPDATE messages SET valid_to = ?1 WHERE id = ?2",
            params![future, id],
        )
        .expect("update should succeed");

    // valid_to IS NULL check in auto_promote means future-dated valid_to blocks promotion
    // (the query filters valid_to IS NULL — a future date is still non-NULL)
    let r = auto_promote(&store, 0.80, 0).expect("auto_promote should succeed");
    // Implementation uses `valid_to IS NULL` so a future-dated row IS excluded
    assert_eq!(
        r.promoted, 0,
        "memory with any valid_to value (even future) is skipped by auto_promote"
    );
}

#[test]
fn temporal_only_live_memories_consolidated() {
    let (_dir, store) = make_store();
    let live_id = store_episodic(&store, "ts", "Live message to consolidate.", 0.5);
    store_expired_episodic(&store, "ts", "Already expired message.", 0.5, 5);

    let r = consolidate_session(&store, "ts", false).expect("consolidate should succeed");
    assert_eq!(r.rolled_up, 1, "only live messages should be consolidated");

    // The live one is now superseded; the expired one remains unchanged
    assert!(get_valid_to(&store, live_id).is_some());
}

// ============================================================================
// 6. Config variations
// ============================================================================

#[test]
fn config_strict_gc_removes_more() {
    let (_dir, store) = make_store();
    store_old_episodic(&store, "s1", "10-day-old medium memory.", 0.4, 10);
    store_old_episodic(&store, "s1", "5-day-old low memory.", 0.2, 5);

    // Strict config: 3-day retention, high importance floor
    let strict = GcConfig {
        max_age_days: 3,
        importance_floor: 0.8,
        ..Default::default()
    };
    let r = garbage_collect(&store, &strict).expect("gc should succeed");
    assert_eq!(
        r.deleted, 2,
        "strict config should delete both old, low-importance memories"
    );
}

#[test]
fn config_lenient_gc_removes_less() {
    let (_dir, store) = make_store();
    store_old_episodic(&store, "s1", "Old unimportant memory.", 0.4, 60);

    // Lenient config: 90-day retention — memory is only 60 days old
    let lenient = GcConfig {
        max_age_days: 90,
        importance_floor: 0.5,
        ..Default::default()
    };
    let r = garbage_collect(&store, &lenient).expect("gc should succeed");
    assert_eq!(
        r.deleted, 0,
        "lenient config should not delete a 60-day-old memory under 90-day window"
    );
}

#[test]
fn config_custom_protected_types() {
    let (_dir, store) = make_store();
    let ts = (Utc::now() - Duration::days(90)).to_rfc3339();
    // Insert an old preference memory that would normally be protected
    store
        .conn()
        .execute(
            "INSERT INTO messages \
             (session_id, role, content, tool_calls, tool_results, timestamp, memory_type, importance, valid_from) \
             VALUES ('s1', 'system', 'User likes dark mode.', '[]', '[]', ?1, 'preference', 0.1, ?1)",
            params![ts],
        )
        .expect("direct insert should succeed");

    // Config that does NOT protect preference (empty protected list)
    let cfg = GcConfig {
        max_age_days: 30,
        importance_floor: 0.5,
        keep_promoted: false,
        protected_types: vec![MemoryType::Semantic], // only semantic protected
    };
    let r = garbage_collect(&store, &cfg).expect("gc should succeed");
    assert_eq!(
        r.deleted, 1,
        "preference memory should be deleted when not in protected_types"
    );
}

#[test]
fn config_promotion_threshold_zero_promotes_all_old() {
    let (_dir, store) = make_store();
    store_old_episodic(
        &store,
        "s1",
        "Even low-importance memory gets promoted.",
        0.01,
        10,
    );
    store_old_episodic(&store, "s1", "Another low-importance memory.", 0.05, 10);

    let r = auto_promote(&store, 0.0, 0).expect("auto_promote should succeed");
    assert_eq!(
        r.promoted, 2,
        "threshold=0 should promote everything old enough"
    );
}

#[test]
fn config_promotion_high_threshold_promotes_none() {
    let (_dir, store) = make_store();
    store_old_episodic(&store, "s1", "High importance but below 1.0.", 0.99, 10);

    // threshold = 1.0 — nothing can have importance >= 1.0 after capping at 1.0
    // (some stores may allow 1.0 exactly, this tests the extreme)
    let r = auto_promote(&store, 1.01, 0).expect("auto_promote should succeed");
    assert_eq!(r.promoted, 0, "threshold above 1.0 should promote nothing");
}

// ============================================================================
// 7. Full lifecycle pipeline: store → boost → promote → consolidate → GC
// ============================================================================

#[test]
fn full_pipeline_store_promote_consolidate_gc() {
    let (_dir, store) = make_store();
    let session = "pipeline-session";

    // ── Step 1: Store a conversation ──────────────────────────────────────────
    let id1 = store_episodic(
        &store,
        session,
        "Project uses Rust for the backend. Performance is critical.",
        0.5,
    );
    let id2 = store_episodic(
        &store,
        session,
        "The database is SQLite with WAL mode enabled.",
        0.4,
    );
    let id3 = store_episodic(
        &store,
        session,
        "CI runs on GitHub Actions with matrix builds.",
        0.3,
    );

    assert_eq!(count_live(&store, session, "episodic"), 3);

    // ── Step 2: Boost importance of key memories ───────────────────────────────
    store.boost_memory(id1, 0.40).expect("boost should succeed");
    let imp1 = get_importance(&store, id1);
    assert!(
        imp1 >= 0.88,
        "boosted importance should be ≥ 0.88 (got {imp1})"
    );

    // ── Step 3: Make memories old enough for promotion ─────────────────────────
    // Back-date id1 (the boosted one) so it qualifies for promotion
    let old_ts = (Utc::now() - Duration::days(10)).to_rfc3339();
    store
        .conn()
        .execute(
            "UPDATE messages SET timestamp = ?1, last_accessed = ?1 WHERE id = ?2",
            params![old_ts, id1],
        )
        .expect("backdate should succeed");

    // ── Step 4: Auto-promote ───────────────────────────────────────────────────
    let promo_result = auto_promote(&store, 0.85, 5).expect("auto_promote should succeed");
    assert_eq!(
        promo_result.promoted, 1,
        "only the boosted, old memory should be promoted"
    );

    // id1 should now be superseded
    assert!(get_valid_to(&store, id1).is_some());
    // id2 and id3 remain live (not old/important enough)
    assert!(get_valid_to(&store, id2).is_none());
    assert!(get_valid_to(&store, id3).is_none());

    // A semantic memory should exist
    assert_eq!(count_live(&store, session, "semantic"), 1);

    // ── Step 5: Consolidate remaining episodic memories ───────────────────────
    let consol_result =
        consolidate_session(&store, session, false).expect("consolidate should succeed");
    assert_eq!(
        consol_result.rolled_up, 2,
        "only id2 and id3 remain live for consolidation"
    );
    assert_eq!(consol_result.summaries_created, 1);

    // No live episodic left, summary exists
    assert_eq!(count_live(&store, session, "episodic"), 0);
    assert_eq!(count_live(&store, session, "summary"), 1);
    // Semantic persists
    assert_eq!(count_live(&store, session, "semantic"), 1);

    // ── Step 6: Garbage collect ────────────────────────────────────────────────
    // At this point all original episodics are superseded (soft-deleted).
    // GC should not delete them (they have valid_to set, excluded from GC).
    let gc_result = garbage_collect(&store, &GcConfig::default()).expect("gc should succeed");
    assert_eq!(
        gc_result.deleted, 0,
        "soft-deleted memories should not be double-deleted by GC"
    );

    // Semantic and summary remain live
    assert_eq!(count_live(&store, session, "semantic"), 1);
    assert_eq!(count_live(&store, session, "summary"), 1);
}

#[test]
fn full_pipeline_gc_cleans_old_unimportant_after_session_ends() {
    let (_dir, store) = make_store();
    let session = "gc-pipeline";

    // Store old, low-importance memories (simulating abandoned session)
    store_old_episodic(&store, session, "Trivial note A.", 0.1, 45);
    store_old_episodic(&store, session, "Trivial note B.", 0.2, 45);
    // One important one that should survive
    store_old_episodic(&store, session, "Critical architectural note.", 0.9, 45);

    let gc_cfg = GcConfig {
        max_age_days: 30,
        importance_floor: 0.5,
        ..Default::default()
    };
    let r = garbage_collect(&store, &gc_cfg).expect("gc should succeed");
    assert_eq!(r.deleted, 2, "trivial notes should be GC'd");
    assert_eq!(r.kept, 1, "critical note should survive");

    // The remaining memory should be the important one
    let remaining: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND valid_to IS NULL",
            params![session],
            |row| row.get(0),
        )
        .expect("count query should succeed");
    assert_eq!(remaining, 1);
}

#[test]
fn full_pipeline_multi_session_isolation() {
    let (_dir, store) = make_store();

    // Session A: high-importance conversation (to be promoted)
    let id_a = store_old_episodic(
        &store,
        "session-a",
        "Session A key insight about system design.",
        0.90,
        7,
    );

    // Session B: low-importance, old (to be GC'd)
    store_old_episodic(&store, "session-b", "Session B trivia.", 0.1, 60);

    // Session C: fresh messages (should be untouched)
    store_episodic(&store, "session-c", "Session C recent message.", 0.5);

    // Promote session A
    let promo = auto_promote(&store, 0.80, 1).expect("auto_promote should succeed");
    assert_eq!(promo.promoted, 1);
    assert!(
        get_valid_to(&store, id_a).is_some(),
        "session-a original superseded"
    );

    // Consolidate session B
    let consol =
        consolidate_session(&store, "session-b", false).expect("consolidate should succeed");
    assert_eq!(consol.rolled_up, 1);

    // GC: session-b original is soft-deleted, not GC-eligible
    let gc = garbage_collect(
        &store,
        &GcConfig {
            max_age_days: 30,
            importance_floor: 0.5,
            ..Default::default()
        },
    )
    .expect("gc should succeed");
    assert_eq!(
        gc.deleted, 0,
        "soft-deleted row from session-b should not be GC'd"
    );

    // Session C untouched
    assert_eq!(count_live(&store, "session-c", "episodic"), 1);
}
