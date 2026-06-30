//! Per-session cook lanes: `SessionLaneManager`.
//!
//! #192b P2 — the dispatcher primitive. Replaces the global single-cook
//! serializer (one `CookState` CAS fleet-wide) with **per-session lanes**
//! so different sessions cook concurrently while a single session stays
//! serialized + FIFO-ordered.
//!
//! Design notes:
//! - One `Arc<tokio::sync::Mutex<()>>` per resolved `cook_session_id`.
//!   Holding the session's mutex == that session is cooking. Different
//!   keys → different mutexes → no contention → genuine parallelism.
//! - The registry itself is a `std::sync::Mutex<HashMap<..>>` locked ONLY
//!   for the get-or-insert (microseconds) — never across a cook. We avoid
//!   a new `dashmap` dependency; the critical section is a single map
//!   probe, so a std mutex is more than adequate and keeps deps lean.
//! - The dispatcher acquires the lane via `lock_owned().await` BEFORE
//!   `tokio::spawn`, so the `OwnedMutexGuard` (which is `Send + 'static`)
//!   moves cleanly into the spawned task and is held for the cook's whole
//!   lifetime, dropped (RAII, panic-safe) at task end. Acquiring before
//!   spawn in the serial loop body preserves same-session FIFO ordering.
//! - The "inner-lane trap" (spark / OpenClaw `run.ts:648,793`): the
//!   spawned cook must take NO shared `agent.write()` — P1 removed it.
//!   The lane is the ONLY mutual-exclusion primitive on the cook path.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use tokio::sync::mpsc;
use tokio::sync::Mutex as AsyncMutex;

/// Manages one cook lane (async mutex) per resolved session key.
///
/// Clone freely — it's `Arc`-backed and shares the same lane registry.
#[derive(Clone, Default)]
pub struct SessionLaneManager {
    /// `session_key -> lane`. The std mutex guards only the get-or-insert;
    /// the per-key async mutex is what a cook actually holds.
    lanes: Arc<StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
    /// `session_key -> active interrupt sender`. The cook owns the receiver;
    /// producers route interrupts by resolved session key through this manager.
    interrupt_lanes: Arc<StdMutex<HashMap<String, mpsc::UnboundedSender<String>>>>,
}

impl std::fmt::Debug for SessionLaneManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.lanes.lock().map(|m| m.len()).unwrap_or(0);
        let interrupts = self.interrupt_lanes.lock().map(|m| m.len()).unwrap_or(0);
        f.debug_struct("SessionLaneManager")
            .field("lanes", &n)
            .field("interrupt_lanes", &interrupts)
            .finish()
    }
}

impl SessionLaneManager {
    pub fn new() -> Self {
        Self {
            lanes: Arc::new(StdMutex::new(HashMap::new())),
            interrupt_lanes: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    /// Get (or create) the cook lane for `key`. Cheap `Arc` clone.
    ///
    /// The caller acquires the lane via `.lock_owned().await` in the loop
    /// body (serial, before spawn) to preserve same-session FIFO order,
    /// then moves the `OwnedMutexGuard` into the spawned cook task.
    pub fn lane(&self, key: &str) -> Arc<AsyncMutex<()>> {
        let mut map = self
            .lanes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        map.entry(key.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    /// Number of distinct session lanes currently registered. Diagnostics.
    pub fn lane_count(&self) -> usize {
        self.lanes
            .lock()
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// True iff a cook currently holds `key`'s lane (i.e. that session is
    /// actively cooking). Non-blocking — uses `try_lock`. A `false` here
    /// is advisory only (the lane may be acquired the next instant).
    pub fn is_session_cooking(&self, key: &str) -> bool {
        let lane = {
            let map = match self.lanes.lock() {
                Ok(m) => m,
                Err(p) => p.into_inner(),
            };
            match map.get(key) {
                Some(l) => l.clone(),
                None => return false,
            }
        };
        // If we can't grab it, someone is holding it → that session cooks.
        lane.try_lock().is_err()
    }

    /// Number of sessions with an active interrupt receiver. Diagnostics/tests.
    pub fn interrupt_lane_count(&self) -> usize {
        self.interrupt_lanes.lock().map(|m| m.len()).unwrap_or(0)
    }

    /// Open/replace the active mpsc interrupt lane for `key`.
    ///
    /// Returns the receiver consumed by the cooking loop plus an RAII guard. The
    /// guard removes this exact sender on drop, so stale guards cannot clear a
    /// replacement lane opened by a newer cook for the same session.
    pub fn open_interrupt_lane(
        &self,
        key: impl Into<String>,
    ) -> (mpsc::UnboundedReceiver<String>, InterruptLaneGuard) {
        let key = key.into();
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        {
            let mut lanes = self
                .interrupt_lanes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            lanes.insert(key.clone(), tx.clone());
        }
        let guard = InterruptLaneGuard {
            key,
            tx,
            lanes: Arc::clone(&self.interrupt_lanes),
        };
        (rx, guard)
    }

    /// Send an interrupt payload to the active cook for `key`.
    ///
    /// Returns `true` iff an active receiver existed and accepted the payload.
    /// Closed/stale senders are pruned opportunistically and return `false`.
    pub fn send_interrupt(&self, key: &str, payload: String) -> bool {
        let sender = {
            let lanes = match self.interrupt_lanes.lock() {
                Ok(m) => m,
                Err(p) => p.into_inner(),
            };
            lanes.get(key).cloned()
        };

        let Some(sender) = sender else {
            return false;
        };

        if sender.send(payload).is_ok() {
            return true;
        }

        let mut lanes = self
            .interrupt_lanes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if lanes
            .get(key)
            .is_some_and(|current| current.same_channel(&sender))
        {
            lanes.remove(key);
        }
        false
    }
}

/// RAII cleanup for a session interrupt lane opened by a cook.
#[derive(Debug)]
pub struct InterruptLaneGuard {
    key: String,
    tx: mpsc::UnboundedSender<String>,
    lanes: Arc<StdMutex<HashMap<String, mpsc::UnboundedSender<String>>>>,
}

impl InterruptLaneGuard {
    /// Session key this guard owns.
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl Drop for InterruptLaneGuard {
    fn drop(&mut self) {
        let mut lanes = self
            .lanes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if lanes
            .get(&self.key)
            .is_some_and(|current| current.same_channel(&self.tx))
        {
            lanes.remove(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    #[test]
    fn lane_is_stable_per_key() {
        let mgr = SessionLaneManager::new();
        let a1 = mgr.lane("agent:main:main");
        let a2 = mgr.lane("agent:main:main");
        let b = mgr.lane("discord:chan:42");
        assert!(Arc::ptr_eq(&a1, &a2), "same key → same lane Arc");
        assert!(!Arc::ptr_eq(&a1, &b), "different key → different lane");
        assert_eq!(mgr.lane_count(), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn different_sessions_cook_concurrently() {
        // Two diff-session cooks, each holds its lane for ~150ms. If lanes
        // are independent, wall-time ≈ 150ms (parallel), not ~300ms (serial).
        let mgr = SessionLaneManager::new();
        let start = std::time::Instant::now();

        let m1 = mgr.clone();
        let m2 = mgr.clone();
        let t1 = tokio::spawn(async move {
            let _g = m1.lane("sessionA").lock_owned().await;
            tokio::time::sleep(Duration::from_millis(150)).await;
        });
        let t2 = tokio::spawn(async move {
            let _g = m2.lane("sessionB").lock_owned().await;
            tokio::time::sleep(Duration::from_millis(150)).await;
        });
        t1.await.unwrap();
        t2.await.unwrap();

        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(280),
            "diff sessions should run concurrently, took {elapsed:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn same_session_serializes_and_preserves_order() {
        // Two same-session cooks acquired in submission order must run
        // strictly serial AND in FIFO order.
        let mgr = SessionLaneManager::new();
        let order = Arc::new(StdMutex::new(Vec::<u32>::new()));
        let overlap = Arc::new(AtomicU32::new(0));
        let max_overlap = Arc::new(AtomicU32::new(0));

        // Acquire both lanes in submission order BEFORE spawning the cook
        // bodies — this is exactly what the dispatcher does in the loop.
        let g1 = mgr.lane("sameKey").lock_owned().await;
        let order1 = order.clone();
        let ov1 = overlap.clone();
        let mo1 = max_overlap.clone();
        let h1 = tokio::spawn(async move {
            let cur = ov1.fetch_add(1, Ordering::SeqCst) + 1;
            mo1.fetch_max(cur, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(80)).await;
            order1.lock().unwrap().push(1);
            ov1.fetch_sub(1, Ordering::SeqCst);
            drop(g1);
        });

        // Second cook: lane acquisition will BLOCK until the first drops.
        let lane2 = mgr.lane("sameKey");
        let order2 = order.clone();
        let ov2 = overlap.clone();
        let mo2 = max_overlap.clone();
        let h2 = tokio::spawn(async move {
            let g2 = lane2.lock_owned().await;
            let cur = ov2.fetch_add(1, Ordering::SeqCst) + 1;
            mo2.fetch_max(cur, Ordering::SeqCst);
            order2.lock().unwrap().push(2);
            ov2.fetch_sub(1, Ordering::SeqCst);
            drop(g2);
        });

        h1.await.unwrap();
        h2.await.unwrap();

        assert_eq!(
            max_overlap.load(Ordering::SeqCst),
            1,
            "same-session cooks must never overlap"
        );
        assert_eq!(
            *order.lock().unwrap(),
            vec![1, 2],
            "same-session cooks must preserve FIFO order"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn lane_released_on_panic() {
        // A panicking cook drops its OwnedMutexGuard (RAII) → the next
        // same-session cook proceeds. tokio's Mutex is NOT poisoned by a
        // panic while held, so no deadlock.
        let mgr = SessionLaneManager::new();

        let m1 = mgr.clone();
        let panicker = tokio::spawn(async move {
            let _g = m1.lane("victim").lock_owned().await;
            panic!("cook blew up mid-flight");
        });
        assert!(panicker.await.is_err(), "first task should have panicked");

        // Next cook on the same key must still be able to acquire the lane.
        let acquired = tokio::time::timeout(
            Duration::from_secs(1),
            mgr.lane("victim").lock_owned(),
        )
        .await;
        assert!(
            acquired.is_ok(),
            "lane must be released after a panicking cook (no poison deadlock)"
        );
    }

    #[tokio::test]
    async fn is_session_cooking_reflects_lane_hold() {
        let mgr = SessionLaneManager::new();
        assert!(!mgr.is_session_cooking("k"), "unknown key not cooking");
        let g = mgr.lane("k").lock_owned().await;
        assert!(mgr.is_session_cooking("k"), "held lane → cooking");
        drop(g);
        assert!(!mgr.is_session_cooking("k"), "released lane → not cooking");
    }

    #[tokio::test]
    async fn interrupt_lane_delivers_to_matching_session() {
        let mgr = SessionLaneManager::new();
        let (mut rx, _guard) = mgr.open_interrupt_lane("session-a");

        assert!(mgr.send_interrupt("session-a", "stop".to_string()));
        assert_eq!(rx.recv().await.as_deref(), Some("stop"));
    }

    #[tokio::test]
    async fn interrupt_lane_does_not_cross_sessions() {
        let mgr = SessionLaneManager::new();
        let (mut a_rx, _a_guard) = mgr.open_interrupt_lane("a");
        let (mut b_rx, _b_guard) = mgr.open_interrupt_lane("b");

        assert!(mgr.send_interrupt("a", "only-a".to_string()));
        assert_eq!(a_rx.recv().await.as_deref(), Some("only-a"));
        let no_b = tokio::time::timeout(Duration::from_millis(25), b_rx.recv()).await;
        assert!(no_b.is_err(), "interrupt for a must not wake b");
    }

    #[tokio::test]
    async fn interrupt_lane_guard_drop_cleans_lane() {
        let mgr = SessionLaneManager::new();
        let (_rx, guard) = mgr.open_interrupt_lane("drop-me");

        assert_eq!(mgr.interrupt_lane_count(), 1);
        drop(guard);

        assert_eq!(mgr.interrupt_lane_count(), 0);
        assert!(!mgr.send_interrupt("drop-me", "late".to_string()));
    }

    #[tokio::test]
    async fn stale_interrupt_guard_does_not_clear_replacement_lane() {
        let mgr = SessionLaneManager::new();
        let (_old_rx, old_guard) = mgr.open_interrupt_lane("same");
        let (mut new_rx, _new_guard) = mgr.open_interrupt_lane("same");

        drop(old_guard);

        assert_eq!(mgr.interrupt_lane_count(), 1);
        assert!(mgr.send_interrupt("same", "new".to_string()));
        assert_eq!(new_rx.recv().await.as_deref(), Some("new"));
    }
}
