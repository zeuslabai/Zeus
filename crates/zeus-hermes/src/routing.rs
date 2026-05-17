//! Adaptive Notification Routing
//!
//! Intelligent routing engine that selects optimal notification channels based on:
//! - **Urgency level**: Critical alerts go to all channels; low priority batched
//! - **Channel health**: Avoid channels with recent failures; prefer healthy ones
//! - **User preferences**: Time-of-day quiet hours, preferred channels per category
//! - **Delivery history**: Track success rates per channel, adapt routing accordingly
//! - **Cost awareness**: Prefer cheaper channels (push > email > SMS) when urgency allows

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the adaptive routing engine.
#[derive(Debug, Clone)]
pub struct RoutingConfig {
    /// Health threshold (0.0–1.0): channels below this are deprioritized.
    pub health_threshold: f64,
    /// Number of recent delivery attempts to track per channel.
    pub history_window: usize,
    /// Quiet hours start (0–23, hour of day UTC).
    pub quiet_hours_start: u8,
    /// Quiet hours end (0–23, hour of day UTC).
    pub quiet_hours_end: u8,
    /// Whether to respect quiet hours for non-critical notifications.
    pub enable_quiet_hours: bool,
    /// Maximum channels to route a single notification to.
    pub max_channels_per_notification: usize,
    /// Fallback channel if no healthy channels available.
    pub fallback_channel: Option<String>,
    /// Minimum success rate (0.0–1.0) to consider a channel reliable.
    pub min_success_rate: f64,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            health_threshold: 0.5,
            history_window: 50,
            quiet_hours_start: 22, // 10 PM UTC
            quiet_hours_end: 7,    // 7 AM UTC
            enable_quiet_hours: true,
            max_channels_per_notification: 3,
            fallback_channel: None,
            min_success_rate: 0.3,
        }
    }
}

// ============================================================================
// Types
// ============================================================================

/// Urgency level for a routable notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Urgency {
    /// System down, security breach, data loss — all channels immediately.
    Critical,
    /// Important but not emergency — primary + backup channels.
    High,
    /// Standard notification — preferred channel only.
    Normal,
    /// Informational — batch or defer, cheapest channel.
    Low,
}

/// A notification category for preference-based routing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NotificationCategory {
    /// Security alerts (auth failures, threat detection).
    Security,
    /// System health (resource usage, service status).
    System,
    /// Task completion or progress updates.
    Task,
    /// Chat messages from other agents or users.
    Chat,
    /// Billing, quota, or cost alerts.
    Billing,
    /// Custom category.
    Custom(String),
}

/// A channel available for routing.
#[derive(Debug, Clone)]
pub struct RouteChannel {
    /// Channel identifier (e.g., "telegram", "discord", "email").
    pub id: String,
    /// Relative cost tier (lower = cheaper): push=1, email=2, sms=5.
    pub cost_tier: u8,
    /// Whether this channel supports batching.
    pub supports_batching: bool,
    /// Maximum message length (0 = unlimited).
    pub max_message_length: usize,
}

/// Delivery attempt record.
#[derive(Debug, Clone)]
struct DeliveryRecord {
    /// Channel ID.
    channel_id: String,
    /// Whether delivery succeeded.
    success: bool,
    /// Unix timestamp of the attempt.
    timestamp: u64,
    /// Latency in milliseconds (if successful).
    latency_ms: Option<u64>,
}

/// Health snapshot for a channel.
#[derive(Debug, Clone)]
pub struct ChannelHealthSnapshot {
    /// Channel identifier.
    pub channel_id: String,
    /// Success rate (0.0–1.0) over recent history window.
    pub success_rate: f64,
    /// Average latency in milliseconds (successful deliveries only).
    pub avg_latency_ms: f64,
    /// Total attempts in window.
    pub total_attempts: usize,
    /// Last failure timestamp (if any).
    pub last_failure: Option<u64>,
    /// Whether the channel is considered healthy.
    pub healthy: bool,
}

/// A routing decision for a notification.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    /// Ordered list of channels to deliver to (best first).
    pub channels: Vec<String>,
    /// Why these channels were selected.
    pub reasoning: Vec<String>,
    /// Whether the notification should be deferred (quiet hours).
    pub deferred: bool,
    /// Whether the notification should be batched.
    pub batched: bool,
    /// Score breakdown per channel considered.
    pub scores: Vec<ChannelScore>,
}

/// Scoring details for a candidate channel.
#[derive(Debug, Clone)]
pub struct ChannelScore {
    /// Channel identifier.
    pub channel_id: String,
    /// Combined score (higher = better).
    pub total_score: f64,
    /// Health component of the score.
    pub health_score: f64,
    /// Cost component (inverted — cheaper = higher score).
    pub cost_score: f64,
    /// Preference component (user preference match).
    pub preference_score: f64,
    /// Whether this channel was selected.
    pub selected: bool,
}

/// User preferences for notification routing.
#[derive(Debug, Clone, Default)]
pub struct UserPreferences {
    /// Preferred channels per category.
    pub category_channels: HashMap<String, Vec<String>>,
    /// Channels to never use.
    pub blocked_channels: HashSet<String>,
    /// Override quiet hours (per-channel).
    pub quiet_hour_overrides: HashMap<String, bool>,
}

/// Statistics about routing operations.
#[derive(Debug, Clone, Default)]
pub struct RoutingStats {
    /// Total routing decisions made.
    pub decisions_made: usize,
    /// Total deliveries recorded.
    pub deliveries_recorded: usize,
    /// Total deferred (quiet hours).
    pub deferred_count: usize,
    /// Total batched.
    pub batched_count: usize,
    /// Total fallback invocations.
    pub fallback_count: usize,
}

// ============================================================================
// Adaptive Router
// ============================================================================

/// The adaptive notification routing engine.
pub struct AdaptiveRouter {
    config: RoutingConfig,
    /// Available channels.
    channels: Vec<RouteChannel>,
    /// Delivery history (ring buffer per channel).
    history: Vec<DeliveryRecord>,
    /// User preferences.
    preferences: UserPreferences,
    /// Statistics.
    stats: RoutingStats,
}

impl AdaptiveRouter {
    /// Create a new router with default configuration.
    pub fn new() -> Self {
        Self {
            config: RoutingConfig::default(),
            channels: Vec::new(),
            history: Vec::new(),
            preferences: UserPreferences::default(),
            stats: RoutingStats::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: RoutingConfig) -> Self {
        Self {
            config,
            channels: Vec::new(),
            history: Vec::new(),
            preferences: UserPreferences::default(),
            stats: RoutingStats::default(),
        }
    }

    /// Register a channel available for routing.
    pub fn register_channel(&mut self, channel: RouteChannel) {
        // Replace if exists
        self.channels.retain(|c| c.id != channel.id);
        self.channels.push(channel);
    }

    /// Remove a channel from routing.
    pub fn unregister_channel(&mut self, channel_id: &str) {
        self.channels.retain(|c| c.id != channel_id);
    }

    /// Set user preferences.
    pub fn set_preferences(&mut self, prefs: UserPreferences) {
        self.preferences = prefs;
    }

    /// Update configuration.
    pub fn set_config(&mut self, config: RoutingConfig) {
        self.config = config;
    }

    /// Get current statistics.
    pub fn stats(&self) -> &RoutingStats {
        &self.stats
    }

    /// Get registered channels.
    pub fn channels(&self) -> &[RouteChannel] {
        &self.channels
    }

    /// Record a delivery attempt for health tracking.
    pub fn record_delivery(&mut self, channel_id: &str, success: bool, latency_ms: Option<u64>) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.history.push(DeliveryRecord {
            channel_id: channel_id.to_string(),
            success,
            timestamp: now,
            latency_ms,
        });

        // Trim history per channel to window size
        let window = self.config.history_window;
        let mut per_channel: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, r) in self.history.iter().enumerate() {
            per_channel.entry(r.channel_id.clone()).or_default().push(i);
        }
        let mut remove_indices: Vec<usize> = Vec::new();
        for indices in per_channel.values() {
            if indices.len() > window {
                let excess = indices.len() - window;
                remove_indices.extend(&indices[..excess]);
            }
        }
        remove_indices.sort_by(|a, b| b.cmp(a));
        for idx in remove_indices {
            self.history.remove(idx);
        }

        self.stats.deliveries_recorded += 1;
    }

    /// Get health snapshot for a specific channel.
    pub fn channel_health(&self, channel_id: &str) -> ChannelHealthSnapshot {
        let records: Vec<&DeliveryRecord> = self
            .history
            .iter()
            .filter(|r| r.channel_id == channel_id)
            .collect();

        if records.is_empty() {
            return ChannelHealthSnapshot {
                channel_id: channel_id.to_string(),
                success_rate: 1.0, // Assume healthy if no data
                avg_latency_ms: 0.0,
                total_attempts: 0,
                last_failure: None,
                healthy: true,
            };
        }

        let total = records.len();
        let successes = records.iter().filter(|r| r.success).count();
        let success_rate = successes as f64 / total as f64;

        let latencies: Vec<u64> = records
            .iter()
            .filter_map(|r| if r.success { r.latency_ms } else { None })
            .collect();
        let avg_latency = if latencies.is_empty() {
            0.0
        } else {
            latencies.iter().sum::<u64>() as f64 / latencies.len() as f64
        };

        let last_failure = records
            .iter()
            .filter(|r| !r.success)
            .map(|r| r.timestamp)
            .max();

        ChannelHealthSnapshot {
            channel_id: channel_id.to_string(),
            success_rate,
            avg_latency_ms: avg_latency,
            total_attempts: total,
            last_failure,
            healthy: success_rate >= self.config.health_threshold,
        }
    }

    /// Route a notification: select optimal channels based on urgency, health, preferences.
    pub fn route(&mut self, urgency: Urgency, category: &NotificationCategory) -> RoutingDecision {
        self.route_at(urgency, category, None)
    }

    /// Route with an explicit timestamp (for testing).
    pub fn route_at(
        &mut self,
        urgency: Urgency,
        category: &NotificationCategory,
        now_override: Option<u64>,
    ) -> RoutingDecision {
        let now = now_override.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        });

        let mut reasoning = Vec::new();
        let mut deferred = false;
        let mut batched = false;

        // 1. Check quiet hours (non-critical only)
        if self.config.enable_quiet_hours && urgency != Urgency::Critical && self.is_quiet_hour(now)
        {
            if urgency == Urgency::Low {
                deferred = true;
                reasoning.push("Deferred: quiet hours active, low urgency".into());
                self.stats.deferred_count += 1;
                self.stats.decisions_made += 1;
                return RoutingDecision {
                    channels: Vec::new(),
                    reasoning,
                    deferred,
                    batched: false,
                    scores: Vec::new(),
                };
            }
            reasoning.push("Quiet hours active but urgency overrides".into());
        }

        // 2. Check batching for low urgency
        if urgency == Urgency::Low {
            batched = true;
            reasoning.push("Low urgency: eligible for batching".into());
            self.stats.batched_count += 1;
        }

        // 3. Score each channel
        let category_key = match category {
            NotificationCategory::Security => "security",
            NotificationCategory::System => "system",
            NotificationCategory::Task => "task",
            NotificationCategory::Chat => "chat",
            NotificationCategory::Billing => "billing",
            NotificationCategory::Custom(s) => s.as_str(),
        };

        let mut scores: Vec<ChannelScore> = Vec::new();

        for channel in &self.channels {
            // Skip blocked channels
            if self.preferences.blocked_channels.contains(&channel.id) {
                continue;
            }

            let health = self.channel_health(&channel.id);

            // Health score (0.0–1.0)
            let health_score = health.success_rate;

            // Cost score: invert cost_tier (1→1.0, 2→0.5, 5→0.2)
            let cost_score = 1.0 / channel.cost_tier.max(1) as f64;

            // Preference score: boost if user prefers this channel for this category
            let preference_score =
                if let Some(preferred) = self.preferences.category_channels.get(category_key) {
                    if preferred.contains(&channel.id) {
                        1.0
                    } else {
                        0.3
                    }
                } else {
                    0.5 // neutral
                };

            // Weight by urgency
            let (health_weight, cost_weight, pref_weight) = match urgency {
                Urgency::Critical => (0.7, 0.0, 0.3), // Health matters most, ignore cost
                Urgency::High => (0.5, 0.1, 0.4),
                Urgency::Normal => (0.3, 0.3, 0.4),
                Urgency::Low => (0.2, 0.5, 0.3), // Cost matters most
            };

            let total_score = health_score * health_weight
                + cost_score * cost_weight
                + preference_score * pref_weight;

            scores.push(ChannelScore {
                channel_id: channel.id.clone(),
                total_score,
                health_score,
                cost_score,
                preference_score,
                selected: false,
            });
        }

        // Sort by score descending
        scores.sort_by(|a, b| {
            b.total_score
                .partial_cmp(&a.total_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // 4. Select channels based on urgency
        let max_channels = match urgency {
            Urgency::Critical => self.channels.len(), // All available
            Urgency::High => 2.min(self.config.max_channels_per_notification),
            Urgency::Normal => 1,
            Urgency::Low => 1,
        };

        let mut selected: Vec<String> = Vec::new();
        for score in &mut scores {
            if selected.len() >= max_channels {
                break;
            }
            // Skip unhealthy channels for non-critical
            if urgency != Urgency::Critical && score.health_score < self.config.min_success_rate {
                reasoning.push(format!(
                    "Skipped {}: success rate {:.0}% below minimum",
                    score.channel_id,
                    score.health_score * 100.0
                ));
                continue;
            }
            score.selected = true;
            selected.push(score.channel_id.clone());
        }

        // 5. Fallback if nothing selected
        if selected.is_empty() {
            if let Some(ref fallback) = self.config.fallback_channel {
                selected.push(fallback.clone());
                reasoning.push(format!("Using fallback channel: {}", fallback));
                self.stats.fallback_count += 1;
            } else {
                reasoning.push("No healthy channels available and no fallback configured".into());
            }
        }

        if !selected.is_empty() {
            reasoning.push(format!(
                "Selected {} channel(s) for {:?} urgency",
                selected.len(),
                urgency
            ));
        }

        self.stats.decisions_made += 1;

        RoutingDecision {
            channels: selected,
            reasoning,
            deferred,
            batched,
            scores,
        }
    }

    /// Check if the given timestamp falls within quiet hours.
    fn is_quiet_hour(&self, unix_secs: u64) -> bool {
        let hour = ((unix_secs % 86400) / 3600) as u8;
        let start = self.config.quiet_hours_start;
        let end = self.config.quiet_hours_end;

        if start <= end {
            // Same day range: e.g., 8–17
            hour >= start && hour < end
        } else {
            // Overnight range: e.g., 22–7
            hour >= start || hour < end
        }
    }
}

impl Default for AdaptiveRouter {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_router() -> AdaptiveRouter {
        let mut router = AdaptiveRouter::new();
        router.register_channel(RouteChannel {
            id: "telegram".into(),
            cost_tier: 1,
            supports_batching: true,
            max_message_length: 4096,
        });
        router.register_channel(RouteChannel {
            id: "email".into(),
            cost_tier: 2,
            supports_batching: true,
            max_message_length: 0,
        });
        router.register_channel(RouteChannel {
            id: "sms".into(),
            cost_tier: 5,
            supports_batching: false,
            max_message_length: 160,
        });
        router
    }

    #[test]
    fn test_default_config() {
        let config = RoutingConfig::default();
        assert_eq!(config.health_threshold, 0.5);
        assert_eq!(config.history_window, 50);
        assert_eq!(config.quiet_hours_start, 22);
        assert_eq!(config.quiet_hours_end, 7);
        assert!(config.enable_quiet_hours);
        assert_eq!(config.max_channels_per_notification, 3);
    }

    #[test]
    fn test_new_router() {
        let router = AdaptiveRouter::new();
        assert!(router.channels().is_empty());
        assert_eq!(router.stats().decisions_made, 0);
    }

    #[test]
    fn test_register_channel() {
        let mut router = AdaptiveRouter::new();
        router.register_channel(RouteChannel {
            id: "telegram".into(),
            cost_tier: 1,
            supports_batching: true,
            max_message_length: 4096,
        });
        assert_eq!(router.channels().len(), 1);
        assert_eq!(router.channels()[0].id, "telegram");
    }

    #[test]
    fn test_register_replaces_existing() {
        let mut router = AdaptiveRouter::new();
        router.register_channel(RouteChannel {
            id: "telegram".into(),
            cost_tier: 1,
            supports_batching: true,
            max_message_length: 4096,
        });
        router.register_channel(RouteChannel {
            id: "telegram".into(),
            cost_tier: 3,
            supports_batching: false,
            max_message_length: 2000,
        });
        assert_eq!(router.channels().len(), 1);
        assert_eq!(router.channels()[0].cost_tier, 3);
    }

    #[test]
    fn test_unregister_channel() {
        let mut router = setup_router();
        assert_eq!(router.channels().len(), 3);
        router.unregister_channel("sms");
        assert_eq!(router.channels().len(), 2);
        assert!(!router.channels().iter().any(|c| c.id == "sms"));
    }

    #[test]
    fn test_route_critical_uses_all_channels() {
        let mut router = setup_router();
        let decision = router.route(Urgency::Critical, &NotificationCategory::Security);
        assert_eq!(decision.channels.len(), 3);
        assert!(!decision.deferred);
        assert!(!decision.batched);
    }

    #[test]
    fn test_route_high_uses_two_channels() {
        let mut router = setup_router();
        let decision = router.route(Urgency::High, &NotificationCategory::System);
        assert_eq!(decision.channels.len(), 2);
    }

    #[test]
    fn test_route_normal_uses_one_channel() {
        let mut router = setup_router();
        let decision = router.route(Urgency::Normal, &NotificationCategory::Task);
        assert_eq!(decision.channels.len(), 1);
    }

    #[test]
    fn test_route_low_is_batched() {
        let mut router = setup_router();
        // Use noon UTC (43200s from epoch) to avoid quiet hours (22:00-07:00 UTC)
        let decision = router.route_at(Urgency::Low, &NotificationCategory::Chat, Some(43200));
        assert!(decision.batched);
        assert_eq!(router.stats().batched_count, 1);
    }

    #[test]
    fn test_quiet_hours_defers_low() {
        let mut router = setup_router();
        // 23:30 UTC = quiet hours (22–7)
        let timestamp = 23 * 3600 + 30 * 60; // 23:30
        let decision = router.route_at(Urgency::Low, &NotificationCategory::Chat, Some(timestamp));
        assert!(decision.deferred);
        assert!(decision.channels.is_empty());
        assert_eq!(router.stats().deferred_count, 1);
    }

    #[test]
    fn test_quiet_hours_critical_overrides() {
        let mut router = setup_router();
        let timestamp = 23 * 3600 + 30 * 60; // 23:30 UTC
        let decision = router.route_at(
            Urgency::Critical,
            &NotificationCategory::Security,
            Some(timestamp),
        );
        assert!(!decision.deferred);
        assert!(!decision.channels.is_empty());
    }

    #[test]
    fn test_quiet_hours_normal_overrides() {
        let mut router = setup_router();
        let timestamp = 2 * 3600; // 2 AM UTC — quiet hours
        let decision = router.route_at(
            Urgency::Normal,
            &NotificationCategory::System,
            Some(timestamp),
        );
        // Normal urgency should still route during quiet hours (only Low is deferred)
        assert!(!decision.deferred);
        assert!(!decision.channels.is_empty());
    }

    #[test]
    fn test_outside_quiet_hours() {
        let mut router = setup_router();
        let timestamp = 12 * 3600; // noon UTC — not quiet
        let decision = router.route_at(Urgency::Low, &NotificationCategory::Chat, Some(timestamp));
        assert!(!decision.deferred);
    }

    #[test]
    fn test_health_tracking() {
        let mut router = setup_router();
        router.record_delivery("telegram", true, Some(50));
        router.record_delivery("telegram", true, Some(70));
        router.record_delivery("telegram", false, None);

        let health = router.channel_health("telegram");
        assert_eq!(health.total_attempts, 3);
        let expected_rate = 2.0 / 3.0;
        assert!((health.success_rate - expected_rate).abs() < 0.01);
        assert!((health.avg_latency_ms - 60.0).abs() < 0.01);
        assert!(health.last_failure.is_some());
    }

    #[test]
    fn test_no_history_assumes_healthy() {
        let router = setup_router();
        let health = router.channel_health("telegram");
        assert_eq!(health.success_rate, 1.0);
        assert!(health.healthy);
        assert_eq!(health.total_attempts, 0);
    }

    #[test]
    fn test_unhealthy_channel_deprioritized() {
        let mut router = setup_router();
        // Make telegram unhealthy
        for _ in 0..10 {
            router.record_delivery("telegram", false, None);
        }
        // Make email healthy
        for _ in 0..10 {
            router.record_delivery("email", true, Some(100));
        }

        let decision = router.route(Urgency::Normal, &NotificationCategory::Task);
        // Should pick email (healthy) over telegram (unhealthy)
        assert_eq!(decision.channels.len(), 1);
        assert_eq!(decision.channels[0], "email");
    }

    #[test]
    fn test_critical_uses_unhealthy_channels() {
        let mut router = setup_router();
        // Make all channels have some failures
        for _ in 0..5 {
            router.record_delivery("telegram", false, None);
            router.record_delivery("email", false, None);
            router.record_delivery("sms", false, None);
        }

        let decision = router.route(Urgency::Critical, &NotificationCategory::Security);
        // Critical should still use all channels regardless of health
        assert_eq!(decision.channels.len(), 3);
    }

    #[test]
    fn test_blocked_channels_excluded() {
        let mut router = setup_router();
        let mut prefs = UserPreferences::default();
        prefs.blocked_channels.insert("sms".into());
        router.set_preferences(prefs);

        let decision = router.route(Urgency::Critical, &NotificationCategory::Security);
        assert!(!decision.channels.contains(&"sms".to_string()));
    }

    #[test]
    fn test_preferred_channel_boosted() {
        let mut router = setup_router();
        let mut prefs = UserPreferences::default();
        prefs
            .category_channels
            .insert("task".into(), vec!["email".into()]);
        router.set_preferences(prefs);

        let decision = router.route(Urgency::Normal, &NotificationCategory::Task);
        // Email should be preferred for task category
        assert_eq!(decision.channels[0], "email");
    }

    #[test]
    fn test_cost_matters_for_low_urgency() {
        let mut router = setup_router();
        // All channels healthy — low urgency should prefer cheapest (telegram, cost_tier=1)
        let decision = router.route(Urgency::Low, &NotificationCategory::Chat);
        if !decision.deferred {
            assert_eq!(decision.channels[0], "telegram");
        }
    }

    #[test]
    fn test_fallback_channel() {
        let mut router = AdaptiveRouter::with_config(RoutingConfig {
            fallback_channel: Some("console".into()),
            min_success_rate: 1.0, // Unreachable threshold
            ..RoutingConfig::default()
        });
        // No channels registered, but fallback configured
        let decision = router.route(Urgency::Normal, &NotificationCategory::Task);
        assert_eq!(decision.channels, vec!["console"]);
        assert_eq!(router.stats().fallback_count, 1);
    }

    #[test]
    fn test_no_channels_no_fallback() {
        let mut router = AdaptiveRouter::new();
        let decision = router.route(Urgency::Normal, &NotificationCategory::Task);
        assert!(decision.channels.is_empty());
        assert!(
            decision
                .reasoning
                .iter()
                .any(|r| r.contains("No healthy channels"))
        );
    }

    #[test]
    fn test_stats_tracking() {
        let mut router = setup_router();
        router.route(Urgency::Normal, &NotificationCategory::Task);
        router.route(Urgency::High, &NotificationCategory::Security);
        router.record_delivery("telegram", true, Some(50));
        assert_eq!(router.stats().decisions_made, 2);
        assert_eq!(router.stats().deliveries_recorded, 1);
    }

    #[test]
    fn test_scores_populated() {
        let mut router = setup_router();
        let decision = router.route(Urgency::Normal, &NotificationCategory::Task);
        assert!(!decision.scores.is_empty());
        for score in &decision.scores {
            assert!(score.total_score >= 0.0 && score.total_score <= 1.0);
            assert!(score.health_score >= 0.0 && score.health_score <= 1.0);
            assert!(score.cost_score >= 0.0 && score.cost_score <= 1.0);
        }
    }

    #[test]
    fn test_reasoning_populated() {
        let mut router = setup_router();
        let decision = router.route(Urgency::Normal, &NotificationCategory::Task);
        assert!(!decision.reasoning.is_empty());
        assert!(decision.reasoning.iter().any(|r| r.contains("Selected")));
    }

    #[test]
    fn test_history_window_trimmed() {
        let mut router = AdaptiveRouter::with_config(RoutingConfig {
            history_window: 3,
            ..RoutingConfig::default()
        });
        router.register_channel(RouteChannel {
            id: "test".into(),
            cost_tier: 1,
            supports_batching: false,
            max_message_length: 0,
        });
        for i in 0..10 {
            router.record_delivery("test", i % 2 == 0, Some(100));
        }
        let health = router.channel_health("test");
        assert_eq!(health.total_attempts, 3);
    }

    #[test]
    fn test_quiet_hours_same_day_range() {
        let router = AdaptiveRouter::with_config(RoutingConfig {
            quiet_hours_start: 8,
            quiet_hours_end: 17,
            enable_quiet_hours: true,
            ..RoutingConfig::default()
        });
        // 10 AM should be quiet
        assert!(router.is_quiet_hour(10 * 3600));
        // 6 AM should not be quiet
        assert!(!router.is_quiet_hour(6 * 3600));
        // 18 should not be quiet
        assert!(!router.is_quiet_hour(18 * 3600));
    }

    #[test]
    fn test_quiet_hours_overnight_range() {
        let router = AdaptiveRouter::new(); // 22–7 default
        // 23 should be quiet
        assert!(router.is_quiet_hour(23 * 3600));
        // 3 AM should be quiet
        assert!(router.is_quiet_hour(3 * 3600));
        // 12 noon should not
        assert!(!router.is_quiet_hour(12 * 3600));
    }

    #[test]
    fn test_custom_category() {
        let mut router = setup_router();
        let mut prefs = UserPreferences::default();
        prefs
            .category_channels
            .insert("deployment".into(), vec!["email".into()]);
        router.set_preferences(prefs);

        let decision = router.route(
            Urgency::Normal,
            &NotificationCategory::Custom("deployment".into()),
        );
        assert_eq!(decision.channels[0], "email");
    }

    #[test]
    fn test_set_config() {
        let mut router = AdaptiveRouter::new();
        assert_eq!(router.config.health_threshold, 0.5);
        router.set_config(RoutingConfig {
            health_threshold: 0.8,
            ..RoutingConfig::default()
        });
        assert_eq!(router.config.health_threshold, 0.8);
    }
}
