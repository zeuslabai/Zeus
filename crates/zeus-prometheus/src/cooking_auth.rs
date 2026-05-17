//! Auth Profile Management
//!
//! Manages multiple authentication profiles with rotation, cooldown, and
//! automatic failover on rate limits, billing errors, or auth failures.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Reason for failover to another profile
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailoverReason {
    /// Rate limit hit (429)
    RateLimit,
    /// Billing/quota issue (402)
    Billing,
    /// Authentication failure (401, 403)
    Auth,
    /// Network/timeout error
    Network,
    /// Provider unavailable
    Unavailable,
    /// Context compaction triggered rotation
    Compaction,
    /// Manual rotation requested
    Manual,
}

impl std::fmt::Display for FailoverReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RateLimit => write!(f, "rate_limit"),
            Self::Billing => write!(f, "billing"),
            Self::Auth => write!(f, "auth"),
            Self::Network => write!(f, "network"),
            Self::Unavailable => write!(f, "unavailable"),
            Self::Compaction => write!(f, "compaction"),
            Self::Manual => write!(f, "manual"),
        }
    }
}

/// Status of an auth profile
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthProfileStatus {
    /// Profile is available for use
    Available,
    /// Profile is currently in use
    Active,
    /// Profile is in cooldown after failure
    Cooldown,
    /// Profile is permanently disabled
    Disabled,
}

/// Cooldown configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CooldownConfig {
    /// Base cooldown duration in milliseconds
    pub base_ms: u64,
    /// Maximum cooldown duration in milliseconds
    pub max_ms: u64,
    /// Multiplier for exponential backoff
    pub multiplier: f64,
    /// Special longer backoff for billing errors (5 hours default)
    pub billing_backoff_ms: u64,
    /// Window for counting failures (failures older than this are ignored)
    pub failure_window_ms: u64,
    /// Number of failures before longer cooldown kicks in
    pub failure_threshold: u32,
}

impl Default for CooldownConfig {
    fn default() -> Self {
        Self {
            base_ms: 60_000,                // 1 minute
            max_ms: 3_600_000,              // 1 hour
            multiplier: 2.0,                // Double each time
            billing_backoff_ms: 18_000_000, // 5 hours
            failure_window_ms: 86_400_000,  // 24 hours
            failure_threshold: 3,           // 3 failures before escalation
        }
    }
}

/// Failure record for a profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureRecord {
    /// Reason for failure
    pub reason: FailoverReason,
    /// When the failure occurred
    pub timestamp: DateTime<Utc>,
    /// Cooldown ends at this time
    pub cooldown_until: DateTime<Utc>,
}

/// Authentication profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProfile {
    /// Unique identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Provider (anthropic, openai, ollama)
    pub provider: String,
    /// Current status
    pub status: AuthProfileStatus,
    /// Priority (lower = higher priority)
    pub priority: u32,
    /// Failure history
    #[serde(default)]
    pub failures: Vec<FailureRecord>,
    /// Last successful use
    pub last_success: Option<DateTime<Utc>>,
    /// Total successful requests
    pub success_count: u64,
    /// Total failed requests
    pub failure_count: u64,
    /// When the profile was created
    pub created_at: DateTime<Utc>,
}

impl AuthProfile {
    /// Create a new auth profile
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        provider: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            provider: provider.into(),
            status: AuthProfileStatus::Available,
            priority: 100,
            failures: Vec::new(),
            last_success: None,
            success_count: 0,
            failure_count: 0,
            created_at: Utc::now(),
        }
    }

    /// Set the priority
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// Check if profile is available (not in cooldown or disabled)
    pub fn is_available(&self) -> bool {
        match self.status {
            AuthProfileStatus::Available | AuthProfileStatus::Active => true,
            AuthProfileStatus::Cooldown => {
                // Check if cooldown has expired
                if let Some(record) = self.failures.last() {
                    record.cooldown_until <= Utc::now()
                } else {
                    true
                }
            }
            AuthProfileStatus::Disabled => false,
        }
    }

    /// Get remaining cooldown duration
    pub fn cooldown_remaining(&self) -> Option<Duration> {
        if self.status != AuthProfileStatus::Cooldown {
            return None;
        }

        if let Some(record) = self.failures.last() {
            let now = Utc::now();
            if record.cooldown_until > now {
                let diff = record.cooldown_until - now;
                return Some(Duration::from_millis(diff.num_milliseconds() as u64));
            }
        }

        None
    }

    /// Record a successful use
    pub fn record_success(&mut self) {
        self.last_success = Some(Utc::now());
        self.success_count += 1;
        self.status = AuthProfileStatus::Available;
    }

    /// Record a failure and enter cooldown
    pub fn record_failure(&mut self, reason: FailoverReason, config: &CooldownConfig) {
        self.failure_count += 1;

        // Calculate cooldown based on recent failure count
        let recent_failures = self.count_recent_failures(config.failure_window_ms);
        let cooldown_ms = self.calculate_cooldown(reason, recent_failures, config);

        let now = Utc::now();
        let cooldown_until = now + chrono::Duration::milliseconds(cooldown_ms as i64);

        self.failures.push(FailureRecord {
            reason,
            timestamp: now,
            cooldown_until,
        });

        self.status = AuthProfileStatus::Cooldown;

        // Prune old failures outside the window
        self.prune_old_failures(config.failure_window_ms);
    }

    /// Count failures within the time window
    fn count_recent_failures(&self, window_ms: u64) -> u32 {
        let cutoff = Utc::now() - chrono::Duration::milliseconds(window_ms as i64);
        self.failures
            .iter()
            .filter(|f| f.timestamp > cutoff)
            .count() as u32
    }

    /// Calculate cooldown duration
    fn calculate_cooldown(
        &self,
        reason: FailoverReason,
        recent_failures: u32,
        config: &CooldownConfig,
    ) -> u64 {
        // Billing errors get a special long cooldown
        if matches!(reason, FailoverReason::Billing) {
            return config.billing_backoff_ms;
        }

        // Auth errors are also serious
        if matches!(reason, FailoverReason::Auth) {
            return config.max_ms; // Full hour cooldown
        }

        // Exponential backoff for other failures
        let base = config.base_ms as f64;
        let multiplier = config.multiplier.powi(recent_failures as i32);
        let cooldown = (base * multiplier) as u64;

        cooldown.min(config.max_ms)
    }

    /// Remove failures older than the window
    fn prune_old_failures(&mut self, window_ms: u64) {
        let cutoff = Utc::now() - chrono::Duration::milliseconds(window_ms as i64);
        self.failures.retain(|f| f.timestamp > cutoff);
    }

    /// Disable the profile
    pub fn disable(&mut self) {
        self.status = AuthProfileStatus::Disabled;
    }

    /// Enable a disabled profile
    pub fn enable(&mut self) {
        if self.status == AuthProfileStatus::Disabled {
            self.status = AuthProfileStatus::Available;
        }
    }
}

/// Manages multiple auth profiles with rotation
pub struct AuthProfileManager {
    /// All registered profiles
    profiles: HashMap<String, AuthProfile>,
    /// Cooldown configuration
    config: CooldownConfig,
    /// Current session profile assignments (session_id -> profile_id)
    session_profiles: HashMap<String, String>,
    /// Round-robin index for rotation
    rotation_index: usize,
}

impl AuthProfileManager {
    /// Create a new auth profile manager
    pub fn new(config: CooldownConfig) -> Self {
        Self {
            profiles: HashMap::new(),
            config,
            session_profiles: HashMap::new(),
            rotation_index: 0,
        }
    }

    /// Add a profile
    pub fn add_profile(&mut self, profile: AuthProfile) {
        self.profiles.insert(profile.id.clone(), profile);
    }

    /// Remove a profile
    pub fn remove_profile(&mut self, id: &str) -> Option<AuthProfile> {
        self.profiles.remove(id)
    }

    /// Get a profile by ID
    pub fn get_profile(&self, id: &str) -> Option<&AuthProfile> {
        self.profiles.get(id)
    }

    /// Get a mutable profile by ID
    pub fn get_profile_mut(&mut self, id: &str) -> Option<&mut AuthProfile> {
        self.profiles.get_mut(id)
    }

    /// Get all profiles
    pub fn all_profiles(&self) -> Vec<&AuthProfile> {
        self.profiles.values().collect()
    }

    /// Get next available profile (round-robin with priority)
    pub fn next_available(&mut self) -> Option<&AuthProfile> {
        // Sort by priority, then by success rate
        let mut available: Vec<_> = self
            .profiles
            .values()
            .filter(|p| p.is_available())
            .collect();

        if available.is_empty() {
            return None;
        }

        available.sort_by(|a, b| {
            a.priority.cmp(&b.priority).then_with(|| {
                // Higher success rate is better
                let a_rate = if a.success_count + a.failure_count > 0 {
                    a.success_count as f64 / (a.success_count + a.failure_count) as f64
                } else {
                    0.5
                };
                let b_rate = if b.success_count + b.failure_count > 0 {
                    b.success_count as f64 / (b.success_count + b.failure_count) as f64
                } else {
                    0.5
                };
                b_rate
                    .partial_cmp(&a_rate)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });

        // Round-robin within same priority
        self.rotation_index = (self.rotation_index + 1) % available.len();
        available.get(self.rotation_index).copied()
    }

    /// Get profile for a session (creates assignment if needed)
    pub fn get_session_profile(&mut self, session_id: &str) -> Option<&AuthProfile> {
        // Check existing assignment and validate
        let existing_valid = if let Some(profile_id) = self.session_profiles.get(session_id) {
            self.profiles
                .get(profile_id)
                .is_some_and(|p| p.is_available())
        } else {
            false
        };

        if existing_valid {
            // Safe to return the existing profile
            let profile_id = self
                .session_profiles
                .get(session_id)
                .expect("session profile exists after validation");
            return self.profiles.get(profile_id);
        }

        // Need to assign a new profile
        // First collect the profile id
        let new_profile_id = {
            let mut available: Vec<_> = self
                .profiles
                .values()
                .filter(|p| p.is_available())
                .collect();

            if available.is_empty() {
                return None;
            }

            available.sort_by(|a, b| a.priority.cmp(&b.priority));
            available.first().map(|p| p.id.clone())
        };

        if let Some(profile_id) = new_profile_id {
            self.session_profiles
                .insert(session_id.to_string(), profile_id.clone());
            return self.profiles.get(&profile_id);
        }

        None
    }

    /// Rotate session to a new profile (e.g., after compaction)
    /// Returns (from_profile_id, to_profile_id) as owned strings
    pub fn rotate_session_profile(
        &mut self,
        session_id: &str,
        reason: FailoverReason,
    ) -> Option<(String, String)> {
        let current_id = self.session_profiles.get(session_id)?.clone();

        // Mark current profile as failed
        let config = self.config.clone();
        if let Some(profile) = self.profiles.get_mut(&current_id) {
            profile.record_failure(reason, &config);
        }

        // Get next available profile
        let next_profile = self.next_available()?;
        let next_id = next_profile.id.clone();

        self.session_profiles
            .insert(session_id.to_string(), next_id.clone());

        Some((current_id, next_id))
    }

    /// Mark a profile as failed
    pub fn mark_failure(&mut self, profile_id: &str, reason: FailoverReason) {
        if let Some(profile) = self.profiles.get_mut(profile_id) {
            profile.record_failure(reason, &self.config);
        }
    }

    /// Mark a profile as successful
    pub fn mark_success(&mut self, profile_id: &str) {
        if let Some(profile) = self.profiles.get_mut(profile_id) {
            profile.record_success();
        }
    }

    /// Check if any profiles are available
    pub fn has_available_profiles(&self) -> bool {
        self.profiles.values().any(|p| p.is_available())
    }

    /// Get statistics about profiles
    pub fn stats(&self) -> AuthProfileStats {
        let total = self.profiles.len();
        let available = self.profiles.values().filter(|p| p.is_available()).count();
        let in_cooldown = self
            .profiles
            .values()
            .filter(|p| p.status == AuthProfileStatus::Cooldown)
            .count();
        let disabled = self
            .profiles
            .values()
            .filter(|p| p.status == AuthProfileStatus::Disabled)
            .count();

        AuthProfileStats {
            total,
            available,
            in_cooldown,
            disabled,
            active_sessions: self.session_profiles.len(),
        }
    }

    /// Clear session assignment
    pub fn clear_session(&mut self, session_id: &str) {
        self.session_profiles.remove(session_id);
    }

    /// Update cooldown configuration
    pub fn update_config(&mut self, config: CooldownConfig) {
        self.config = config;
    }
}

/// Statistics about auth profiles
#[derive(Debug, Clone)]
pub struct AuthProfileStats {
    pub total: usize,
    pub available: usize,
    pub in_cooldown: usize,
    pub disabled: usize,
    pub active_sessions: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_creation() {
        let profile = AuthProfile::new("prof-1", "Primary Claude", "anthropic");
        assert_eq!(profile.id, "prof-1");
        assert_eq!(profile.status, AuthProfileStatus::Available);
        assert!(profile.is_available());
    }

    #[test]
    fn test_profile_failure_cooldown() {
        let config = CooldownConfig {
            base_ms: 1000,
            max_ms: 10000,
            multiplier: 2.0,
            billing_backoff_ms: 5000,
            failure_window_ms: 60000,
            failure_threshold: 3,
        };

        let mut profile = AuthProfile::new("prof-1", "Test", "anthropic");
        profile.record_failure(FailoverReason::RateLimit, &config);

        assert_eq!(profile.status, AuthProfileStatus::Cooldown);
        assert_eq!(profile.failure_count, 1);
        assert!(profile.cooldown_remaining().is_some());
    }

    #[test]
    fn test_billing_longer_cooldown() {
        let config = CooldownConfig::default();
        let mut profile = AuthProfile::new("prof-1", "Test", "anthropic");

        profile.record_failure(FailoverReason::Billing, &config);

        // Billing should get the longer cooldown
        let remaining = profile.cooldown_remaining().unwrap();
        assert!(remaining.as_millis() as u64 > config.base_ms);
    }

    #[test]
    fn test_profile_success() {
        let config = CooldownConfig::default();
        let mut profile = AuthProfile::new("prof-1", "Test", "anthropic");

        profile.record_failure(FailoverReason::Network, &config);
        assert_eq!(profile.status, AuthProfileStatus::Cooldown);

        profile.record_success();
        assert_eq!(profile.status, AuthProfileStatus::Available);
        assert_eq!(profile.success_count, 1);
    }

    #[test]
    fn test_manager_rotation() {
        let config = CooldownConfig::default();
        let mut manager = AuthProfileManager::new(config);

        manager.add_profile(AuthProfile::new("prof-1", "Primary", "anthropic"));
        manager.add_profile(AuthProfile::new("prof-2", "Secondary", "openai"));

        let first = manager.next_available().map(|p| p.id.clone());
        let second = manager.next_available().map(|p| p.id.clone());

        // Should rotate between profiles
        assert!(first.is_some());
        assert!(second.is_some());
    }

    #[test]
    fn test_session_profile_assignment() {
        let config = CooldownConfig::default();
        let mut manager = AuthProfileManager::new(config);

        manager.add_profile(AuthProfile::new("prof-1", "Primary", "anthropic"));

        // First call assigns
        let profile_id = manager
            .get_session_profile("session-1")
            .map(|p| p.id.clone());
        assert!(profile_id.is_some());

        // Second call should return same profile
        let same_id = manager
            .get_session_profile("session-1")
            .map(|p| p.id.clone());
        assert_eq!(profile_id, same_id);
    }

    #[test]
    fn test_all_profiles_exhausted() {
        let config = CooldownConfig::default();
        let mut manager = AuthProfileManager::new(config.clone());

        let mut profile = AuthProfile::new("prof-1", "Only", "anthropic");
        profile.record_failure(FailoverReason::Billing, &config);
        manager.add_profile(profile);

        // Profile is in cooldown, should have none available
        assert!(!manager.has_available_profiles());
        assert!(manager.next_available().is_none());
    }

    #[test]
    fn test_stats() {
        let config = CooldownConfig::default();
        let mut manager = AuthProfileManager::new(config.clone());

        manager.add_profile(AuthProfile::new("prof-1", "Primary", "anthropic"));

        let mut disabled = AuthProfile::new("prof-2", "Disabled", "openai");
        disabled.disable();
        manager.add_profile(disabled);

        let stats = manager.stats();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.available, 1);
        assert_eq!(stats.disabled, 1);
    }
}
