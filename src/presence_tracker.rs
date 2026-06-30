//! Presence Tracker — observes peer agent messages to track liveness.
//!
//! The election in `gateway_consumer::check_mention` picks the alphabetically-first
//! peer agent to respond to role/broadcast mentions. If that agent is offline or
//! wedged, nobody responds. This tracker gives us a last-seen timestamp per peer
//! so election can filter out dead peers before picking a winner.
//!
//! Population: call `record_seen(name)` whenever we observe a message from a peer
//! agent in the channel consumer. Query: call `live_peers(all_peers, staleness)`
//! during election to get the subset of peers seen within the staleness window.
//!
//! Self is always considered live (we're obviously running to execute this code).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Default staleness window for considering a peer "live" — 120 seconds.
pub const DEFAULT_STALENESS_SECS: u64 = 120;

/// Thread-safe presence tracker for peer agent liveness.
#[derive(Clone, Default)]
pub struct PresenceTracker {
    inner: Arc<RwLock<HashMap<String, Instant>>>,
}

impl PresenceTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that we observed activity from `agent_name` right now.
    /// Name is normalized to lowercase for matching.
    pub fn record_seen(&self, agent_name: &str) {
        let key = agent_name.to_lowercase();
        if let Ok(mut map) = self.inner.write() {
            map.insert(key, Instant::now());
        }
    }

    /// Filter `all_peers` (including self) to the subset considered live:
    /// self is always included, peers with `last_seen` within `staleness` are included,
    /// peers never seen or stale are excluded.
    ///
    /// Returns lowercased names for direct comparison with election logic.
    pub fn live_peers(
        &self,
        all_peers: &[String],
        self_name: &str,
        staleness: Duration,
    ) -> Vec<String> {
        let self_lower = self_name.to_lowercase();
        let map = match self.inner.read() {
            Ok(g) => g,
            Err(_) => return vec![self_lower], // poisoned lock → only self is safe
        };
        let now = Instant::now();
        let mut live: Vec<String> = all_peers
            .iter()
            .map(|n| n.to_lowercase())
            .filter(|name| {
                if name == &self_lower {
                    return true; // self is always live
                }
                match map.get(name) {
                    Some(seen) => now.duration_since(*seen) < staleness,
                    None => false,
                }
            })
            .collect();
        // Ensure self is in the list even if it wasn't in all_peers
        if !live.contains(&self_lower) {
            live.push(self_lower);
        }
        live
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn self_always_live_with_empty_tracker() {
        let t = PresenceTracker::new();
        let peers = vec!["zeus100".into(), "zeus112".into()];
        let live = t.live_peers(&peers, "zeus112", Duration::from_secs(60));
        assert_eq!(live, vec!["zeus112".to_string()]);
    }

    #[test]
    fn recorded_peer_is_live() {
        let t = PresenceTracker::new();
        t.record_seen("Zeus100");
        let peers = vec!["zeus100".into(), "zeus112".into()];
        let live = t.live_peers(&peers, "zeus112", Duration::from_secs(60));
        assert!(live.contains(&"zeus100".to_string()));
        assert!(live.contains(&"zeus112".to_string()));
    }

    #[test]
    fn stale_peer_is_excluded() {
        let t = PresenceTracker::new();
        t.record_seen("zeus100");
        sleep(Duration::from_millis(50));
        let peers = vec!["zeus100".into(), "zeus112".into()];
        let live = t.live_peers(&peers, "zeus112", Duration::from_millis(10));
        assert!(!live.contains(&"zeus100".to_string()));
        assert!(live.contains(&"zeus112".to_string()));
    }

    #[test]
    fn case_insensitive_matching() {
        let t = PresenceTracker::new();
        t.record_seen("ZEUS100");
        let peers = vec!["Zeus100".into()];
        let live = t.live_peers(&peers, "zeus112", Duration::from_secs(60));
        assert!(live.contains(&"zeus100".to_string()));
    }

    #[test]
    fn unseen_peers_excluded() {
        let t = PresenceTracker::new();
        let peers = vec!["zeus100".into(), "zeus106".into(), "zeus112".into()];
        let live = t.live_peers(&peers, "zeus112", Duration::from_secs(60));
        assert_eq!(live, vec!["zeus112".to_string()]);
    }

    #[test]
    fn clone_shares_state() {
        let t1 = PresenceTracker::new();
        let t2 = t1.clone();
        t1.record_seen("zeus100");
        let peers = vec!["zeus100".into()];
        let live = t2.live_peers(&peers, "zeus112", Duration::from_secs(60));
        assert!(live.contains(&"zeus100".to_string()));
    }
}
