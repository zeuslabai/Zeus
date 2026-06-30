//! Shared cook state: `ActiveCookType` + `CookGuard`.
//!
//! Replaces the old `channel_message_active: AtomicBool` with a richer
//! lock-free state machine that tracks *which kind* of cook is running,
//! so both the channel consumer and the heartbeat can defer to each
//! other instead of preempting.
//!
//! Design notes (per fleet review — Zeus100, Zeus112, zeusmolty):
//! - `#[repr(u8)]` enum over `AtomicU8` for lock-free access.
//! - `CookGuard` is RAII — panic-safe by construction.
//! - `CookGuard::drop` uses `compare_exchange(expected, None)` so it
//!   never clobbers a higher-priority cook that swapped in between
//!   construction and drop (Zeus112's note).
//! - Self-deferral falls out naturally from the CAS-gated `try_acquire`:
//!   if a heartbeat tick fires while one is already running, the CAS
//!   fails and the tick is skipped (Zeus100's note).

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::time::Instant;

/// Which kind of cook is currently active.
///
/// Repr as u8 so we can stuff it in `AtomicU8`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveCookType {
    /// No cook in flight.
    None = 0,
    /// Human-initiated cook (Discord/Telegram/Slack message).
    Channel = 1,
    /// Autonomy-driven cook (heartbeat tick cooking CURRENT TASK).
    Heartbeat = 2,
}

impl ActiveCookType {
    #[inline]
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Channel,
            2 => Self::Heartbeat,
            _ => Self::None,
        }
    }

    #[inline]
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Shared cook-state handle. Clone freely — it's just an `Arc`.
///
/// # Two modes
/// - **Single-slot CAS** (`try_acquire`/`load`) — the original fleet-wide
///   one-cook-at-a-time gate. Retained for the legacy serial `run()` path
///   and back-compat; superseded for laned cooks by per-session lanes
///   (`SessionLaneManager`).
/// - **In-flight counters** (`begin_cook`/`is_channel_cooking`) — #192b P2.
///   Under per-session dispatch, multiple cooks run concurrently, so the
///   single slot can no longer answer "is a user/channel cook in flight?"
///   The counters track that: the dispatcher `begin_cook`s on spawn and the
///   returned `CookFlight` decrements on drop (RAII, panic-safe). The
///   heartbeat reads `is_channel_cooking()` to defer (Bug-A preserved).
#[derive(Debug, Clone, Default)]
pub struct CookState {
    inner: Arc<AtomicU8>,
    /// When the current cook was acquired. Set by `try_acquire`, cleared on
    /// guard drop or `force_clear`. Used by the heartbeat to detect leaked
    /// guards and reclaim the slot.
    active_since: Arc<Mutex<Option<Instant>>>,
    /// #192b P2: count of in-flight **channel** (human-initiated) cooks.
    /// Incremented by `begin_cook(Channel)` on dispatch, decremented when
    /// the returned `CookFlight` drops. Lets the heartbeat answer
    /// "is a user cook running?" under concurrent per-session dispatch,
    /// where the single-slot CAS cannot.
    channel_inflight: Arc<AtomicU32>,
    /// #192b P2: count of in-flight **heartbeat** (autonomy) cooks.
    heartbeat_inflight: Arc<AtomicU32>,
}

impl CookState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicU8::new(ActiveCookType::None.as_u8())),
            active_since: Arc::new(Mutex::new(None)),
            channel_inflight: Arc::new(AtomicU32::new(0)),
            heartbeat_inflight: Arc::new(AtomicU32::new(0)),
        }
    }

    // ───────────────────────── #192b P2: in-flight counters ─────────────
    //
    // Under per-session dispatch, cooks for different sessions run
    // concurrently — the single-slot CAS (`try_acquire`) can no longer
    // answer "is a channel/heartbeat cook in flight?". These counters do.
    // The dispatcher calls `begin_cook(kind)` right before spawning a laned
    // cook and moves the returned `CookFlight` into the task; its `Drop`
    // (panic-safe) decrements on completion — so a panicking cook still
    // releases its count.

    /// Register an in-flight cook of `kind`. Returns a `CookFlight` whose
    /// drop decrements the matching counter. Move it into the spawned task.
    pub fn begin_cook(&self, kind: ActiveCookType) -> CookFlight {
        let counter = match kind {
            ActiveCookType::Channel => self.channel_inflight.clone(),
            ActiveCookType::Heartbeat => self.heartbeat_inflight.clone(),
            ActiveCookType::None => {
                debug_assert!(false, "cannot begin_cook(None)");
                self.channel_inflight.clone()
            }
        };
        counter.fetch_add(1, Ordering::SeqCst);
        CookFlight { counter }
    }

    /// Count of in-flight channel (human-initiated) cooks.
    #[inline]
    pub fn channel_inflight(&self) -> u32 {
        self.channel_inflight.load(Ordering::SeqCst)
    }

    /// Count of in-flight heartbeat (autonomy) cooks.
    #[inline]
    pub fn heartbeat_inflight(&self) -> u32 {
        self.heartbeat_inflight.load(Ordering::SeqCst)
    }

    /// True iff any channel cook is in flight (concurrency-aware replacement
    /// for `is_active()` on the heartbeat-deferral path). Preserves the
    /// channel-vs-heartbeat priority: the heartbeat defers to channel cooks.
    #[inline]
    pub fn is_channel_cooking(&self) -> bool {
        self.channel_inflight() > 0
    }

    /// True iff any cook (channel OR heartbeat) is in flight.
    #[inline]
    pub fn any_cook_inflight(&self) -> bool {
        self.channel_inflight() > 0 || self.heartbeat_inflight() > 0
    }

    /// How long the current cook has been active, if any.
    pub fn active_duration(&self) -> Option<std::time::Duration> {
        self.active_since.lock().ok()?.map(|t| t.elapsed())
    }

    /// Force-clear the cook slot regardless of who holds it. Use only when
    /// a guard has leaked (e.g. cook hung past its deadline). The next
    /// legitimate guard drop will be a no-op (its CAS will fail).
    pub fn force_clear(&self) {
        self.inner.store(ActiveCookType::None.as_u8(), Ordering::SeqCst);
        if let Ok(mut g) = self.active_since.lock() {
            *g = None;
        }
    }

    /// Current cook type (snapshot).
    #[inline]
    pub fn load(&self) -> ActiveCookType {
        ActiveCookType::from_u8(self.inner.load(Ordering::SeqCst))
    }

    /// True iff any cook is in flight.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.load() != ActiveCookType::None
    }

    /// Try to acquire the cook slot for `kind`.
    ///
    /// Returns `Some(CookGuard)` if the slot was `None` and is now `kind`.
    /// Returns `None` if another cook is already running — caller should
    /// queue / defer / skip the tick.
    pub fn try_acquire(&self, kind: ActiveCookType) -> Option<CookGuard> {
        debug_assert!(kind != ActiveCookType::None, "cannot acquire None");
        match self.inner.compare_exchange(
            ActiveCookType::None.as_u8(),
            kind.as_u8(),
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => {
                if let Ok(mut g) = self.active_since.lock() {
                    *g = Some(Instant::now());
                }
                Some(CookGuard {
                    state: self.inner.clone(),
                    expected: kind,
                    active_since: self.active_since.clone(),
                })
            }
            Err(_) => None,
        }
    }
}

/// RAII guard: clears the cook slot on drop, but only if it still holds
/// the value we set. If something else raced in, we leave it alone.
pub struct CookGuard {
    state: Arc<AtomicU8>,
    expected: ActiveCookType,
    active_since: Arc<Mutex<Option<Instant>>>,
}

impl CookGuard {
    pub fn kind(&self) -> ActiveCookType {
        self.expected
    }
}

impl Drop for CookGuard {
    fn drop(&mut self) {
        // compare_exchange: only clear if we still hold the slot.
        // This prevents clobbering a higher-priority cook that
        // legitimately swapped in while we were alive.
        if self.state.compare_exchange(
            self.expected.as_u8(),
            ActiveCookType::None.as_u8(),
            Ordering::SeqCst,
            Ordering::SeqCst,
        ).is_ok() {
            // We successfully cleared — also clear the timestamp.
            if let Ok(mut g) = self.active_since.lock() {
                *g = None;
            }
        }
    }
}

/// RAII in-flight token (#192b P2). Decrements its `CookState` counter on
/// drop — panic-safe, so a cook that panics mid-flight still releases its
/// count. Move it into the spawned cook task alongside the session lane
/// guard.
pub struct CookFlight {
    counter: Arc<AtomicU32>,
}

impl Drop for CookFlight {
    fn drop(&mut self) {
        // saturating: never wrap below zero even under a double-drop bug.
        let prev = self.counter.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(prev > 0, "CookFlight decremented below zero");
        if prev == 0 {
            // Defensive: restore to 0 if we somehow underflowed.
            self.counter.store(0, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_releases_on_drop() {
        let state = CookState::new();
        assert!(!state.is_active());

        {
            let _guard = state.try_acquire(ActiveCookType::Channel).expect("acquire");
            assert_eq!(state.load(), ActiveCookType::Channel);
        }

        assert!(!state.is_active(), "guard drop should clear state");
    }

    #[test]
    fn concurrent_acquire_blocks_second() {
        let state = CookState::new();
        let _g1 = state.try_acquire(ActiveCookType::Channel).expect("first acquire");

        // Second acquire of same kind — should fail (CAS expected None).
        assert!(state.try_acquire(ActiveCookType::Channel).is_none());
        // Different kind — also fails.
        assert!(state.try_acquire(ActiveCookType::Heartbeat).is_none());
    }

    #[test]
    fn self_deferral_via_cas_fail() {
        // Heartbeat tick #1 acquires.
        let state = CookState::new();
        let _g1 = state.try_acquire(ActiveCookType::Heartbeat).expect("tick 1");

        // Heartbeat tick #2 fires while #1 still running — skip.
        assert!(state.try_acquire(ActiveCookType::Heartbeat).is_none());
    }

    #[test]
    fn guard_drop_does_not_clobber_swapped_state() {
        // Simulate: guard alive, but state was externally changed.
        // Drop should NOT clear the new value.
        let state = CookState::new();
        let guard = state.try_acquire(ActiveCookType::Heartbeat).expect("acquire");

        // Externally force-swap to Channel (simulating a bug or a
        // higher-priority cook stealing the slot — shouldn't happen in
        // practice but we're defensive).
        state.inner.store(ActiveCookType::Channel.as_u8(), Ordering::SeqCst);

        drop(guard);
        // Should still be Channel, not None — guard's CAS failed.
        assert_eq!(state.load(), ActiveCookType::Channel);
    }

    #[test]
    fn active_duration_tracks_acquire() {
        let state = CookState::new();
        assert!(state.active_duration().is_none());

        let _guard = state.try_acquire(ActiveCookType::Heartbeat).expect("acquire");
        let dur = state.active_duration().expect("should have duration");
        assert!(dur < std::time::Duration::from_secs(1));

        drop(_guard);
        assert!(state.active_duration().is_none(), "drop should clear timestamp");
    }

    #[test]
    fn force_clear_reclaims_leaked_slot() {
        let state = CookState::new();
        // Simulate a leak: acquire and forget the guard.
        let guard = state.try_acquire(ActiveCookType::Heartbeat).expect("acquire");
        std::mem::forget(guard);
        assert!(state.is_active());
        assert!(state.active_duration().is_some());

        state.force_clear();
        assert!(!state.is_active());
        assert!(state.active_duration().is_none());

        // Can acquire again after force_clear.
        let _g2 = state.try_acquire(ActiveCookType::Channel).expect("reacquire after force_clear");
        assert_eq!(state.load(), ActiveCookType::Channel);
    }

    #[test]
    fn clone_shares_state() {
        let a = CookState::new();
        let b = a.clone();
        let _g = a.try_acquire(ActiveCookType::Channel).expect("acquire on a");
        assert!(b.is_active(), "clone sees same state");
        assert_eq!(b.load(), ActiveCookType::Channel);
    }

    // ───────────────────────── #192b P2: counter tests ──────────────────

    #[test]
    fn inflight_counter_tracks_concurrent_channel_cooks() {
        let state = CookState::new();
        assert!(!state.is_channel_cooking());
        assert_eq!(state.channel_inflight(), 0);

        let f1 = state.begin_cook(ActiveCookType::Channel);
        let f2 = state.begin_cook(ActiveCookType::Channel);
        assert_eq!(state.channel_inflight(), 2, "two concurrent channel cooks");
        assert!(state.is_channel_cooking());

        drop(f1);
        assert_eq!(state.channel_inflight(), 1, "one still in flight");
        assert!(state.is_channel_cooking());

        drop(f2);
        assert_eq!(state.channel_inflight(), 0);
        assert!(!state.is_channel_cooking(), "all channel cooks done");
    }

    #[test]
    fn inflight_counters_separate_channel_and_heartbeat() {
        let state = CookState::new();
        let c = state.begin_cook(ActiveCookType::Channel);
        let h = state.begin_cook(ActiveCookType::Heartbeat);
        assert_eq!(state.channel_inflight(), 1);
        assert_eq!(state.heartbeat_inflight(), 1);
        assert!(state.is_channel_cooking(), "channel cook gates heartbeat");
        assert!(state.any_cook_inflight());

        drop(c);
        assert!(!state.is_channel_cooking(), "channel cook ended");
        assert!(state.any_cook_inflight(), "heartbeat still in flight");
        drop(h);
        assert!(!state.any_cook_inflight());
    }

    #[test]
    fn cookflight_clone_shares_counters() {
        let a = CookState::new();
        let b = a.clone();
        let _f = a.begin_cook(ActiveCookType::Channel);
        assert_eq!(b.channel_inflight(), 1, "clone shares the inflight counter");
        assert!(b.is_channel_cooking());
    }

    #[test]
    fn heartbeat_defers_while_channel_cook_inflight() {
        // #192b P2 deferral migration: proves the counter-based deferral the
        // 4 migrated sites rely on. Under lanes, channel cooks mark the counter
        // (begin_cook) instead of the global CAS — so the heartbeat/autonomy
        // deferral reads must consult the counter, not is_active().
        let state = CookState::new();

        // No cook in flight → heartbeat may fire (gateway sites ① / ③ read
        // is_channel_cooking()).
        assert!(!state.is_channel_cooking(), "idle: heartbeat free to fire");

        // A channel cook begins (the gateway channel path begin_cook(Channel)
        // alongside its CAS).
        let channel = state.begin_cook(ActiveCookType::Channel);

        // Site ① (fallback HB) + site ③ (channel_busy advisory) + site ④
        // (heartbeat pre-check) all defer now — they read is_channel_cooking().
        assert!(
            state.is_channel_cooking(),
            "site ①/③/④: heartbeat must defer to the in-flight channel cook"
        );
        // Site ② (autonomous-goal loop) reads any_cook_inflight() and also defers.
        assert!(
            state.any_cook_inflight(),
            "site ②: autonomy defers while a channel cook is in flight"
        );

        drop(channel);
        assert!(
            !state.is_channel_cooking(),
            "channel cook done → heartbeat may fire again"
        );
        assert!(!state.any_cook_inflight(), "no cooks left in flight");
    }

    #[test]
    fn heartbeat_begin_cook_makes_autonomy_defer_honest() {
        // #192b P2 site ②↔④ coupling: the autonomous-goal loop defers on
        // any_cook_inflight(), which reads the COUNTERS. The heartbeat marks
        // itself via the CAS for singleton-ness, but must ALSO begin_cook(
        // Heartbeat) — otherwise any_cook_inflight() can't see it and site ②'s
        // "defer to the heartbeat" silently dies. This test pins that the
        // heartbeat's begin_cook is what makes site ② honest.
        let state = CookState::new();

        // Heartbeat acquires the CAS (singleton) — but the CAS alone is invisible
        // to the counter-based any_cook_inflight().
        let _cas = state
            .try_acquire(ActiveCookType::Heartbeat)
            .expect("heartbeat acquires the singleton slot");

        // The site-④ widening: begin_cook(Heartbeat) alongside the CAS, scoped to
        // the heartbeat's lifetime.
        let hb_flight = state.begin_cook(ActiveCookType::Heartbeat);
        assert!(
            state.any_cook_inflight(),
            "site ②: autonomy now sees the heartbeat via the counter"
        );
        // It is a heartbeat, not a channel cook — channel-only sites stay free.
        assert!(
            !state.is_channel_cooking(),
            "a heartbeat is not a channel cook (sites ①/③ unaffected)"
        );

        // Flight scoped to the heartbeat cook: when it ends, the counter clears so
        // site ② doesn't defer forever (the lifetime-scoping diff-read note).
        drop(hb_flight);
        assert!(
            !state.any_cook_inflight(),
            "heartbeat ended → counter clears → autonomy free to fire (no stuck >0)"
        );
    }

    #[test]
    fn cookflight_released_on_panic() {
        // A panic while holding a CookFlight still decrements (RAII Drop).
        let state = CookState::new();
        let s2 = state.clone();
        let res = std::panic::catch_unwind(move || {
            let _f = s2.begin_cook(ActiveCookType::Channel);
            assert_eq!(s2.channel_inflight(), 1);
            panic!("cook blew up");
        });
        assert!(res.is_err(), "closure should have panicked");
        assert_eq!(
            state.channel_inflight(),
            0,
            "CookFlight must decrement even on panic"
        );
    }
}
