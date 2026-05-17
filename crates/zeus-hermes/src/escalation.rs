//! Notification Escalation — Time-based escalation for unacknowledged alerts
//!
//! When a critical notification isn't acknowledged within a deadline,
//! escalation rules automatically promote it to higher-priority channels
//! or additional recipients.
//!
//! ```text
//! Alert (Telegram) → [30s no ack] → Escalate (Telegram + Email) → [60s] → Escalate (All channels)
//! ```
//!
//! Features:
//! - Multi-level escalation chains with configurable timeouts
//! - Acknowledgment tracking (mark alerts as ack'd to stop escalation)
//! - Repeat suppression (don't re-escalate for same alert)
//! - Escalation policies per alert category

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, warn};

// ============================================================================
// Configuration
// ============================================================================

/// Escalation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationConfig {
    /// Enable escalation system
    pub enabled: bool,
    /// Default escalation chain (used when no category-specific chain exists)
    pub default_chain: EscalationChain,
    /// Per-category escalation chains
    pub category_chains: HashMap<String, EscalationChain>,
    /// Maximum escalation level (prevents infinite escalation)
    pub max_level: u32,
}

impl Default for EscalationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_chain: EscalationChain::default(),
            category_chains: HashMap::new(),
            max_level: 3,
        }
    }
}

/// An escalation chain — ordered list of escalation levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationChain {
    /// Ordered escalation levels
    pub levels: Vec<EscalationLevel>,
}

impl Default for EscalationChain {
    fn default() -> Self {
        Self {
            levels: vec![
                EscalationLevel {
                    timeout_secs: 30,
                    channels: vec!["telegram".into()],
                    message_prefix: "[ALERT]".into(),
                },
                EscalationLevel {
                    timeout_secs: 60,
                    channels: vec!["telegram".into(), "email".into()],
                    message_prefix: "[ESCALATED]".into(),
                },
                EscalationLevel {
                    timeout_secs: 120,
                    channels: vec!["telegram".into(), "email".into(), "discord".into()],
                    message_prefix: "[CRITICAL-ESCALATION]".into(),
                },
            ],
        }
    }
}

/// A single escalation level
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationLevel {
    /// Seconds to wait at this level before escalating further
    pub timeout_secs: i64,
    /// Channels to notify at this level
    pub channels: Vec<String>,
    /// Prefix added to the notification message
    pub message_prefix: String,
}

// ============================================================================
// Alert Tracking
// ============================================================================

/// A tracked alert that may need escalation
#[derive(Debug, Clone)]
struct TrackedAlert {
    id: String,
    category: String,
    message: String,
    created_at: DateTime<Utc>,
    current_level: u32,
    last_escalated_at: DateTime<Utc>,
    acknowledged: bool,
    suppressed: bool,
}

/// Escalation action to take
#[derive(Debug, Clone, PartialEq)]
pub enum EscalationAction {
    /// No action needed (not due, acknowledged, or suppressed)
    None,
    /// Escalate to the specified level with these channels
    Escalate {
        alert_id: String,
        level: u32,
        channels: Vec<String>,
        message: String,
    },
    /// Alert has exhausted all escalation levels
    MaxLevelReached { alert_id: String },
}

// ============================================================================
// Escalation Manager
// ============================================================================

/// Manages escalation for tracked alerts
pub struct EscalationManager {
    config: EscalationConfig,
    alerts: HashMap<String, TrackedAlert>,
    stats: EscalationStats,
}

/// Escalation statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EscalationStats {
    pub total_alerts: u64,
    pub total_escalations: u64,
    pub total_acknowledged: u64,
    pub total_suppressed: u64,
    pub total_max_level_reached: u64,
    pub active_alerts: u64,
}

impl EscalationManager {
    pub fn new(config: EscalationConfig) -> Self {
        Self {
            config,
            alerts: HashMap::new(),
            stats: EscalationStats::default(),
        }
    }

    /// Register a new alert for escalation tracking
    pub fn track_alert(&mut self, alert_id: &str, category: &str, message: &str) {
        if !self.config.enabled {
            return;
        }

        let now = Utc::now();
        self.alerts.insert(
            alert_id.to_string(),
            TrackedAlert {
                id: alert_id.to_string(),
                category: category.to_string(),
                message: message.to_string(),
                created_at: now,
                current_level: 0,
                last_escalated_at: now,
                acknowledged: false,
                suppressed: false,
            },
        );
        self.stats.total_alerts += 1;
        self.stats.active_alerts += 1;
        debug!(alert_id = %alert_id, category = %category, "Tracking alert for escalation");
    }

    /// Acknowledge an alert (stops further escalation)
    pub fn acknowledge(&mut self, alert_id: &str) -> bool {
        if let Some(alert) = self.alerts.get_mut(alert_id)
            && !alert.acknowledged
        {
            alert.acknowledged = true;
            self.stats.total_acknowledged += 1;
            self.stats.active_alerts = self.stats.active_alerts.saturating_sub(1);
            debug!(alert_id = %alert_id, "Alert acknowledged");
            return true;
        }
        false
    }

    /// Suppress an alert (stops escalation without acknowledging)
    pub fn suppress(&mut self, alert_id: &str) -> bool {
        if let Some(alert) = self.alerts.get_mut(alert_id)
            && !alert.suppressed
        {
            alert.suppressed = true;
            self.stats.total_suppressed += 1;
            self.stats.active_alerts = self.stats.active_alerts.saturating_sub(1);
            debug!(alert_id = %alert_id, "Alert suppressed");
            return true;
        }
        false
    }

    /// Check all tracked alerts and return actions needed
    pub fn check_escalations(&mut self) -> Vec<EscalationAction> {
        if !self.config.enabled {
            return Vec::new();
        }

        let now = Utc::now();
        let max_level = self.config.max_level;
        let mut actions = Vec::new();

        // Collect alert IDs and their escalation info to avoid borrow issues
        type AlertCheck = (String, String, String, u32, DateTime<Utc>, bool, bool);
        let alert_checks: Vec<AlertCheck> = self
            .alerts
            .values()
            .map(|a| {
                (
                    a.id.clone(),
                    a.category.clone(),
                    a.message.clone(),
                    a.current_level,
                    a.last_escalated_at,
                    a.acknowledged,
                    a.suppressed,
                )
            })
            .collect();

        for (id, category, message, level, last_escalated, ack, suppressed) in alert_checks {
            if ack || suppressed {
                continue;
            }

            let chain = self
                .config
                .category_chains
                .get(&category)
                .unwrap_or(&self.config.default_chain);

            let next_level = level + 1;

            if next_level > max_level || next_level as usize > chain.levels.len() {
                if level < max_level && (level as usize) < chain.levels.len() {
                    // Not yet at max — check timeout for current level
                } else {
                    // Already at max
                    continue;
                }
            }

            // Get the current level's timeout
            let level_idx = if level == 0 {
                0
            } else {
                (level as usize).min(chain.levels.len() - 1)
            };
            if level_idx >= chain.levels.len() {
                continue;
            }

            let timeout = Duration::seconds(chain.levels[level_idx].timeout_secs);
            let elapsed = now - last_escalated;

            if elapsed >= timeout {
                // Time to escalate
                let target_idx = (next_level as usize).min(chain.levels.len()) - 1;
                if target_idx < chain.levels.len() {
                    let target_level = &chain.levels[target_idx.min(chain.levels.len() - 1)];
                    let escalated_message = format!("{} {}", target_level.message_prefix, message);

                    actions.push(EscalationAction::Escalate {
                        alert_id: id.clone(),
                        level: next_level,
                        channels: target_level.channels.clone(),
                        message: escalated_message,
                    });
                } else if next_level > max_level {
                    actions.push(EscalationAction::MaxLevelReached {
                        alert_id: id.clone(),
                    });
                }
            }
        }

        // Apply escalation state updates
        for action in &actions {
            match action {
                EscalationAction::Escalate {
                    alert_id, level, ..
                } => {
                    if let Some(alert) = self.alerts.get_mut(alert_id) {
                        alert.current_level = *level;
                        alert.last_escalated_at = now;
                        self.stats.total_escalations += 1;
                        warn!(alert_id = %alert_id, level = %level, "Alert escalated");
                    }
                }
                EscalationAction::MaxLevelReached { alert_id } => {
                    if let Some(alert) = self.alerts.get_mut(alert_id) {
                        alert.suppressed = true;
                        self.stats.total_max_level_reached += 1;
                        self.stats.active_alerts = self.stats.active_alerts.saturating_sub(1);
                        warn!(alert_id = %alert_id, "Alert hit max escalation level");
                    }
                }
                EscalationAction::None => {}
            }
        }

        actions
    }

    /// Get current state of a tracked alert
    pub fn alert_status(&self, alert_id: &str) -> Option<AlertStatus> {
        self.alerts.get(alert_id).map(|a| AlertStatus {
            id: a.id.clone(),
            category: a.category.clone(),
            current_level: a.current_level,
            acknowledged: a.acknowledged,
            suppressed: a.suppressed,
            created_at: a.created_at,
            last_escalated_at: a.last_escalated_at,
        })
    }

    /// Get all active (unacknowledged, unsuppressed) alert IDs
    pub fn active_alerts(&self) -> Vec<String> {
        self.alerts
            .values()
            .filter(|a| !a.acknowledged && !a.suppressed)
            .map(|a| a.id.clone())
            .collect()
    }

    /// Remove resolved alerts (acknowledged or suppressed)
    pub fn cleanup_resolved(&mut self) -> usize {
        let before = self.alerts.len();
        self.alerts.retain(|_, a| !a.acknowledged && !a.suppressed);
        before - self.alerts.len()
    }

    /// Get statistics
    pub fn stats(&self) -> &EscalationStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = EscalationStats::default();
    }

    /// Get config
    pub fn config(&self) -> &EscalationConfig {
        &self.config
    }

    /// Update config
    pub fn set_config(&mut self, config: EscalationConfig) {
        self.config = config;
    }

    /// Total tracked alerts (including resolved)
    pub fn tracked_count(&self) -> usize {
        self.alerts.len()
    }
}

impl Default for EscalationManager {
    fn default() -> Self {
        Self::new(EscalationConfig::default())
    }
}

// ============================================================================
// Alert Status
// ============================================================================

/// Public-facing alert status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertStatus {
    pub id: String,
    pub category: String,
    pub current_level: u32,
    pub acknowledged: bool,
    pub suppressed: bool,
    pub created_at: DateTime<Utc>,
    pub last_escalated_at: DateTime<Utc>,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> EscalationConfig {
        EscalationConfig {
            enabled: true,
            default_chain: EscalationChain {
                levels: vec![
                    EscalationLevel {
                        timeout_secs: 10,
                        channels: vec!["telegram".into()],
                        message_prefix: "[L1]".into(),
                    },
                    EscalationLevel {
                        timeout_secs: 20,
                        channels: vec!["telegram".into(), "email".into()],
                        message_prefix: "[L2]".into(),
                    },
                    EscalationLevel {
                        timeout_secs: 30,
                        channels: vec!["telegram".into(), "email".into(), "discord".into()],
                        message_prefix: "[L3]".into(),
                    },
                ],
            },
            category_chains: HashMap::new(),
            max_level: 3,
        }
    }

    fn test_manager() -> EscalationManager {
        EscalationManager::new(test_config())
    }

    #[test]
    fn test_track_alert() {
        let mut mgr = test_manager();
        mgr.track_alert("alert-1", "security", "Unauthorized access detected");
        assert_eq!(mgr.tracked_count(), 1);
        assert_eq!(mgr.stats().total_alerts, 1);
        assert_eq!(mgr.stats().active_alerts, 1);
    }

    #[test]
    fn test_acknowledge_stops_escalation() {
        let mut mgr = test_manager();
        mgr.track_alert("alert-1", "security", "Test alert");
        assert!(mgr.acknowledge("alert-1"));
        assert_eq!(mgr.stats().total_acknowledged, 1);
        assert_eq!(mgr.stats().active_alerts, 0);

        // Check escalations returns nothing for ack'd alerts
        let actions = mgr.check_escalations();
        assert!(actions.is_empty());
    }

    #[test]
    fn test_acknowledge_nonexistent() {
        let mut mgr = test_manager();
        assert!(!mgr.acknowledge("nonexistent"));
    }

    #[test]
    fn test_double_acknowledge() {
        let mut mgr = test_manager();
        mgr.track_alert("alert-1", "security", "Test");
        assert!(mgr.acknowledge("alert-1"));
        assert!(!mgr.acknowledge("alert-1")); // second time returns false
        assert_eq!(mgr.stats().total_acknowledged, 1);
    }

    #[test]
    fn test_suppress_alert() {
        let mut mgr = test_manager();
        mgr.track_alert("alert-1", "error", "Something broke");
        assert!(mgr.suppress("alert-1"));
        assert_eq!(mgr.stats().total_suppressed, 1);
        assert_eq!(mgr.stats().active_alerts, 0);
    }

    #[test]
    fn test_no_escalation_before_timeout() {
        let mut mgr = test_manager();
        mgr.track_alert("alert-1", "security", "Test alert");
        // Immediately check — should not escalate yet
        let actions = mgr.check_escalations();
        assert!(actions.is_empty());
    }

    #[test]
    fn test_escalation_after_timeout() {
        let mut mgr = test_manager();
        mgr.track_alert("alert-1", "security", "Breach detected");

        // Backdate the alert to simulate timeout
        if let Some(alert) = mgr.alerts.get_mut("alert-1") {
            alert.last_escalated_at = Utc::now() - Duration::seconds(15);
        }

        let actions = mgr.check_escalations();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            EscalationAction::Escalate {
                alert_id,
                level,
                channels,
                message,
            } => {
                assert_eq!(alert_id, "alert-1");
                assert_eq!(*level, 1);
                assert!(channels.contains(&"telegram".to_string()));
                assert!(message.contains("[L1]"));
                assert!(message.contains("Breach detected"));
            }
            _ => panic!("Expected Escalate action"),
        }
    }

    #[test]
    fn test_multi_level_escalation() {
        let mut mgr = test_manager();
        mgr.track_alert("alert-1", "security", "Critical issue");

        // Level 1 escalation
        if let Some(alert) = mgr.alerts.get_mut("alert-1") {
            alert.last_escalated_at = Utc::now() - Duration::seconds(15);
        }
        let actions = mgr.check_escalations();
        assert_eq!(actions.len(), 1);

        // Level 2 escalation
        if let Some(alert) = mgr.alerts.get_mut("alert-1") {
            alert.last_escalated_at = Utc::now() - Duration::seconds(25);
        }
        let actions = mgr.check_escalations();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            EscalationAction::Escalate {
                level, channels, ..
            } => {
                assert_eq!(*level, 2);
                assert!(channels.contains(&"email".to_string()));
            }
            _ => panic!("Expected Escalate"),
        }
    }

    #[test]
    fn test_active_alerts() {
        let mut mgr = test_manager();
        mgr.track_alert("a1", "security", "Alert 1");
        mgr.track_alert("a2", "error", "Alert 2");
        mgr.track_alert("a3", "warning", "Alert 3");

        assert_eq!(mgr.active_alerts().len(), 3);

        mgr.acknowledge("a1");
        assert_eq!(mgr.active_alerts().len(), 2);

        mgr.suppress("a2");
        assert_eq!(mgr.active_alerts().len(), 1);
    }

    #[test]
    fn test_cleanup_resolved() {
        let mut mgr = test_manager();
        mgr.track_alert("a1", "security", "Alert 1");
        mgr.track_alert("a2", "error", "Alert 2");
        mgr.acknowledge("a1");

        let removed = mgr.cleanup_resolved();
        assert_eq!(removed, 1);
        assert_eq!(mgr.tracked_count(), 1);
    }

    #[test]
    fn test_alert_status() {
        let mut mgr = test_manager();
        mgr.track_alert("alert-1", "security", "Test");

        let status = mgr.alert_status("alert-1").unwrap();
        assert_eq!(status.id, "alert-1");
        assert_eq!(status.category, "security");
        assert_eq!(status.current_level, 0);
        assert!(!status.acknowledged);
        assert!(!status.suppressed);
    }

    #[test]
    fn test_alert_status_nonexistent() {
        let mgr = test_manager();
        assert!(mgr.alert_status("nonexistent").is_none());
    }

    #[test]
    fn test_disabled_manager() {
        let mut mgr = EscalationManager::new(EscalationConfig {
            enabled: false,
            ..Default::default()
        });
        mgr.track_alert("alert-1", "security", "Test");
        assert_eq!(mgr.tracked_count(), 0); // not tracked when disabled

        let actions = mgr.check_escalations();
        assert!(actions.is_empty());
    }

    #[test]
    fn test_category_specific_chain() {
        let mut config = test_config();
        config.category_chains.insert(
            "billing".into(),
            EscalationChain {
                levels: vec![EscalationLevel {
                    timeout_secs: 5,
                    channels: vec!["email".into()],
                    message_prefix: "[BILLING]".into(),
                }],
            },
        );

        let mut mgr = EscalationManager::new(config);
        mgr.track_alert("billing-1", "billing", "Payment failed");

        if let Some(alert) = mgr.alerts.get_mut("billing-1") {
            alert.last_escalated_at = Utc::now() - Duration::seconds(10);
        }

        let actions = mgr.check_escalations();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            EscalationAction::Escalate {
                channels, message, ..
            } => {
                assert!(channels.contains(&"email".to_string()));
                assert!(message.contains("[BILLING]"));
            }
            _ => panic!("Expected Escalate"),
        }
    }

    #[test]
    fn test_stats_tracking() {
        let mut mgr = test_manager();
        mgr.track_alert("a1", "sec", "Alert 1");
        mgr.track_alert("a2", "sec", "Alert 2");
        mgr.acknowledge("a1");

        let stats = mgr.stats();
        assert_eq!(stats.total_alerts, 2);
        assert_eq!(stats.total_acknowledged, 1);
        assert_eq!(stats.active_alerts, 1);
    }

    #[test]
    fn test_reset_stats() {
        let mut mgr = test_manager();
        mgr.track_alert("a1", "sec", "Alert");
        mgr.reset_stats();
        assert_eq!(mgr.stats().total_alerts, 0);
    }

    #[test]
    fn test_config_update() {
        let mut mgr = test_manager();
        mgr.set_config(EscalationConfig {
            max_level: 5,
            ..Default::default()
        });
        assert_eq!(mgr.config().max_level, 5);
    }

    #[test]
    fn test_default_manager() {
        let mgr = EscalationManager::default();
        assert!(mgr.config().enabled);
        assert_eq!(mgr.config().max_level, 3);
        assert_eq!(mgr.tracked_count(), 0);
    }

    #[test]
    fn test_escalation_stats_count() {
        let mut mgr = test_manager();
        mgr.track_alert("a1", "sec", "Alert");

        if let Some(alert) = mgr.alerts.get_mut("a1") {
            alert.last_escalated_at = Utc::now() - Duration::seconds(15);
        }
        mgr.check_escalations();
        assert_eq!(mgr.stats().total_escalations, 1);
    }

    #[test]
    fn test_independent_alerts() {
        let mut mgr = test_manager();
        mgr.track_alert("a1", "security", "Security alert");
        mgr.track_alert("a2", "error", "Error alert");

        mgr.acknowledge("a1");
        assert!(mgr.alert_status("a1").unwrap().acknowledged);
        assert!(!mgr.alert_status("a2").unwrap().acknowledged);
    }
}
