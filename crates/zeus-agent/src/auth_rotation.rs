//! Auth Profile Rotation — per-agent API key profiles with rotation + cooldowns.
//!
//! Each agent can have multiple auth profiles (API keys for different providers).
//! The rotation system cycles through profiles based on:
//! - Rate limit exhaustion (429 responses)
//! - Usage quotas (daily/hourly token limits)
//! - Explicit cooldown periods
//! - Round-robin or priority-based selection

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// A single authentication profile for a provider.
#[derive(Debug, Clone)]
pub struct AuthProfile {
    /// Unique identifier for this profile.
    pub id: String,
    /// Provider name (e.g., "anthropic", "openai").
    pub provider: String,
    /// API key value.
    pub api_key: String,
    /// Maximum tokens per hour (0 = unlimited).
    pub hourly_limit: u64,
    /// Maximum tokens per day (0 = unlimited).
    pub daily_limit: u64,
    /// Priority (lower = preferred).
    pub priority: u32,
    /// Whether this profile is enabled.
    pub enabled: bool,
}

/// Tracks usage and cooldown state for a profile.
#[derive(Debug, Clone)]
struct ProfileState {
    /// Tokens used in current hour window.
    hourly_tokens: u64,
    /// Tokens used in current day window.
    daily_tokens: u64,
    /// When the hourly window started.
    hour_start: Instant,
    /// When the daily window started.
    day_start: Instant,
    /// If in cooldown, when it expires.
    cooldown_until: Option<Instant>,
    /// Number of consecutive failures (rate limits).
    consecutive_failures: u32,
    /// Total requests served by this profile.
    total_requests: u64,
}

impl ProfileState {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            hourly_tokens: 0,
            daily_tokens: 0,
            hour_start: now,
            day_start: now,
            cooldown_until: None,
            consecutive_failures: 0,
            total_requests: 0,
        }
    }

    /// Check if this profile is currently available (not in cooldown, not over limits).
    fn is_available(&self, profile: &AuthProfile) -> bool {
        if !profile.enabled {
            return false;
        }

        let now = Instant::now();

        // Check cooldown
        if let Some(until) = self.cooldown_until
            && now < until
        {
            return false;
        }

        // Reset hourly window if needed
        let hourly_tokens = if now.duration_since(self.hour_start) >= Duration::from_secs(3600) {
            0
        } else {
            self.hourly_tokens
        };

        // Reset daily window if needed
        let daily_tokens = if now.duration_since(self.day_start) >= Duration::from_secs(86400) {
            0
        } else {
            self.daily_tokens
        };

        // Check limits
        if profile.hourly_limit > 0 && hourly_tokens >= profile.hourly_limit {
            return false;
        }
        if profile.daily_limit > 0 && daily_tokens >= profile.daily_limit {
            return false;
        }

        true
    }
}

/// Rotation strategy for selecting profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationStrategy {
    /// Use highest-priority (lowest number) available profile.
    Priority,
    /// Cycle through profiles in order.
    RoundRobin,
    /// Pick the profile with the least usage.
    LeastUsed,
}

/// Manages auth profiles for an agent with automatic rotation.
#[derive(Clone)]
pub struct AuthRotationManager {
    profiles: Arc<RwLock<Vec<AuthProfile>>>,
    states: Arc<RwLock<HashMap<String, ProfileState>>>,
    strategy: Arc<RwLock<RotationStrategy>>,
    round_robin_index: Arc<RwLock<usize>>,
    /// Default cooldown duration after a rate limit hit.
    cooldown_duration: Duration,
}

impl AuthRotationManager {
    /// Create a new rotation manager.
    pub fn new(strategy: RotationStrategy) -> Self {
        Self {
            profiles: Arc::new(RwLock::new(Vec::new())),
            states: Arc::new(RwLock::new(HashMap::new())),
            strategy: Arc::new(RwLock::new(strategy)),
            round_robin_index: Arc::new(RwLock::new(0)),
            cooldown_duration: Duration::from_secs(60),
        }
    }

    /// Create with a custom cooldown duration.
    pub fn with_cooldown(mut self, duration: Duration) -> Self {
        self.cooldown_duration = duration;
        self
    }

    /// Add an auth profile.
    pub async fn add_profile(&self, profile: AuthProfile) {
        let id = profile.id.clone();
        self.profiles.write().await.push(profile);
        self.states.write().await.insert(id, ProfileState::new());
    }

    /// Remove a profile by ID.
    pub async fn remove_profile(&self, id: &str) -> bool {
        let mut profiles = self.profiles.write().await;
        let before = profiles.len();
        profiles.retain(|p| p.id != id);
        self.states.write().await.remove(id);
        profiles.len() < before
    }

    /// Get the best available API key for a given provider.
    pub async fn get_key(&self, provider: &str) -> Option<String> {
        let profiles = self.profiles.read().await;
        let states = self.states.read().await;
        let strategy = *self.strategy.read().await;

        let available: Vec<&AuthProfile> = profiles
            .iter()
            .filter(|p| p.provider == provider)
            .filter(|p| {
                states
                    .get(&p.id)
                    .map(|s| s.is_available(p))
                    .unwrap_or(false)
            })
            .collect();

        if available.is_empty() {
            return None;
        }

        match strategy {
            RotationStrategy::Priority => {
                let mut sorted = available;
                sorted.sort_by_key(|p| p.priority);
                Some(sorted[0].api_key.clone())
            }
            RotationStrategy::RoundRobin => {
                let mut idx = self.round_robin_index.write().await;
                let profile = &available[*idx % available.len()];
                *idx = (*idx + 1) % available.len();
                Some(profile.api_key.clone())
            }
            RotationStrategy::LeastUsed => {
                let mut best: Option<(&AuthProfile, u64)> = None;
                for p in &available {
                    let usage = states.get(&p.id).map(|s| s.total_requests).unwrap_or(0);
                    if best
                        .as_ref()
                        .is_none_or(|(_, best_usage)| usage < *best_usage)
                    {
                        best = Some((p, usage));
                    }
                }
                best.map(|(p, _)| p.api_key.clone())
            }
        }
    }

    /// Record successful usage of a profile.
    pub async fn record_usage(&self, profile_id: &str, tokens: u64) {
        let mut states = self.states.write().await;
        if let Some(state) = states.get_mut(profile_id) {
            let now = Instant::now();

            // Reset windows if expired
            if now.duration_since(state.hour_start) >= Duration::from_secs(3600) {
                state.hourly_tokens = 0;
                state.hour_start = now;
            }
            if now.duration_since(state.day_start) >= Duration::from_secs(86400) {
                state.daily_tokens = 0;
                state.day_start = now;
            }

            state.hourly_tokens += tokens;
            state.daily_tokens += tokens;
            state.total_requests += 1;
            state.consecutive_failures = 0;
        }
    }

    /// Record a rate limit (429) response — puts profile in cooldown.
    pub async fn record_rate_limit(&self, profile_id: &str) {
        let mut states = self.states.write().await;
        if let Some(state) = states.get_mut(profile_id) {
            state.consecutive_failures += 1;
            // Exponential backoff: cooldown_duration * 2^(failures-1), capped at 30 min
            let multiplier = 2u32.saturating_pow(state.consecutive_failures.saturating_sub(1));
            let cooldown = self.cooldown_duration * multiplier;
            let capped = cooldown.min(Duration::from_secs(1800));
            state.cooldown_until = Some(Instant::now() + capped);
        }
    }

    /// Manually put a profile in cooldown.
    pub async fn set_cooldown(&self, profile_id: &str, duration: Duration) {
        let mut states = self.states.write().await;
        if let Some(state) = states.get_mut(profile_id) {
            state.cooldown_until = Some(Instant::now() + duration);
        }
    }

    /// Get profile status summary.
    pub async fn status(&self) -> Vec<ProfileStatus> {
        let profiles = self.profiles.read().await;
        let states = self.states.read().await;
        let now = Instant::now();

        profiles
            .iter()
            .map(|p| {
                let state = states.get(&p.id);
                let available = state.map(|s| s.is_available(p)).unwrap_or(false);
                let in_cooldown = state
                    .and_then(|s| s.cooldown_until)
                    .map(|until| now < until)
                    .unwrap_or(false);
                let cooldown_remaining = state.and_then(|s| s.cooldown_until).and_then(|until| {
                    if now < until {
                        Some(until.duration_since(now))
                    } else {
                        None
                    }
                });

                ProfileStatus {
                    id: p.id.clone(),
                    provider: p.provider.clone(),
                    priority: p.priority,
                    enabled: p.enabled,
                    available,
                    in_cooldown,
                    cooldown_remaining_secs: cooldown_remaining.map(|d| d.as_secs()),
                    hourly_tokens: state.map(|s| s.hourly_tokens).unwrap_or(0),
                    daily_tokens: state.map(|s| s.daily_tokens).unwrap_or(0),
                    total_requests: state.map(|s| s.total_requests).unwrap_or(0),
                    consecutive_failures: state.map(|s| s.consecutive_failures).unwrap_or(0),
                }
            })
            .collect()
    }

    /// Get total number of registered profiles.
    pub async fn profile_count(&self) -> usize {
        self.profiles.read().await.len()
    }

    /// Change the rotation strategy.
    pub async fn set_strategy(&self, strategy: RotationStrategy) {
        *self.strategy.write().await = strategy;
    }
}

/// Status snapshot of a single profile.
#[derive(Debug, Clone)]
pub struct ProfileStatus {
    pub id: String,
    pub provider: String,
    pub priority: u32,
    pub enabled: bool,
    pub available: bool,
    pub in_cooldown: bool,
    pub cooldown_remaining_secs: Option<u64>,
    pub hourly_tokens: u64,
    pub daily_tokens: u64,
    pub total_requests: u64,
    pub consecutive_failures: u32,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_profile(id: &str, provider: &str, priority: u32) -> AuthProfile {
        AuthProfile {
            id: id.to_string(),
            provider: provider.to_string(),
            api_key: format!("sk-{id}"),
            hourly_limit: 0,
            daily_limit: 0,
            priority,
            enabled: true,
        }
    }

    #[tokio::test]
    async fn test_add_and_get_key() {
        let mgr = AuthRotationManager::new(RotationStrategy::Priority);
        mgr.add_profile(test_profile("p1", "openai", 1)).await;
        let key = mgr.get_key("openai").await;
        assert_eq!(key, Some("sk-p1".to_string()));
    }

    #[tokio::test]
    async fn test_priority_rotation() {
        let mgr = AuthRotationManager::new(RotationStrategy::Priority);
        mgr.add_profile(test_profile("p2", "openai", 2)).await;
        mgr.add_profile(test_profile("p1", "openai", 1)).await;
        let key = mgr.get_key("openai").await;
        assert_eq!(key, Some("sk-p1".to_string()));
    }

    #[tokio::test]
    async fn test_round_robin() {
        let mgr = AuthRotationManager::new(RotationStrategy::RoundRobin);
        mgr.add_profile(test_profile("a", "openai", 1)).await;
        mgr.add_profile(test_profile("b", "openai", 1)).await;

        let k1 = mgr.get_key("openai").await.unwrap();
        let k2 = mgr.get_key("openai").await.unwrap();
        // Should cycle through
        assert_ne!(k1, k2);
    }

    #[tokio::test]
    async fn test_least_used() {
        let mgr = AuthRotationManager::new(RotationStrategy::LeastUsed);
        mgr.add_profile(test_profile("a", "openai", 1)).await;
        mgr.add_profile(test_profile("b", "openai", 1)).await;

        // Use "a" twice
        mgr.record_usage("a", 100).await;
        mgr.record_usage("a", 100).await;

        // Should prefer "b" since it has 0 usage
        let key = mgr.get_key("openai").await;
        assert_eq!(key, Some("sk-b".to_string()));
    }

    #[tokio::test]
    async fn test_rate_limit_cooldown() {
        let mgr = AuthRotationManager::new(RotationStrategy::Priority)
            .with_cooldown(Duration::from_millis(100));
        mgr.add_profile(test_profile("p1", "openai", 1)).await;

        mgr.record_rate_limit("p1").await;
        // Should be in cooldown
        let key = mgr.get_key("openai").await;
        assert!(key.is_none());

        // Wait for cooldown
        tokio::time::sleep(Duration::from_millis(150)).await;
        let key = mgr.get_key("openai").await;
        assert_eq!(key, Some("sk-p1".to_string()));
    }

    #[tokio::test]
    async fn test_provider_filtering() {
        let mgr = AuthRotationManager::new(RotationStrategy::Priority);
        mgr.add_profile(test_profile("oai", "openai", 1)).await;
        mgr.add_profile(test_profile("ant", "anthropic", 1)).await;

        let oai = mgr.get_key("openai").await;
        let ant = mgr.get_key("anthropic").await;
        let goo = mgr.get_key("google").await;

        assert_eq!(oai, Some("sk-oai".to_string()));
        assert_eq!(ant, Some("sk-ant".to_string()));
        assert!(goo.is_none());
    }

    #[tokio::test]
    async fn test_remove_profile() {
        let mgr = AuthRotationManager::new(RotationStrategy::Priority);
        mgr.add_profile(test_profile("p1", "openai", 1)).await;
        assert_eq!(mgr.profile_count().await, 1);

        let removed = mgr.remove_profile("p1").await;
        assert!(removed);
        assert_eq!(mgr.profile_count().await, 0);
        assert!(mgr.get_key("openai").await.is_none());
    }

    #[tokio::test]
    async fn test_hourly_limit() {
        let mgr = AuthRotationManager::new(RotationStrategy::Priority);
        mgr.add_profile(AuthProfile {
            id: "limited".to_string(),
            provider: "openai".to_string(),
            api_key: "sk-limited".to_string(),
            hourly_limit: 100,
            daily_limit: 0,
            priority: 1,
            enabled: true,
        })
        .await;

        // Use up the limit
        mgr.record_usage("limited", 100).await;
        assert!(mgr.get_key("openai").await.is_none());
    }

    #[tokio::test]
    async fn test_disabled_profile_skipped() {
        let mgr = AuthRotationManager::new(RotationStrategy::Priority);
        mgr.add_profile(AuthProfile {
            id: "disabled".to_string(),
            provider: "openai".to_string(),
            api_key: "sk-disabled".to_string(),
            hourly_limit: 0,
            daily_limit: 0,
            priority: 1,
            enabled: false,
        })
        .await;

        assert!(mgr.get_key("openai").await.is_none());
    }

    #[tokio::test]
    async fn test_status_report() {
        let mgr = AuthRotationManager::new(RotationStrategy::Priority);
        mgr.add_profile(test_profile("p1", "openai", 1)).await;
        mgr.record_usage("p1", 500).await;

        let statuses = mgr.status().await;
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].id, "p1");
        assert_eq!(statuses[0].hourly_tokens, 500);
        assert_eq!(statuses[0].total_requests, 1);
        assert!(statuses[0].available);
        assert!(!statuses[0].in_cooldown);
    }

    #[tokio::test]
    async fn test_exponential_backoff() {
        let mgr = AuthRotationManager::new(RotationStrategy::Priority)
            .with_cooldown(Duration::from_millis(50));
        mgr.add_profile(test_profile("p1", "openai", 1)).await;

        // First rate limit: 50ms cooldown
        mgr.record_rate_limit("p1").await;
        let statuses = mgr.status().await;
        assert!(statuses[0].in_cooldown);
        assert_eq!(statuses[0].consecutive_failures, 1);

        // Wait and hit again: 100ms cooldown (2x)
        tokio::time::sleep(Duration::from_millis(60)).await;
        mgr.record_rate_limit("p1").await;
        let statuses = mgr.status().await;
        assert_eq!(statuses[0].consecutive_failures, 2);
    }

    #[tokio::test]
    async fn test_fallback_on_cooldown() {
        let mgr = AuthRotationManager::new(RotationStrategy::Priority)
            .with_cooldown(Duration::from_secs(60));
        mgr.add_profile(test_profile("primary", "openai", 1)).await;
        mgr.add_profile(test_profile("backup", "openai", 2)).await;

        // Put primary in cooldown
        mgr.record_rate_limit("primary").await;

        // Should fall back to backup
        let key = mgr.get_key("openai").await;
        assert_eq!(key, Some("sk-backup".to_string()));
    }
}
