//! Channel Health Monitoring
//!
//! Periodic health checks + automatic reconnect for all channel adapters.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

// ============================================================================
// Types
// ============================================================================

/// Health status of a single channel adapter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelHealth {
    /// Channel type identifier (e.g., "telegram", "discord")
    pub channel_type: String,
    /// Whether the channel is currently connected
    pub connected: bool,
    /// Timestamp of the last health check
    pub last_check: DateTime<Utc>,
    /// Timestamp when the channel was last seen connected
    pub last_connected: Option<DateTime<Utc>>,
    /// Number of consecutive health check failures
    pub consecutive_failures: u32,
    /// Total checks performed
    pub total_checks: u64,
    /// Total checks where channel was connected
    pub connected_checks: u64,
}

impl ChannelHealth {
    /// Create a new health record for a channel
    pub fn new(channel_type: &str) -> Self {
        Self {
            channel_type: channel_type.to_string(),
            connected: false,
            last_check: Utc::now(),
            last_connected: None,
            consecutive_failures: 0,
            total_checks: 0,
            connected_checks: 0,
        }
    }

    /// Record a successful health check
    pub fn record_success(&mut self) {
        let now = Utc::now();
        self.connected = true;
        self.last_check = now;
        self.last_connected = Some(now);
        self.consecutive_failures = 0;
        self.total_checks += 1;
        self.connected_checks += 1;
    }

    /// Record a failed health check
    pub fn record_failure(&mut self) {
        self.connected = false;
        self.last_check = Utc::now();
        self.consecutive_failures += 1;
        self.total_checks += 1;
    }

    /// Calculate uptime percentage (0.0 to 100.0)
    pub fn uptime_pct(&self) -> f64 {
        if self.total_checks == 0 {
            return 0.0;
        }
        (self.connected_checks as f64 / self.total_checks as f64) * 100.0
    }
}

/// Configuration for health monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthConfig {
    /// Interval between health checks in seconds
    #[serde(default = "default_check_interval")]
    pub check_interval_secs: u64,
    /// Maximum reconnection attempts before giving up
    #[serde(default = "default_max_reconnect")]
    pub max_reconnect_attempts: u32,
    /// Delay between reconnection attempts in seconds
    #[serde(default = "default_reconnect_delay")]
    pub reconnect_delay_secs: u64,
    /// Number of consecutive failures before alerting
    #[serde(default = "default_alert_threshold")]
    pub alert_after_failures: u32,
}

fn default_check_interval() -> u64 {
    30
}
fn default_max_reconnect() -> u32 {
    3
}
fn default_reconnect_delay() -> u64 {
    5
}
fn default_alert_threshold() -> u32 {
    3
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            check_interval_secs: default_check_interval(),
            max_reconnect_attempts: default_max_reconnect(),
            reconnect_delay_secs: default_reconnect_delay(),
            alert_after_failures: default_alert_threshold(),
        }
    }
}

/// Aggregated health status for all channels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// Whether all channels are healthy
    pub overall_healthy: bool,
    /// Number of connected channels
    pub connected_count: usize,
    /// Total number of monitored channels
    pub total_count: usize,
    /// Per-channel health details
    pub channels: Vec<ChannelHealthReport>,
    /// Timestamp of this status report
    pub checked_at: DateTime<Utc>,
}

/// Per-channel health report (serialization-friendly)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelHealthReport {
    pub channel_type: String,
    pub connected: bool,
    pub last_check: DateTime<Utc>,
    pub last_connected: Option<DateTime<Utc>>,
    pub consecutive_failures: u32,
    pub uptime_pct: f64,
}

impl From<&ChannelHealth> for ChannelHealthReport {
    fn from(h: &ChannelHealth) -> Self {
        Self {
            channel_type: h.channel_type.clone(),
            connected: h.connected,
            last_check: h.last_check,
            last_connected: h.last_connected,
            consecutive_failures: h.consecutive_failures,
            uptime_pct: h.uptime_pct(),
        }
    }
}

// ============================================================================
// ChannelHealthMonitor
// ============================================================================

/// Monitors health of all registered channel adapters
pub struct ChannelHealthMonitor {
    /// Per-channel health state
    state: Arc<RwLock<HashMap<String, ChannelHealth>>>,
    /// Configuration
    config: HealthConfig,
}

impl ChannelHealthMonitor {
    /// Create a new health monitor with default config
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
            config: HealthConfig::default(),
        }
    }

    /// Create with custom config
    pub fn with_config(config: HealthConfig) -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    /// Register a channel for monitoring
    pub async fn register(&self, channel_type: &str) {
        let mut state = self.state.write().await;
        state
            .entry(channel_type.to_string())
            .or_insert_with(|| ChannelHealth::new(channel_type));
    }

    /// Record a health check result for a channel
    pub async fn record_check(&self, channel_type: &str, connected: bool) {
        let mut state = self.state.write().await;
        let health = state
            .entry(channel_type.to_string())
            .or_insert_with(|| ChannelHealth::new(channel_type));

        if connected {
            health.record_success();
            debug!("{} health check: connected", channel_type);
        } else {
            health.record_failure();
            if health.consecutive_failures >= self.config.alert_after_failures {
                warn!(
                    "{} has {} consecutive failures — needs attention",
                    channel_type, health.consecutive_failures
                );
            }
        }
    }

    /// Check if a channel needs reconnection
    pub async fn needs_reconnect(&self, channel_type: &str) -> bool {
        let state = self.state.read().await;
        if let Some(health) = state.get(channel_type) {
            !health.connected
                && health.consecutive_failures > 0
                && health.consecutive_failures <= self.config.max_reconnect_attempts
        } else {
            false
        }
    }

    /// Get the current health status of all channels
    pub async fn get_status(&self) -> HealthStatus {
        let state = self.state.read().await;
        let channels: Vec<ChannelHealthReport> =
            state.values().map(ChannelHealthReport::from).collect();

        let connected_count = channels.iter().filter(|c| c.connected).count();
        let total_count = channels.len();

        HealthStatus {
            overall_healthy: connected_count == total_count && total_count > 0,
            connected_count,
            total_count,
            channels,
            checked_at: Utc::now(),
        }
    }

    /// Get health for a specific channel
    pub async fn get_channel_health(&self, channel_type: &str) -> Option<ChannelHealthReport> {
        let state = self.state.read().await;
        state.get(channel_type).map(ChannelHealthReport::from)
    }

    /// Get the monitor config
    pub fn config(&self) -> &HealthConfig {
        &self.config
    }

    /// Get a cloneable handle to the internal state (for use in spawned tasks)
    pub fn state_handle(&self) -> Arc<RwLock<HashMap<String, ChannelHealth>>> {
        Arc::clone(&self.state)
    }
}

impl Default for ChannelHealthMonitor {
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

    #[test]
    fn test_channel_health_new() {
        let h = ChannelHealth::new("telegram");
        assert_eq!(h.channel_type, "telegram");
        assert!(!h.connected);
        assert_eq!(h.consecutive_failures, 0);
        assert_eq!(h.total_checks, 0);
        assert!(h.last_connected.is_none());
    }

    #[test]
    fn test_record_success() {
        let mut h = ChannelHealth::new("discord");
        h.record_success();
        assert!(h.connected);
        assert_eq!(h.consecutive_failures, 0);
        assert_eq!(h.total_checks, 1);
        assert_eq!(h.connected_checks, 1);
        assert!(h.last_connected.is_some());
    }

    #[test]
    fn test_record_failure() {
        let mut h = ChannelHealth::new("slack");
        h.record_failure();
        assert!(!h.connected);
        assert_eq!(h.consecutive_failures, 1);
        assert_eq!(h.total_checks, 1);
        assert_eq!(h.connected_checks, 0);
    }

    #[test]
    fn test_consecutive_failures_reset() {
        let mut h = ChannelHealth::new("telegram");
        h.record_failure();
        h.record_failure();
        assert_eq!(h.consecutive_failures, 2);
        h.record_success();
        assert_eq!(h.consecutive_failures, 0);
    }

    #[test]
    fn test_uptime_pct() {
        let mut h = ChannelHealth::new("matrix");
        h.record_success();
        h.record_success();
        h.record_failure();
        h.record_success();
        // 3 out of 4 connected
        assert!((h.uptime_pct() - 75.0).abs() < 0.01);
    }

    #[test]
    fn test_uptime_pct_zero_checks() {
        let h = ChannelHealth::new("signal");
        assert_eq!(h.uptime_pct(), 0.0);
    }

    #[test]
    fn test_health_config_defaults() {
        let config = HealthConfig::default();
        assert_eq!(config.check_interval_secs, 30);
        assert_eq!(config.max_reconnect_attempts, 3);
        assert_eq!(config.reconnect_delay_secs, 5);
        assert_eq!(config.alert_after_failures, 3);
    }

    #[tokio::test]
    async fn test_monitor_register_and_check() {
        let monitor = ChannelHealthMonitor::new();
        monitor.register("telegram").await;
        monitor.record_check("telegram", true).await;

        let status = monitor.get_status().await;
        assert_eq!(status.total_count, 1);
        assert_eq!(status.connected_count, 1);
        assert!(status.overall_healthy);
    }

    #[tokio::test]
    async fn test_monitor_needs_reconnect() {
        let monitor = ChannelHealthMonitor::new();
        monitor.register("slack").await;

        // Not yet failed — no reconnect needed
        assert!(!monitor.needs_reconnect("slack").await);

        // Fail once — needs reconnect
        monitor.record_check("slack", false).await;
        assert!(monitor.needs_reconnect("slack").await);

        // Recover — no reconnect needed
        monitor.record_check("slack", true).await;
        assert!(!monitor.needs_reconnect("slack").await);
    }

    #[tokio::test]
    async fn test_monitor_max_reconnect_exceeded() {
        let config = HealthConfig {
            max_reconnect_attempts: 2,
            ..Default::default()
        };
        let monitor = ChannelHealthMonitor::with_config(config);
        monitor.register("email").await;

        monitor.record_check("email", false).await;
        monitor.record_check("email", false).await;
        // 2 failures = at max, still true
        assert!(monitor.needs_reconnect("email").await);

        monitor.record_check("email", false).await;
        // 3 failures > max 2, gives up
        assert!(!monitor.needs_reconnect("email").await);
    }

    #[tokio::test]
    async fn test_monitor_multiple_channels() {
        let monitor = ChannelHealthMonitor::new();
        monitor.register("telegram").await;
        monitor.register("discord").await;
        monitor.register("slack").await;

        monitor.record_check("telegram", true).await;
        monitor.record_check("discord", true).await;
        monitor.record_check("slack", false).await;

        let status = monitor.get_status().await;
        assert_eq!(status.total_count, 3);
        assert_eq!(status.connected_count, 2);
        assert!(!status.overall_healthy);
    }

    #[tokio::test]
    async fn test_get_channel_health() {
        let monitor = ChannelHealthMonitor::new();
        monitor.register("telegram").await;
        monitor.record_check("telegram", true).await;

        let health = monitor.get_channel_health("telegram").await;
        assert!(health.is_some());
        assert!(health.expect("operation should succeed").connected);

        let missing = monitor.get_channel_health("nonexistent").await;
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn test_auto_register_on_check() {
        let monitor = ChannelHealthMonitor::new();
        // record_check without prior register should auto-create
        monitor.record_check("whatsapp", true).await;

        let health = monitor.get_channel_health("whatsapp").await;
        assert!(health.is_some());
        assert!(health.expect("operation should succeed").connected);
    }

    #[test]
    fn test_health_report_from_channel_health() {
        let mut h = ChannelHealth::new("nostr");
        h.record_success();
        h.record_success();
        h.record_failure();

        let report = ChannelHealthReport::from(&h);
        assert_eq!(report.channel_type, "nostr");
        assert!(!report.connected);
        assert_eq!(report.consecutive_failures, 1);
        assert!((report.uptime_pct - 66.66).abs() < 1.0);
    }

    #[test]
    fn test_health_status_serde() {
        let status = HealthStatus {
            overall_healthy: true,
            connected_count: 2,
            total_count: 2,
            channels: vec![],
            checked_at: Utc::now(),
        };
        let json = serde_json::to_string(&status).expect("should serialize to JSON");
        let back: HealthStatus = serde_json::from_str(&json).expect("should parse successfully");
        assert!(back.overall_healthy);
        assert_eq!(back.connected_count, 2);
    }
}
