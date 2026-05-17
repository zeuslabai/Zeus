//! Fleet session resolver — Lane 3a signature lock (commit c).
//!
//! Authoritative signature for `Prometheus::session_resolver`. Body is
//! `unimplemented!()` in this commit; Lane 3b lands the lookup logic, Lane 3c
//! prepends the resolver call to the 4 gateway.rs callsites and the 1
//! `process_autonomous` callsite.
//!
//! See `docs/sprints/backlog-prepare-cook-context-extraction-2026-05-06.md`
//! §3.1–§3.2 for the full design rationale.
//!
//! # Lock semantics
//!
//! Callers MUST hold the existing `RwLockReadGuard<'_, SessionManager>`
//! (acquired via `self.sessions.read().await`) and pass it borrowed.
//! The resolver MUST NOT acquire `self.sessions.write()` — that deadlocks
//! against the existing read at `lib.rs:428` — and MUST NOT re-acquire
//! `self.sessions.read()` — single-writer starvation risk under contention.
//!
//! # Error shape
//!
//! Infallible. Returns [`FleetSessionAlias::unaliased`] when no fleet alias
//! resolves for the (agent, human, channel) key, localizing failure to the
//! resolver and keeping the 5 callsites at zero error-handling diff.
//!
//! # Async-ness
//!
//! `async fn` — v1 body in Lane 3b may be a pure in-mem `&sessions` lookup,
//! but Lane 3b also folds in a Mnemosyne lookup which awaits. Locking the
//! `async` shape here avoids a breaking signature change in 3b.

use chrono::{DateTime, Utc};
use tokio::sync::RwLockReadGuard;

use zeus_mnemosyne::Mnemosyne;

use crate::channels::ChannelKind;
use crate::session::SessionManager;
use crate::Prometheus;

/// Newtype wrapping the resolved fleet session alias.
///
/// Prevents `String` confusion at call sites and allows additive metadata
/// fields (e.g. `merge_decision: Merged | Fresh | Aliased`) to land in
/// Lane 3b without breaking Lane 3c callsites.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FleetSessionAlias(String);

impl FleetSessionAlias {
    /// Fallback constructor — resolver returns this when no fleet alias
    /// resolves for a given key.
    ///
    /// Yields `unaliased:<original_session_id>` so downstream `Display`
    /// and `AsRef<str>` consumers see a stable, debuggable string with
    /// no behavioral change vs the pre-resolver world.
    pub fn unaliased(original: &str) -> Self {
        Self(format!("unaliased:{}", original))
    }

    /// Construct a resolved alias directly. Reserved for Lane 3b's
    /// resolver body and tests; callers outside this crate should
    /// route through `Prometheus::session_resolver`.
    #[allow(dead_code)] // Used by Lane 3b body + tests; suppress warning until 3b lands.
    pub(crate) fn resolved(alias: impl Into<String>) -> Self {
        Self(alias.into())
    }

    /// Borrow the underlying alias string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Return how this alias was resolved for telemetry. Lane 2b-i.
    pub fn resolved_via(&self) -> &'static str {
        if self.0.starts_with("unaliased:") {
            "fallback_unaliased"
        } else {
            "channel_alias"
        }
    }
}

impl std::fmt::Display for FleetSessionAlias {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for FleetSessionAlias {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Prometheus {
    /// Resolve a fleet session alias for the given (agent, human, channel) key.
    ///
    /// **Lane 3a:** signature locked, body is `unimplemented!()`.
    /// **Lane 3b:** body lands (in-mem `sessions` lookup + Mnemosyne fallback).
    /// **Lane 3c:** call sites in `gateway.rs` (lines 1113, 2014, 2270, 2709)
    /// and `process_autonomous` are wired to invoke this method.
    /// **Lane 2b-fixture (γ-fn-extract):** body delegates to free function
    /// [`resolve_alias_via_cache`] so the cross-channel falsifier can exercise
    /// the lookup/upsert path without constructing a full `Prometheus`.
    ///
    /// See module-level docs for lock semantics and error shape.
    pub async fn session_resolver<'g>(
        &self,
        sessions: &RwLockReadGuard<'g, SessionManager>,
        agent_id: &str,
        human_id: Option<&str>,
        channel_kind: ChannelKind,
        now: DateTime<Utc>,
    ) -> FleetSessionAlias {
        let Some(human_id) = human_id else {
            return FleetSessionAlias::unaliased(agent_id);
        };

        let Some(mnemosyne) = self.mnemosyne.as_ref() else {
            return FleetSessionAlias::unaliased(agent_id);
        };

        resolve_alias_via_cache(
            mnemosyne.as_ref(),
            sessions,
            agent_id,
            human_id,
            channel_kind,
            now,
        )
        .await
    }
}

/// Resolve a fleet session alias against the Mnemosyne cache, performing
/// the lookup/upsert dance described in the algorithm below.
///
/// This is the testable seam extracted from [`Prometheus::session_resolver`]
/// per Lane 2b-fixture (γ-fn-extract). The method retains the two None-guards
/// (human_id, mnemosyne) so the locked signature semantics are preserved at
/// the 5 callsites; this free function carries everything below the guards.
///
/// # Algorithm
///
///   1. Lookup `(agent_id, human_id)` against the `fleet_session_alias` cache,
///      filtered by a 24-hour rolling window (`last_seen >= now - 24h`).
///   2. Hit → [`FleetSessionAlias::resolved`] with the cached `session_id`.
///   3. Miss-or-stale-or-error → upsert the current active session against
///      the `(agent, human)` key so the next cook within the window
///      correlates, then return [`FleetSessionAlias::unaliased`].
///
/// Errors from Mnemosyne are swallowed by design — the resolver is infallible
/// per Lane 3a's locked error shape. A failed cache lookup degrades to "no
/// correlation," not to a panic.
pub async fn resolve_alias_via_cache<'g>(
    mnemosyne: &Mnemosyne,
    sessions: &RwLockReadGuard<'g, SessionManager>,
    agent_id: &str,
    human_id: &str,
    channel_kind: ChannelKind,
    now: DateTime<Utc>,
) -> FleetSessionAlias {
    let since = (now - chrono::Duration::hours(24)).to_rfc3339();

    match mnemosyne.lookup_alias(agent_id, human_id, &since).await {
        Ok(Some(row)) => FleetSessionAlias::resolved(row.session_id),
        Ok(None) | Err(_) => {
            // Upsert current active session on miss so the next cook
            // within the 24-hour window correlates.
            if let Some(current) = sessions.current() {
                let now_rfc = now.to_rfc3339();
                let _ = mnemosyne
                    .upsert_alias(
                        agent_id,
                        human_id,
                        &current.id,
                        channel_kind.as_canonical(),
                        &now_rfc,
                    )
                    .await;
            }
            FleetSessionAlias::unaliased(agent_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unaliased_yields_stable_debuggable_string() {
        let alias = FleetSessionAlias::unaliased("session-abc-123");
        assert_eq!(alias.as_str(), "unaliased:session-abc-123");
        assert_eq!(alias.to_string(), "unaliased:session-abc-123");
    }

    #[test]
    fn unaliased_preserves_empty_original() {
        // Defensive: empty original_session_id should still produce a
        // grep-friendly prefix, not silently elide.
        let alias = FleetSessionAlias::unaliased("");
        assert_eq!(alias.as_str(), "unaliased:");
    }

    #[test]
    fn resolved_constructor_skips_unaliased_prefix() {
        // Lane 3b's resolver body uses `resolved(...)` to bank a real
        // alias; verify it doesn't accidentally double-prefix.
        let alias = FleetSessionAlias::resolved("fleet-xyz-789");
        assert_eq!(alias.as_str(), "fleet-xyz-789");
        assert!(!alias.as_str().starts_with("unaliased:"));
    }

    #[test]
    fn as_ref_str_matches_as_str() {
        let alias = FleetSessionAlias::unaliased("k");
        let via_as_ref: &str = alias.as_ref();
        assert_eq!(via_as_ref, alias.as_str());
    }

    #[test]
    fn equality_and_hash_distinguish_unaliased_from_resolved() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(FleetSessionAlias::unaliased("x"));
        set.insert(FleetSessionAlias::resolved("unaliased:x"));
        // Both strings are literally equal ("unaliased:x"), so the set
        // collapses to one entry — confirms equality is by string content,
        // not by construction path. Lane 3b should not rely on
        // unaliased-vs-resolved distinction at the type level; if that
        // distinction matters, add an explicit enum variant.
        assert_eq!(set.len(), 1);
    }

    // ----- resolved_via() telemetry tag — Lane 2b-i coverage ----------------
    //
    // `resolved_via()` is the telemetry-tag accessor consumed by the
    // `fleet_session_correlation` event payload (gateway.rs callsites at
    // 1136/2063/2357/2828). It must distinguish the fallback-unaliased path
    // from the channel-alias-resolved path so dashboards can split fleet
    // session correlation rate by resolution mode. These tests pin the two
    // tag strings as part of the load-bearing telemetry contract.

    #[test]
    fn resolved_via_returns_fallback_unaliased_for_unaliased_constructor() {
        let alias = FleetSessionAlias::unaliased("session-abc-123");
        assert_eq!(alias.resolved_via(), "fallback_unaliased");
    }

    #[test]
    fn resolved_via_returns_channel_alias_for_resolved_constructor() {
        let alias = FleetSessionAlias::resolved("fleet-xyz-789");
        assert_eq!(alias.resolved_via(), "channel_alias");
    }

    #[test]
    fn resolved_via_dispatches_on_string_content_not_construction_path() {
        // Confirms `resolved_via()` is a pure function of the underlying
        // string content (`starts_with("unaliased:")`), not of which
        // constructor was called. A `resolved(...)` alias whose string
        // happens to start with `"unaliased:"` is reported as the fallback
        // path — this is the documented invariant matching the
        // equality-and-hash test above. Dashboards keying on
        // `resolved_via` therefore see the same partition as the string
        // representation, never a construction-path artifact.
        let pseudo_unaliased = FleetSessionAlias::resolved("unaliased:x");
        assert_eq!(pseudo_unaliased.resolved_via(), "fallback_unaliased");
    }

    #[test]
    fn resolved_via_returns_static_str_with_stable_tag_set() {
        // The tag set is part of the telemetry schema — only two values
        // are valid, and dashboards/alerts pivot on string equality.
        // Pinning the closed set guards against silent tag drift in
        // future refactors of `resolved_via()` (e.g., if a new variant
        // is added, this test forces the schema discussion explicitly).
        let unaliased_tag = FleetSessionAlias::unaliased("k").resolved_via();
        let resolved_tag = FleetSessionAlias::resolved("alias-1").resolved_via();
        assert!(matches!(unaliased_tag, "fallback_unaliased" | "channel_alias"));
        assert!(matches!(resolved_tag, "fallback_unaliased" | "channel_alias"));
        assert_ne!(unaliased_tag, resolved_tag);
    }
}
