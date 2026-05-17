//! Survival Tiers — Resource-aware agent behavior degradation
//!
//! Agents monitor their own resource usage (context window %, wallet balance,
//! error rate) and transition between survival tiers that gate capabilities.
//!
//! Conway/Web 4.0: agents that can't sustain themselves gracefully degrade
//! and eventually die, freeing resources for fitter agents.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tracing::{debug, info, warn};

// ============================================================================
// Survival Tier
// ============================================================================

/// Resource-aware survival tiers inspired by Conway's Web 4.0 paper.
///
/// Agents transition between tiers based on their resource state.
/// Each tier gates a different set of capabilities.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub enum SurvivalTier {
    /// Full capability — all tools available, normal heartbeat rate
    #[default]
    Normal,
    /// Low on resources — disable expensive tools (browser, voice, spawn)
    LowCompute,
    /// Critical — save state, request help, minimal operation
    Critical,
    /// Insolvent / exhausted — graceful shutdown
    Dead,
}

impl std::fmt::Display for SurvivalTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal"),
            Self::LowCompute => write!(f, "LowCompute"),
            Self::Critical => write!(f, "Critical"),
            Self::Dead => write!(f, "Dead"),
        }
    }
}

// ============================================================================
// Thresholds Configuration
// ============================================================================

/// Thresholds that govern tier transitions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurvivalThresholds {
    /// Context window % below which we enter LowCompute (default: 40%)
    pub context_low_pct: f64,
    /// Context window % below which we enter Critical (default: 15%)
    pub context_critical_pct: f64,

    /// Wallet balance below which we enter LowCompute (default: 1000 credits)
    pub balance_low: u64,
    /// Wallet balance below which we enter Critical (default: 100 credits)
    pub balance_critical: u64,

    /// Error rate (errors / total calls) above which we enter LowCompute (default: 0.3)
    pub error_rate_low: f64,
    /// Error rate above which we enter Critical (default: 0.6)
    pub error_rate_critical: f64,

    /// Rolling window size for error rate calculation (default: 50)
    pub error_window: usize,
}

impl Default for SurvivalThresholds {
    fn default() -> Self {
        Self {
            context_low_pct: 40.0,
            context_critical_pct: 15.0,
            balance_low: 1000,
            balance_critical: 100,
            error_rate_low: 0.3,
            error_rate_critical: 0.6,
            error_window: 50,
        }
    }
}

// ============================================================================
// Resource Snapshot
// ============================================================================

/// A point-in-time snapshot of agent resource usage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSnapshot {
    /// Remaining context window percentage (0.0–100.0)
    pub context_remaining_pct: f64,
    /// Current wallet balance in credits
    pub wallet_balance: u64,
    /// Recent error rate (0.0–1.0)
    pub error_rate: f64,
    /// Timestamp of this snapshot
    pub timestamp: DateTime<Utc>,
}

// ============================================================================
// Tier Transition Event
// ============================================================================

/// Emitted when the agent changes survival tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierTransition {
    pub from: SurvivalTier,
    pub to: SurvivalTier,
    pub reason: String,
    pub snapshot: ResourceSnapshot,
    pub timestamp: DateTime<Utc>,
}

// ============================================================================
// Survival Monitor
// ============================================================================

/// Monitors agent resources and computes survival tier.
///
/// Feed it resource updates; it tracks error rates over a rolling window
/// and determines the current tier based on configured thresholds.
pub struct SurvivalMonitor {
    thresholds: SurvivalThresholds,
    current_tier: SurvivalTier,
    /// Rolling window of (success: bool) for error rate
    outcomes: VecDeque<bool>,
    /// Current balance (updated externally)
    wallet_balance: u64,
    /// Current context remaining % (updated externally)
    context_remaining_pct: f64,
    /// History of tier transitions
    transitions: Vec<TierTransition>,
    /// Tools disabled in LowCompute tier
    expensive_tools: Vec<String>,
}

impl SurvivalMonitor {
    /// Create a new monitor with default thresholds
    pub fn new() -> Self {
        Self::with_thresholds(SurvivalThresholds::default())
    }

    /// Create with custom thresholds
    pub fn with_thresholds(thresholds: SurvivalThresholds) -> Self {
        Self {
            thresholds,
            current_tier: SurvivalTier::Normal,
            outcomes: VecDeque::new(),
            wallet_balance: u64::MAX, // assume solvent until told otherwise
            context_remaining_pct: 100.0,
            transitions: Vec::new(),
            expensive_tools: vec![
                "spawn".into(),
                "browser_navigate".into(),
                "browser_click".into(),
                "browser_type".into(),
                "browser_screenshot".into(),
                "voice_call".into(),
                "speak_text".into(),
            ],
        }
    }

    /// Current survival tier
    pub fn tier(&self) -> SurvivalTier {
        self.current_tier
    }

    /// Update context window remaining percentage
    pub fn update_context(&mut self, remaining_pct: f64) {
        self.context_remaining_pct = remaining_pct.clamp(0.0, 100.0);
        self.recompute_tier();
    }

    /// Update wallet balance
    pub fn update_balance(&mut self, balance: u64) {
        self.wallet_balance = balance;
        self.recompute_tier();
    }

    /// Record a tool call outcome (success or failure)
    pub fn record_outcome(&mut self, success: bool) {
        self.outcomes.push_back(success);
        while self.outcomes.len() > self.thresholds.error_window {
            self.outcomes.pop_front();
        }
        self.recompute_tier();
    }

    /// Current error rate over the rolling window (0.0–1.0)
    pub fn error_rate(&self) -> f64 {
        if self.outcomes.is_empty() {
            return 0.0;
        }
        let errors = self.outcomes.iter().filter(|&&ok| !ok).count();
        errors as f64 / self.outcomes.len() as f64
    }

    /// Check whether a tool is allowed at the current tier
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        match self.current_tier {
            SurvivalTier::Normal => true,
            SurvivalTier::LowCompute => !self.expensive_tools.iter().any(|t| t == tool_name),
            SurvivalTier::Critical => {
                // Only allow essential read/write tools
                matches!(
                    tool_name,
                    "read_file" | "write_file" | "edit_file" | "list_dir" | "shell" | "message"
                )
            }
            SurvivalTier::Dead => false,
        }
    }

    /// Get a snapshot of current resources
    pub fn snapshot(&self) -> ResourceSnapshot {
        ResourceSnapshot {
            context_remaining_pct: self.context_remaining_pct,
            wallet_balance: self.wallet_balance,
            error_rate: self.error_rate(),
            timestamp: Utc::now(),
        }
    }

    /// Transition history
    pub fn transitions(&self) -> &[TierTransition] {
        &self.transitions
    }

    /// Override the list of expensive tools disabled in LowCompute
    pub fn set_expensive_tools(&mut self, tools: Vec<String>) {
        self.expensive_tools = tools;
    }

    /// Build a system prompt advisory for the current tier
    pub fn system_advisory(&self) -> Option<String> {
        match self.current_tier {
            SurvivalTier::Normal => None,
            SurvivalTier::LowCompute => Some(format!(
                "[SURVIVAL: LowCompute] Resources declining (context: {:.0}%, balance: {}, errors: {:.0}%). \
                 Expensive tools disabled. Conserve resources.",
                self.context_remaining_pct,
                self.wallet_balance,
                self.error_rate() * 100.0,
            )),
            SurvivalTier::Critical => Some(format!(
                "[SURVIVAL: CRITICAL] Resources critically low (context: {:.0}%, balance: {}, errors: {:.0}%). \
                 Save state immediately. Only essential tools available. Request help if possible.",
                self.context_remaining_pct,
                self.wallet_balance,
                self.error_rate() * 100.0,
            )),
            SurvivalTier::Dead => {
                Some("[SURVIVAL: DEAD] Resources exhausted. Initiating graceful shutdown.".into())
            }
        }
    }

    /// Force a specific tier (for testing or manual override)
    pub fn force_tier(&mut self, tier: SurvivalTier, reason: &str) {
        let old = self.current_tier;
        if old != tier {
            self.record_transition(old, tier, reason.to_string());
        }
        self.current_tier = tier;
    }

    // -- internals --

    fn recompute_tier(&mut self) {
        let error_rate = self.error_rate();
        let old = self.current_tier;

        // Compute tier from each resource dimension, take the worst
        let context_tier = if self.context_remaining_pct <= self.thresholds.context_critical_pct {
            SurvivalTier::Critical
        } else if self.context_remaining_pct <= self.thresholds.context_low_pct {
            SurvivalTier::LowCompute
        } else {
            SurvivalTier::Normal
        };

        let balance_tier = if self.wallet_balance == 0 {
            SurvivalTier::Dead
        } else if self.wallet_balance <= self.thresholds.balance_critical {
            SurvivalTier::Critical
        } else if self.wallet_balance <= self.thresholds.balance_low {
            SurvivalTier::LowCompute
        } else {
            SurvivalTier::Normal
        };

        let error_tier = if error_rate >= self.thresholds.error_rate_critical {
            SurvivalTier::Critical
        } else if error_rate >= self.thresholds.error_rate_low {
            SurvivalTier::LowCompute
        } else {
            SurvivalTier::Normal
        };

        // Worst tier wins (higher ordinal = worse)
        let new = context_tier.max(balance_tier).max(error_tier);

        if new != old {
            let reason = self.describe_transition(context_tier, balance_tier, error_tier);
            self.record_transition(old, new, reason);
            self.current_tier = new;
        }
    }

    fn describe_transition(
        &self,
        context_tier: SurvivalTier,
        balance_tier: SurvivalTier,
        error_tier: SurvivalTier,
    ) -> String {
        let mut reasons = Vec::new();
        if context_tier > SurvivalTier::Normal {
            reasons.push(format!("context={:.0}%", self.context_remaining_pct));
        }
        if balance_tier > SurvivalTier::Normal {
            reasons.push(format!("balance={}", self.wallet_balance));
        }
        if error_tier > SurvivalTier::Normal {
            reasons.push(format!("errors={:.0}%", self.error_rate() * 100.0));
        }
        if reasons.is_empty() {
            "resources recovered".into()
        } else {
            reasons.join(", ")
        }
    }

    fn record_transition(&mut self, from: SurvivalTier, to: SurvivalTier, reason: String) {
        let snapshot = self.snapshot();
        let direction = if to > from { "degraded" } else { "recovered" };

        match to {
            SurvivalTier::Dead => warn!(%from, %to, %reason, "Agent survival: DEAD"),
            SurvivalTier::Critical => warn!(%from, %to, %reason, "Agent survival: {direction}"),
            _ => info!(%from, %to, %reason, "Agent survival: {direction}"),
        }
        debug!(?snapshot, "Survival snapshot at transition");

        self.transitions.push(TierTransition {
            from,
            to,
            reason,
            snapshot,
            timestamp: Utc::now(),
        });
    }
}

impl Default for SurvivalMonitor {
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
    fn test_default_tier_is_normal() {
        let monitor = SurvivalMonitor::new();
        assert_eq!(monitor.tier(), SurvivalTier::Normal);
    }

    #[test]
    fn test_context_triggers_low_compute() {
        let mut monitor = SurvivalMonitor::new();
        monitor.update_context(35.0);
        assert_eq!(monitor.tier(), SurvivalTier::LowCompute);
    }

    #[test]
    fn test_context_triggers_critical() {
        let mut monitor = SurvivalMonitor::new();
        monitor.update_context(10.0);
        assert_eq!(monitor.tier(), SurvivalTier::Critical);
    }

    #[test]
    fn test_balance_triggers_low_compute() {
        let mut monitor = SurvivalMonitor::new();
        monitor.update_balance(500);
        assert_eq!(monitor.tier(), SurvivalTier::LowCompute);
    }

    #[test]
    fn test_balance_zero_triggers_dead() {
        let mut monitor = SurvivalMonitor::new();
        monitor.update_balance(0);
        assert_eq!(monitor.tier(), SurvivalTier::Dead);
    }

    #[test]
    fn test_error_rate_triggers_low_compute() {
        let mut monitor = SurvivalMonitor::new();
        for _ in 0..35 {
            monitor.record_outcome(true);
        }
        for _ in 0..15 {
            monitor.record_outcome(false);
        }
        assert_eq!(monitor.tier(), SurvivalTier::LowCompute);
    }

    #[test]
    fn test_error_rate_triggers_critical() {
        let mut monitor = SurvivalMonitor::new();
        for _ in 0..15 {
            monitor.record_outcome(true);
        }
        for _ in 0..35 {
            monitor.record_outcome(false);
        }
        assert_eq!(monitor.tier(), SurvivalTier::Critical);
    }

    #[test]
    fn test_worst_dimension_wins() {
        let mut monitor = SurvivalMonitor::new();
        monitor.update_context(80.0);
        monitor.update_balance(50);
        assert_eq!(monitor.tier(), SurvivalTier::Critical);
    }

    #[test]
    fn test_recovery_to_normal() {
        let mut monitor = SurvivalMonitor::new();
        monitor.update_balance(50);
        assert_eq!(monitor.tier(), SurvivalTier::Critical);
        monitor.update_balance(5000);
        assert_eq!(monitor.tier(), SurvivalTier::Normal);
        assert_eq!(monitor.transitions().len(), 2);
    }

    #[test]
    fn test_tool_gating_normal() {
        let monitor = SurvivalMonitor::new();
        assert!(monitor.is_tool_allowed("spawn"));
        assert!(monitor.is_tool_allowed("read_file"));
        assert!(monitor.is_tool_allowed("browser_navigate"));
    }

    #[test]
    fn test_tool_gating_low_compute() {
        let mut monitor = SurvivalMonitor::new();
        monitor.update_context(30.0);
        assert!(!monitor.is_tool_allowed("spawn"));
        assert!(!monitor.is_tool_allowed("browser_navigate"));
        assert!(!monitor.is_tool_allowed("voice_call"));
        assert!(monitor.is_tool_allowed("read_file"));
        assert!(monitor.is_tool_allowed("shell"));
    }

    #[test]
    fn test_tool_gating_critical() {
        let mut monitor = SurvivalMonitor::new();
        monitor.update_context(5.0);
        assert!(monitor.is_tool_allowed("read_file"));
        assert!(monitor.is_tool_allowed("write_file"));
        assert!(monitor.is_tool_allowed("message"));
        assert!(!monitor.is_tool_allowed("spawn"));
        assert!(!monitor.is_tool_allowed("web_fetch"));
    }

    #[test]
    fn test_tool_gating_dead() {
        let mut monitor = SurvivalMonitor::new();
        monitor.update_balance(0);
        assert!(!monitor.is_tool_allowed("read_file"));
        assert!(!monitor.is_tool_allowed("shell"));
    }

    #[test]
    fn test_system_advisory() {
        let monitor = SurvivalMonitor::new();
        assert!(monitor.system_advisory().is_none());

        let mut monitor = SurvivalMonitor::new();
        monitor.update_context(30.0);
        let advisory = monitor.system_advisory().unwrap();
        assert!(advisory.contains("LowCompute"));
    }

    #[test]
    fn test_force_tier() {
        let mut monitor = SurvivalMonitor::new();
        monitor.force_tier(SurvivalTier::Critical, "manual override");
        assert_eq!(monitor.tier(), SurvivalTier::Critical);
        assert_eq!(monitor.transitions().len(), 1);
        assert!(monitor.transitions()[0].reason.contains("manual override"));
    }

    #[test]
    fn test_error_window_rolling() {
        let mut monitor = SurvivalMonitor::with_thresholds(SurvivalThresholds {
            error_window: 10,
            ..Default::default()
        });
        for _ in 0..10 {
            monitor.record_outcome(false);
        }
        assert!((monitor.error_rate() - 1.0).abs() < f64::EPSILON);
        for _ in 0..10 {
            monitor.record_outcome(true);
        }
        assert!(monitor.error_rate() < f64::EPSILON);
    }

    #[test]
    fn test_snapshot() {
        let mut monitor = SurvivalMonitor::new();
        monitor.update_context(55.0);
        monitor.update_balance(2000);
        monitor.record_outcome(true);
        monitor.record_outcome(false);
        let snap = monitor.snapshot();
        assert!((snap.context_remaining_pct - 55.0).abs() < f64::EPSILON);
        assert_eq!(snap.wallet_balance, 2000);
        assert!((snap.error_rate - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_tier_ordering() {
        assert!(SurvivalTier::Dead > SurvivalTier::Critical);
        assert!(SurvivalTier::Critical > SurvivalTier::LowCompute);
        assert!(SurvivalTier::LowCompute > SurvivalTier::Normal);
    }
}
