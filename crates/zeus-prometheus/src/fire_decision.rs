//! Busy-aware heartbeat fire-decision pure logic (Lane A1.5b-i.α).
//!
//! Pure-function library API for the 4-bucket disjunction that decides whether
//! a heartbeat tick should fire, skip, or otherwise yield. Atomic-loads happen
//! at the call-site (in `Heartbeat::run`); this module operates on primitive
//! inputs to keep the decision logic trivially testable without
//! `Heartbeat::new()` scaffolding (see SOUL.md banked discipline:
//! "free-function-with-injectable-seam-collapses-test-scaffolding-cost").
//!
//! # Design
//!
//! Four orthogonal buckets, each emitting a distinct skip-reason for structured
//! observability (per spec §3.2 and SOUL.md banked
//! "distinct-signal-orthogonality-must-surface-in-observability-shape"):
//!
//! 1. **CookInFlight** — this-cook handler currently active (`channel_active`)
//! 2. **InboundPending** — queued external messages waiting (`inbox_depth > 0`)
//! 3. **SubagentActive** — subagent cooks in flight (`subagent_depth > 0`)
//! 4. **RecentInteraction** — user-interaction recency below quiet threshold
//!
//! Wiring at the tick-site is the A1.5b-i.β follow-up; this module lands as
//! pure-additive substrate (Lane A1.5b-i.α) per banked
//! "recursive-substrate-plumbing-before-semantic-rewrite" discipline.

/// Outcome of a fire-decision evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FireDecision {
    /// All four buckets clear — heartbeat tick should proceed.
    Fire,
    /// At least one bucket triggered a skip; reason identifies which.
    Skip { reason: SkipReason },
}

/// The four distinct skip reasons (4-bucket disjunction).
///
/// Each variant maps 1:1 to a bucket-cause for structured trace events
/// (`heartbeat_skipped{reason: "..."}`); no variant collapses two buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// `channel_active = true` — a cook is currently in flight on this channel.
    CookInFlight,
    /// `inbox_depth > 0` — one or more queued external messages waiting.
    InboundPending,
    /// `subagent_depth > 0` — one or more subagent cooks in flight.
    SubagentActive,
    /// `(now - last_interaction_at) < threshold_secs` — user is currently
    /// active, suppressing autonomous fire to avoid talking-over.
    RecentInteraction,
}

impl SkipReason {
    /// String label for structured trace `reason` field.
    ///
    /// Stable across versions; intended as a dashboard/alert key.
    pub const fn as_str(self) -> &'static str {
        match self {
            SkipReason::CookInFlight => "cook_in_flight",
            SkipReason::InboundPending => "inbound_pending",
            SkipReason::SubagentActive => "subagent_active",
            SkipReason::RecentInteraction => "recent_interaction",
        }
    }
}

/// Evaluate the busy-aware fire-decision against primitive inputs.
///
/// Pure function — no side effects, no atomic-loads, no time source. The
/// caller (heartbeat tick at `heartbeat.rs`) reads atomic counters at the
/// call-site and passes the loaded values plus the current time.
///
/// # Bucket evaluation order
///
/// Buckets are evaluated in declaration order (Cook → Inbound → Subagent →
/// RecentInteraction); first match wins. The order is observability-driven:
/// `CookInFlight` is the most specific (handler-active), `RecentInteraction`
/// is the broadest (quiet-window). A skip emits the *first-matching* reason,
/// which gives ops a deterministic attribution even when multiple buckets
/// would have triggered.
///
/// # None-handle graceful-degrade
///
/// `last_interaction_at` is `Option<i64>`: `None` indicates the tick-site
/// has no time-anchor wired (A1.5b-ii follow-up will add the cook-completion
/// write-site). When `None`, the `RecentInteraction` bucket is **inactive
/// (graceful-degrade)** — never fires, never skips. This matches the
/// Option-shape graceful-degrade pattern banked from A1.5a (`Option<Arc<...>>`
/// counters): wired-but-inactive on None, not dead-code.
///
/// # Arguments
///
/// * `channel_active` — `true` if a cook handler is currently in flight
/// * `inbox_depth` — queued external messages count (atomic-load at call-site)
/// * `subagent_depth` — subagent cooks in flight count (atomic-load)
/// * `last_interaction_at` — `Some(unix_secs)` of last user interaction, or
///   `None` if no time-anchor wired (graceful-degrade: bucket inactive)
/// * `now_unix_secs` — current time as unix seconds
/// * `quiet_threshold_secs` — RecentInteraction bucket threshold; `(now - last)
///   < threshold` triggers the skip
pub fn should_fire_heartbeat(
    channel_active: bool,
    inbox_depth: usize,
    subagent_depth: usize,
    last_interaction_at: Option<i64>,
    now_unix_secs: i64,
    quiet_threshold_secs: i64,
) -> FireDecision {
    if channel_active {
        return FireDecision::Skip {
            reason: SkipReason::CookInFlight,
        };
    }
    if inbox_depth > 0 {
        return FireDecision::Skip {
            reason: SkipReason::InboundPending,
        };
    }
    if subagent_depth > 0 {
        return FireDecision::Skip {
            reason: SkipReason::SubagentActive,
        };
    }
    if let Some(last_at) = last_interaction_at {
        if now_unix_secs.saturating_sub(last_at) < quiet_threshold_secs {
            return FireDecision::Skip {
                reason: SkipReason::RecentInteraction,
            };
        }
    }
    FireDecision::Fire
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §3.5 test 1: all buckets clear → Fire.
    #[test]
    fn all_clear_fires() {
        let decision = should_fire_heartbeat(false, 0, 0, Some(0), 1000, 60);
        assert_eq!(decision, FireDecision::Fire);
    }

    /// §3.5 test 2: channel_active → Skip{CookInFlight}.
    #[test]
    fn cook_in_flight_skips() {
        let decision = should_fire_heartbeat(true, 0, 0, Some(0), 1000, 60);
        assert_eq!(
            decision,
            FireDecision::Skip {
                reason: SkipReason::CookInFlight
            }
        );
    }

    /// §3.5 test 3: inbox_depth > 0 → Skip{InboundPending}.
    #[test]
    fn inbound_pending_skips() {
        let decision = should_fire_heartbeat(false, 1, 0, Some(0), 1000, 60);
        assert_eq!(
            decision,
            FireDecision::Skip {
                reason: SkipReason::InboundPending
            }
        );
    }

    /// §3.5 test 4: subagent_depth > 0 → Skip{SubagentActive}.
    #[test]
    fn subagent_active_skips() {
        let decision = should_fire_heartbeat(false, 0, 1, Some(0), 1000, 60);
        assert_eq!(
            decision,
            FireDecision::Skip {
                reason: SkipReason::SubagentActive
            }
        );
    }

    /// §3.5 test 5: recent interaction within threshold → Skip{RecentInteraction}.
    #[test]
    fn recent_interaction_skips() {
        // last_at=970, now=1000, threshold=60 → (1000-970)=30 < 60 → skip
        let decision = should_fire_heartbeat(false, 0, 0, Some(970), 1000, 60);
        assert_eq!(
            decision,
            FireDecision::Skip {
                reason: SkipReason::RecentInteraction
            }
        );
    }

    /// None-handle graceful-degrade test: `last_interaction_at = None` →
    /// RecentInteraction bucket inactive; with all other buckets clear, fires.
    /// Mirrors A1.5a Option-shape graceful-degrade banked discipline.
    #[test]
    fn none_handle_graceful_degrade_fires() {
        let decision = should_fire_heartbeat(false, 0, 0, None, 1000, 60);
        assert_eq!(decision, FireDecision::Fire);
    }

    /// Bucket-priority test: Cook precedes Inbound (first-matching-wins).
    /// Documents the observability-driven evaluation order.
    #[test]
    fn cook_precedes_inbound() {
        let decision = should_fire_heartbeat(true, 5, 5, Some(990), 1000, 60);
        assert_eq!(
            decision,
            FireDecision::Skip {
                reason: SkipReason::CookInFlight
            }
        );
    }

    /// Bucket-priority test: Inbound precedes Subagent.
    #[test]
    fn inbound_precedes_subagent() {
        let decision = should_fire_heartbeat(false, 1, 1, Some(990), 1000, 60);
        assert_eq!(
            decision,
            FireDecision::Skip {
                reason: SkipReason::InboundPending
            }
        );
    }

    /// SkipReason::as_str maps each variant to a stable dashboard key.
    #[test]
    fn skip_reason_str_labels_stable() {
        assert_eq!(SkipReason::CookInFlight.as_str(), "cook_in_flight");
        assert_eq!(SkipReason::InboundPending.as_str(), "inbound_pending");
        assert_eq!(SkipReason::SubagentActive.as_str(), "subagent_active");
        assert_eq!(SkipReason::RecentInteraction.as_str(), "recent_interaction");
    }
}
