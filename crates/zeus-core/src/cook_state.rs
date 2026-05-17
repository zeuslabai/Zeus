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
use std::sync::atomic::{AtomicU8, Ordering};
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
#[derive(Debug, Clone, Default)]
pub struct CookState {
    inner: Arc<AtomicU8>,
    /// When the current cook was acquired. Set by `try_acquire`, cleared on
    /// guard drop or `force_clear`. Used by the heartbeat to detect leaked
    /// guards and reclaim the slot.
    active_since: Arc<Mutex<Option<Instant>>>,
}

impl CookState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicU8::new(ActiveCookType::None.as_u8())),
            active_since: Arc::new(Mutex::new(None)),
        }
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
}
