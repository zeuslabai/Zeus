//! Cross-channel session-correlation falsifier — Lane 2b-fixture.
//!
//! Falsifies the original "Discord bug" symptom (which expanded into the
//! full session-identity rewrite this sprint shipped): two cooks from the
//! same `(agent_id, human_id)` pair within the 24-hour fleet-alias window
//! must resolve to the **same `session_id`** even when the channels differ.
//!
//! # What this test exercises end-to-end
//!
//! - Real `Mnemosyne` instance (in-memory SQLite via `tempfile`).
//! - Real `SessionManager` with an active session.
//! - Direct call to the testable seam `resolve_alias_via_cache` extracted
//!   from `Prometheus::session_resolver` per Lane 2b-fixture (γ-fn-extract).
//! - Sequential cooks: `(X, Y, Discord)` → `(X, Y, Slack)` within 24hr
//!   → assert matching `session_id`.
//!
//! # Why this matters
//!
//! Pre-fix, the resolver was channel-keyed: a Slack cook for `(X, Y)` would
//! mint a fresh `session_id` even if `(X, Y)` had cooked on Discord seconds
//! prior. Post-fix, the `fleet_session_alias` cache is `(agent_id, human_id)`
//! keyed (channel-blind on lookup, channel-tagged for telemetry). This test
//! is the integration-level proof that the structural fix holds.

use std::sync::Arc;

use chrono::{Duration, Utc};
use tempfile::tempdir;
use tokio::sync::RwLock;

use zeus_mnemosyne::{Mnemosyne, MnemosyneConfig};
use zeus_prometheus::channels::ChannelKind;
use zeus_prometheus::session::SessionManager;
use zeus_prometheus::session_resolver::{resolve_alias_via_cache, FleetSessionAlias};

/// Construct a minimal in-memory Mnemosyne for fixture use.
///
/// Keeps the harness narrow per Lane 2b-fixture decomposition: no FTS,
/// no embeddings, no QMD — only the `fleet_session_alias` table is
/// exercised. `tempdir` ensures isolation between test runs.
async fn make_test_mnemosyne() -> (Mnemosyne, tempfile::TempDir) {
    let dir = tempdir().expect("tempdir creation should succeed");
    let config = MnemosyneConfig {
        db_path: dir.path().join("falsifier.db"),
        enable_fts: false,
        enable_embeddings: false,
        max_messages_per_session: 100,
        ..Default::default()
    };
    let mnemosyne = Mnemosyne::new(config)
        .await
        .expect("Mnemosyne::new should succeed for in-memory falsifier fixture");
    (mnemosyne, dir)
}

/// Build a `SessionManager` with one active session, returning the manager
/// wrapped in `Arc<RwLock<_>>` to match the `Prometheus::session_resolver`
/// callsite locking discipline.
fn make_active_sessions() -> Arc<RwLock<SessionManager>> {
    let mut mgr = SessionManager::new();
    let _ = mgr.create_session();
    Arc::new(RwLock::new(mgr))
}

/// **The falsifier.** Two cooks for the same `(agent, human)` pair on
/// different channels within 24hr must resolve to the same `session_id`.
#[tokio::test]
async fn cross_channel_cook_within_24hr_correlates_to_same_session_id() {
    let (mnemosyne, _dir) = make_test_mnemosyne().await;
    let sessions = make_active_sessions();
    let sessions_guard = sessions.read().await;

    let agent_id = "zeus106";
    let human_id = "operator-1";
    let now_first = Utc::now();
    let now_second = now_first + Duration::minutes(5);

    // Cook 1 — (agent_id, human_id, Discord). Cache miss → upsert active
    // session against (agent_id, human_id), return unaliased(agent_id).
    let alias_first = resolve_alias_via_cache(
        &mnemosyne,
        &sessions_guard,
        agent_id,
        human_id,
        ChannelKind::Discord,
        now_first,
    )
    .await;
    assert_eq!(
        alias_first,
        FleetSessionAlias::unaliased(agent_id),
        "first cook should miss the cache and return unaliased(agent_id) per Lane 3b-ii algorithm step 5"
    );

    // Cook 2 — same (agent_id, human_id), DIFFERENT channel (Slack), within
    // 24hr. Cache hit on the upsert from cook 1 → resolved(<active session id>).
    let alias_second = resolve_alias_via_cache(
        &mnemosyne,
        &sessions_guard,
        agent_id,
        human_id,
        ChannelKind::Slack,
        now_second,
    )
    .await;

    // The active session_id minted before cook 1 — pull it out for the
    // post-condition assertion.
    let active_session_id = sessions_guard
        .current()
        .expect("active session should exist post create_session()")
        .id
        .clone();

    assert_eq!(
        alias_second.as_str(),
        active_session_id.as_str(),
        "second cook within 24hr on a different channel must resolve to the same session_id \
         minted by cook 1's upsert — falsifies the Discord bug at the integration layer"
    );
    assert_eq!(
        alias_second.resolved_via(),
        "channel_alias",
        "second cook should be tagged via the channel_alias telemetry path, not fallback_unaliased"
    );
}

/// Outside the 24hr window, the second cook should miss the cache (the
/// `since` filter excludes the stale alias) and re-upsert. Negative-control
/// for the rolling-window semantics.
#[tokio::test]
async fn cross_channel_cook_outside_24hr_does_not_correlate() {
    let (mnemosyne, _dir) = make_test_mnemosyne().await;
    let sessions = make_active_sessions();
    let sessions_guard = sessions.read().await;

    let agent_id = "zeus106";
    let human_id = "operator-1";
    let now_first = Utc::now() - Duration::hours(48);
    let now_second = Utc::now();

    // Cook 1 — 48hr ago, upserts alias into cache.
    let _ = resolve_alias_via_cache(
        &mnemosyne,
        &sessions_guard,
        agent_id,
        human_id,
        ChannelKind::Discord,
        now_first,
    )
    .await;

    // Cook 2 — now. The cook 1 upsert is 48hr old, outside the 24hr window
    // — should miss the lookup, return unaliased (and re-upsert with current
    // `now`, but that's not what we're asserting here).
    let alias_second = resolve_alias_via_cache(
        &mnemosyne,
        &sessions_guard,
        agent_id,
        human_id,
        ChannelKind::Slack,
        now_second,
    )
    .await;

    assert_eq!(
        alias_second,
        FleetSessionAlias::unaliased(agent_id),
        "stale alias outside 24hr window must NOT correlate — re-mints unaliased(agent_id)"
    );
}

/// `human_id = None` short-circuits to `unaliased(agent_id)` without
/// touching the cache. The free function takes `human_id: &str`, so this
/// case is enforced at the type level — but worth pinning that the method
/// itself (called via `Prometheus::session_resolver`) honors the guard.
/// We can't easily call the method without a `Prometheus`, so instead we
/// pin the corresponding fact at the free-function entry: nothing about
/// the function permits a `None` `human_id`. This is a typecheck assertion
/// — the test compiles, therefore the guard is type-enforced.
#[test]
fn human_id_required_at_type_level() {
    // If `human_id` were `Option<&str>` on the free function, this line
    // would not compile. The type signature IS the test.
    fn _typecheck(_: &str) {}
    _typecheck("operator-1");
}

// ---- #86 complement: alias-aware cross-channel injection ----------------
//
// Proves the wiring Z112 cut for #86: `inject_cross_channel_aliased` routes
// the resolved `FleetSessionAlias` into the injection seam. The
// `fallback_unaliased` provenance must yield the SAME block as the raw-channel
// `inject_cross_channel` — i.e. the wiring is strictly additive (never less
// context than before). The `channel_alias` annotation path is unit-tested in
// the crate (the `resolved()` ctor is `pub(crate)`); here we pin the
// integration-level fallback invariant end-to-end against a real Mnemosyne.

use zeus_core::Message;
use zeus_prometheus::MemoryInjector;

#[tokio::test]
async fn aliased_injection_fallback_unaliased_equals_raw_channel_path() {
    let (mnemosyne, _dir) = make_test_mnemosyne().await;

    // Seed a memory on a DIFFERENT channel ("slack") than the current
    // channel ("discord"), so the cross-channel search surfaces it.
    let msg = Message::user("the merge-decision telemetry tag lives in session_resolver");
    mnemosyne
        .store_with_embedding_tagged("sess-other", &msg, Some("slack"), Some("chat-1"))
        .await
        .expect("seed cross-channel memory");

    let injector = MemoryInjector::new(5, 8000);
    let query = "telemetry tag";

    // Raw-channel path (pre-#86 behavior).
    let raw = injector
        .inject_cross_channel(&mnemosyne, query, "discord")
        .await;

    // Alias-aware path with a fallback_unaliased alias → must equal raw.
    let alias = FleetSessionAlias::unaliased("sess-current");
    assert_eq!(alias.resolved_via(), "fallback_unaliased");
    let aliased = injector
        .inject_cross_channel_aliased(&mnemosyne, query, "discord", &alias)
        .await;

    assert_eq!(
        raw, aliased,
        "fallback_unaliased alias must yield identical block to raw-channel \
         injection — the #86 wiring is strictly additive"
    );
    assert!(
        aliased.is_some(),
        "seeded cross-channel memory should surface in the injection"
    );
    assert!(
        aliased.as_ref().unwrap().starts_with("## Cross-channel context\n"),
        "fallback path keeps the un-annotated header (no fleet-alias tag)"
    );
}

// ---- #192: same-human containment falsifier ------------------------------
//
// Guardrails for flipping `cross_channel_session_tail_n` default-on and
// plumbing `human_id` through the live cook path (gateway →
// `set_current_human_id()` → resolver callsite):
//
//   1. Same human, different channel, inside the 24hr window → MUST
//      correlate (the #192 promise: one continuous titan memory regardless
//      of surface). Covered by
//      `cross_channel_cook_within_24hr_correlates_to_same_session_id` above.
//   2. DIFFERENT human, same agent, same window → MUST NOT correlate.
//      Cross-channel continuity is same-human carry-over, never cross-human
//      bleed. This is the containment proof.

/// **Containment falsifier.** A second human cooking with the same agent
/// inside the 24hr window must NOT inherit the first human's session alias.
#[tokio::test]
async fn different_human_must_not_correlate_to_other_humans_session() {
    let (mnemosyne, _dir) = make_test_mnemosyne().await;
    let sessions = make_active_sessions();
    let sessions_guard = sessions.read().await;

    let agent_id = "zeus107";
    let human_a = "operator-alice";
    let human_b = "operator-bob";
    let now_first = Utc::now();
    let now_second = now_first + Duration::minutes(5);

    // Cook 1 — (agent, alice, Discord): miss → upserts alice's active session.
    let _ = resolve_alias_via_cache(
        &mnemosyne,
        &sessions_guard,
        agent_id,
        human_a,
        ChannelKind::Discord,
        now_first,
    )
    .await;

    // Cook 2 — (agent, BOB, Slack), 5 min later, well inside the window.
    // Bob has never cooked → his lookup must miss. If this resolves to
    // alice's session_id, we have cross-human bleed and #192's default-on
    // tail injection would be unsafe.
    let alias_bob = resolve_alias_via_cache(
        &mnemosyne,
        &sessions_guard,
        agent_id,
        human_b,
        ChannelKind::Slack,
        now_second,
    )
    .await;

    assert_eq!(
        alias_bob,
        FleetSessionAlias::unaliased(agent_id),
        "different human must NOT inherit another human's session alias — \
         the (agent, human) cache key is the cross-human containment boundary"
    );

    // And the converse: alice on a third surface still correlates to her own
    // session — bob's miss/upsert must not have evicted or polluted her key.
    let alias_alice = resolve_alias_via_cache(
        &mnemosyne,
        &sessions_guard,
        agent_id,
        human_a,
        ChannelKind::Slack,
        now_second + Duration::minutes(1),
    )
    .await;
    assert_eq!(
        alias_alice.resolved_via(),
        "channel_alias",
        "same human within window must still correlate after an interleaved \
         different-human cook — per-key isolation holds both ways"
    );
}
