//! Circuit Breaker — Resilience pattern for channel adapters
//!
//! Prevents cascading failures by tracking error rates per channel and
//! transitioning through states:
//!
//! ```text
//! Closed (healthy) → Open (failing) → HalfOpen (probing) → Closed
//! ```
//!
//! When a channel accumulates too many failures within a time window,
//! the breaker "opens" — blocking further requests until a cooldown
//! expires. After cooldown, it enters "half-open" and allows a single
//! probe request. If the probe succeeds, the breaker closes; if it
//! fails, it reopens with an exponential backoff.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, warn};

// ============================================================================
// Configuration
// ============================================================================

/// Circuit breaker configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening the circuit
    pub failure_threshold: u32,
    /// Time window for counting failures (seconds)
    pub failure_window_secs: i64,
    /// Base cooldown before transitioning to half-open (seconds)
    pub cooldown_secs: i64,
    /// Maximum cooldown after exponential backoff (seconds)
    pub max_cooldown_secs: i64,
    /// Number of successful probes in half-open before closing
    pub success_threshold: u32,
    /// Exponential backoff multiplier on repeated opens
    pub backoff_multiplier: f64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            failure_window_secs: 60,
            cooldown_secs: 30,
            max_cooldown_secs: 600,
            success_threshold: 2,
            backoff_multiplier: 2.0,
        }
    }
}

// ============================================================================
// Circuit State
// ============================================================================

/// The three states of a circuit breaker
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircuitState {
    /// Normal operation — requests flow through
    Closed,
    /// Failures exceeded threshold — requests are blocked
    Open,
    /// Cooldown expired — allowing probe requests
    HalfOpen,
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "closed"),
            Self::Open => write!(f, "open"),
            Self::HalfOpen => write!(f, "half-open"),
        }
    }
}

// ============================================================================
// Per-Channel Circuit
// ============================================================================

/// Per-channel circuit breaker state
#[derive(Debug, Clone)]
struct ChannelCircuit {
    state: CircuitState,
    /// Recent failure timestamps (within window)
    failures: Vec<DateTime<Utc>>,
    /// When the circuit was last opened
    opened_at: Option<DateTime<Utc>>,
    /// Current cooldown duration (increases with backoff)
    current_cooldown_secs: i64,
    /// Consecutive successful probes in half-open state
    half_open_successes: u32,
    /// Total times this circuit has opened
    total_opens: u64,
    /// Total failures recorded
    total_failures: u64,
    /// Total successes recorded
    total_successes: u64,
    /// Last state transition time
    last_transition: DateTime<Utc>,
}

impl ChannelCircuit {
    fn new() -> Self {
        Self {
            state: CircuitState::Closed,
            failures: Vec::new(),
            opened_at: None,
            current_cooldown_secs: 0,
            half_open_successes: 0,
            total_opens: 0,
            total_failures: 0,
            total_successes: 0,
            last_transition: Utc::now(),
        }
    }
}

// ============================================================================
// Request Verdict
// ============================================================================

/// Whether a request should be allowed through
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestVerdict {
    /// Request is allowed
    Allow,
    /// Request is blocked (circuit is open)
    Block {
        /// When the circuit will transition to half-open
        retry_after_secs: i64,
    },
    /// Request is allowed as a probe (circuit is half-open)
    Probe,
}

impl RequestVerdict {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow | Self::Probe)
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Block { .. })
    }
}

// ============================================================================
// Circuit Breaker Manager
// ============================================================================

/// Manages circuit breakers for all channels
pub struct CircuitBreakerManager {
    config: CircuitBreakerConfig,
    circuits: HashMap<String, ChannelCircuit>,
}

impl CircuitBreakerManager {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            circuits: HashMap::new(),
        }
    }

    /// Check if a request to a channel should be allowed
    pub fn check(&mut self, channel_id: &str) -> RequestVerdict {
        let circuit = self
            .circuits
            .entry(channel_id.to_string())
            .or_insert_with(ChannelCircuit::new);

        match circuit.state {
            CircuitState::Closed => RequestVerdict::Allow,
            CircuitState::Open => {
                // Check if cooldown has expired
                if let Some(opened_at) = circuit.opened_at {
                    let elapsed = (Utc::now() - opened_at).num_seconds();
                    if elapsed >= circuit.current_cooldown_secs {
                        // Transition to half-open
                        circuit.state = CircuitState::HalfOpen;
                        circuit.half_open_successes = 0;
                        circuit.last_transition = Utc::now();
                        debug!(channel = %channel_id, "Circuit breaker → half-open");
                        return RequestVerdict::Probe;
                    }
                    let remaining = circuit.current_cooldown_secs - elapsed;
                    RequestVerdict::Block {
                        retry_after_secs: remaining,
                    }
                } else {
                    // Shouldn't happen, but treat as half-open
                    circuit.state = CircuitState::HalfOpen;
                    circuit.last_transition = Utc::now();
                    RequestVerdict::Probe
                }
            }
            CircuitState::HalfOpen => RequestVerdict::Probe,
        }
    }

    /// Record a successful request
    pub fn record_success(&mut self, channel_id: &str) {
        let config = &self.config;
        let circuit = self
            .circuits
            .entry(channel_id.to_string())
            .or_insert_with(ChannelCircuit::new);

        circuit.total_successes += 1;

        match circuit.state {
            CircuitState::HalfOpen => {
                circuit.half_open_successes += 1;
                if circuit.half_open_successes >= config.success_threshold {
                    // Close the circuit — channel is healthy again
                    circuit.state = CircuitState::Closed;
                    circuit.failures.clear();
                    circuit.opened_at = None;
                    circuit.current_cooldown_secs = 0;
                    circuit.half_open_successes = 0;
                    circuit.last_transition = Utc::now();
                    debug!(channel = %channel_id, "Circuit breaker → closed (recovered)");
                }
            }
            CircuitState::Closed => {
                // Normal operation, nothing to do
            }
            CircuitState::Open => {
                // Shouldn't get successes while open, but record anyway
            }
        }
    }

    /// Record a failed request
    pub fn record_failure(&mut self, channel_id: &str) {
        let config_threshold = self.config.failure_threshold;
        let config_window = self.config.failure_window_secs;
        let config_cooldown = self.config.cooldown_secs;
        let config_max_cooldown = self.config.max_cooldown_secs;
        let config_backoff = self.config.backoff_multiplier;

        let circuit = self
            .circuits
            .entry(channel_id.to_string())
            .or_insert_with(ChannelCircuit::new);

        circuit.total_failures += 1;

        match circuit.state {
            CircuitState::Closed => {
                let now = Utc::now();
                circuit.failures.push(now);

                // Prune failures outside the window
                let cutoff = now - Duration::seconds(config_window);
                circuit.failures.retain(|t| *t >= cutoff);

                // Check if threshold exceeded
                if circuit.failures.len() as u32 >= config_threshold {
                    // Open the circuit
                    let cooldown = if circuit.current_cooldown_secs > 0 {
                        let next = (circuit.current_cooldown_secs as f64 * config_backoff) as i64;
                        next.min(config_max_cooldown)
                    } else {
                        config_cooldown
                    };

                    circuit.state = CircuitState::Open;
                    circuit.opened_at = Some(now);
                    circuit.current_cooldown_secs = cooldown;
                    circuit.total_opens += 1;
                    circuit.last_transition = now;
                    warn!(
                        channel = %channel_id,
                        failures = circuit.failures.len(),
                        cooldown_secs = cooldown,
                        "Circuit breaker → open"
                    );
                }
            }
            CircuitState::HalfOpen => {
                // Probe failed — reopen with backoff
                let cooldown = {
                    let next = (circuit.current_cooldown_secs as f64 * config_backoff) as i64;
                    next.min(config_max_cooldown)
                };

                circuit.state = CircuitState::Open;
                circuit.opened_at = Some(Utc::now());
                circuit.current_cooldown_secs = cooldown;
                circuit.half_open_successes = 0;
                circuit.total_opens += 1;
                circuit.last_transition = Utc::now();
                warn!(
                    channel = %channel_id,
                    cooldown_secs = cooldown,
                    "Circuit breaker → open (probe failed, backoff)"
                );
            }
            CircuitState::Open => {
                // Already open, just count
            }
        }
    }

    /// Get the current state of a channel's circuit
    pub fn state(&self, channel_id: &str) -> CircuitState {
        self.circuits
            .get(channel_id)
            .map(|c| c.state)
            .unwrap_or(CircuitState::Closed)
    }

    /// Get health report for all channels
    pub fn health_report(&self) -> Vec<CircuitHealth> {
        self.circuits
            .iter()
            .map(|(id, circuit)| CircuitHealth {
                channel_id: id.clone(),
                state: circuit.state,
                total_failures: circuit.total_failures,
                total_successes: circuit.total_successes,
                total_opens: circuit.total_opens,
                recent_failures: circuit.failures.len() as u32,
                last_transition: circuit.last_transition,
                cooldown_remaining_secs: match (circuit.state, circuit.opened_at) {
                    (CircuitState::Open, Some(opened)) => {
                        let elapsed = (Utc::now() - opened).num_seconds();
                        (circuit.current_cooldown_secs - elapsed).max(0)
                    }
                    _ => 0,
                },
            })
            .collect()
    }

    /// Reset a specific channel's circuit breaker
    pub fn reset(&mut self, channel_id: &str) {
        if let Some(circuit) = self.circuits.get_mut(channel_id) {
            circuit.state = CircuitState::Closed;
            circuit.failures.clear();
            circuit.opened_at = None;
            circuit.current_cooldown_secs = 0;
            circuit.half_open_successes = 0;
            circuit.last_transition = Utc::now();
            debug!(channel = %channel_id, "Circuit breaker manually reset");
        }
    }

    /// Reset all circuits
    pub fn reset_all(&mut self) {
        for (id, circuit) in self.circuits.iter_mut() {
            circuit.state = CircuitState::Closed;
            circuit.failures.clear();
            circuit.opened_at = None;
            circuit.current_cooldown_secs = 0;
            circuit.half_open_successes = 0;
            circuit.last_transition = Utc::now();
            debug!(channel = %id, "Circuit breaker reset (all)");
        }
    }

    /// Number of tracked channels
    pub fn channel_count(&self) -> usize {
        self.circuits.len()
    }

    /// List channels currently in open state
    pub fn open_circuits(&self) -> Vec<String> {
        self.circuits
            .iter()
            .filter(|(_, c)| c.state == CircuitState::Open)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get config
    pub fn config(&self) -> &CircuitBreakerConfig {
        &self.config
    }

    /// Update config (applies to future state transitions)
    pub fn set_config(&mut self, config: CircuitBreakerConfig) {
        self.config = config;
    }
}

impl Default for CircuitBreakerManager {
    fn default() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }
}

// ============================================================================
// Health Report
// ============================================================================

/// Health status of a single channel's circuit breaker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitHealth {
    pub channel_id: String,
    pub state: CircuitState,
    pub total_failures: u64,
    pub total_successes: u64,
    pub total_opens: u64,
    pub recent_failures: u32,
    pub last_transition: DateTime<Utc>,
    pub cooldown_remaining_secs: i64,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manager() -> CircuitBreakerManager {
        CircuitBreakerManager::new(CircuitBreakerConfig {
            failure_threshold: 3,
            failure_window_secs: 60,
            cooldown_secs: 10,
            max_cooldown_secs: 120,
            success_threshold: 2,
            backoff_multiplier: 2.0,
        })
    }

    #[test]
    fn test_initial_state_closed() {
        let manager = test_manager();
        assert_eq!(manager.state("telegram"), CircuitState::Closed);
    }

    #[test]
    fn test_allow_when_closed() {
        let mut manager = test_manager();
        let verdict = manager.check("telegram");
        assert_eq!(verdict, RequestVerdict::Allow);
    }

    #[test]
    fn test_stays_closed_below_threshold() {
        let mut manager = test_manager();
        manager.record_failure("telegram");
        manager.record_failure("telegram");
        // 2 failures, threshold is 3
        assert_eq!(manager.state("telegram"), CircuitState::Closed);
        assert_eq!(manager.check("telegram"), RequestVerdict::Allow);
    }

    #[test]
    fn test_opens_at_threshold() {
        let mut manager = test_manager();
        for _ in 0..3 {
            manager.record_failure("telegram");
        }
        assert_eq!(manager.state("telegram"), CircuitState::Open);
    }

    #[test]
    fn test_blocks_when_open() {
        let mut manager = test_manager();
        for _ in 0..3 {
            manager.record_failure("telegram");
        }
        let verdict = manager.check("telegram");
        assert!(verdict.is_blocked());
        match verdict {
            RequestVerdict::Block { retry_after_secs } => {
                assert!(retry_after_secs > 0);
                assert!(retry_after_secs <= 10);
            }
            _ => panic!("Expected Block"),
        }
    }

    #[test]
    fn test_half_open_after_cooldown() {
        let mut manager = test_manager();
        for _ in 0..3 {
            manager.record_failure("telegram");
        }
        // Manually backdate opened_at to simulate cooldown
        if let Some(circuit) = manager.circuits.get_mut("telegram") {
            circuit.opened_at = Some(Utc::now() - Duration::seconds(15));
        }
        let verdict = manager.check("telegram");
        assert_eq!(verdict, RequestVerdict::Probe);
        assert_eq!(manager.state("telegram"), CircuitState::HalfOpen);
    }

    #[test]
    fn test_close_after_successful_probes() {
        let mut manager = test_manager();
        for _ in 0..3 {
            manager.record_failure("telegram");
        }
        // Force half-open
        if let Some(circuit) = manager.circuits.get_mut("telegram") {
            circuit.opened_at = Some(Utc::now() - Duration::seconds(15));
        }
        manager.check("telegram"); // triggers half-open

        // Two successful probes (success_threshold = 2)
        manager.record_success("telegram");
        assert_eq!(manager.state("telegram"), CircuitState::HalfOpen);
        manager.record_success("telegram");
        assert_eq!(manager.state("telegram"), CircuitState::Closed);
    }

    #[test]
    fn test_reopen_on_probe_failure() {
        let mut manager = test_manager();
        for _ in 0..3 {
            manager.record_failure("telegram");
        }
        // Force half-open
        if let Some(circuit) = manager.circuits.get_mut("telegram") {
            circuit.opened_at = Some(Utc::now() - Duration::seconds(15));
        }
        manager.check("telegram"); // triggers half-open
        assert_eq!(manager.state("telegram"), CircuitState::HalfOpen);

        // Probe fails — reopens with backoff
        manager.record_failure("telegram");
        assert_eq!(manager.state("telegram"), CircuitState::Open);

        // Cooldown should have doubled (10 * 2 = 20)
        if let Some(circuit) = manager.circuits.get("telegram") {
            assert_eq!(circuit.current_cooldown_secs, 20);
        }
    }

    #[test]
    fn test_backoff_caps_at_max() {
        let mut manager = test_manager();

        // Cycle through open/half-open/reopen several times
        for _ in 0..10 {
            for _ in 0..3 {
                manager.record_failure("telegram");
            }
            if let Some(circuit) = manager.circuits.get_mut("telegram") {
                circuit.opened_at = Some(Utc::now() - Duration::seconds(600));
            }
            manager.check("telegram"); // half-open
            manager.record_failure("telegram"); // reopen with backoff
        }

        if let Some(circuit) = manager.circuits.get("telegram") {
            assert!(circuit.current_cooldown_secs <= 120); // max_cooldown_secs
        }
    }

    #[test]
    fn test_independent_channels() {
        let mut manager = test_manager();
        for _ in 0..3 {
            manager.record_failure("telegram");
        }
        assert_eq!(manager.state("telegram"), CircuitState::Open);
        assert_eq!(manager.state("discord"), CircuitState::Closed);
        assert_eq!(manager.check("discord"), RequestVerdict::Allow);
    }

    #[test]
    fn test_health_report() {
        let mut manager = test_manager();
        manager.record_success("telegram");
        manager.record_failure("discord");

        let report = manager.health_report();
        assert_eq!(report.len(), 2);

        let tg = report.iter().find(|h| h.channel_id == "telegram").unwrap();
        assert_eq!(tg.state, CircuitState::Closed);
        assert_eq!(tg.total_successes, 1);

        let dc = report.iter().find(|h| h.channel_id == "discord").unwrap();
        assert_eq!(dc.total_failures, 1);
    }

    #[test]
    fn test_manual_reset() {
        let mut manager = test_manager();
        for _ in 0..3 {
            manager.record_failure("telegram");
        }
        assert_eq!(manager.state("telegram"), CircuitState::Open);

        manager.reset("telegram");
        assert_eq!(manager.state("telegram"), CircuitState::Closed);
        assert_eq!(manager.check("telegram"), RequestVerdict::Allow);
    }

    #[test]
    fn test_reset_all() {
        let mut manager = test_manager();
        for _ in 0..3 {
            manager.record_failure("telegram");
            manager.record_failure("discord");
        }
        assert_eq!(manager.state("telegram"), CircuitState::Open);
        assert_eq!(manager.state("discord"), CircuitState::Open);

        manager.reset_all();
        assert_eq!(manager.state("telegram"), CircuitState::Closed);
        assert_eq!(manager.state("discord"), CircuitState::Closed);
    }

    #[test]
    fn test_channel_count() {
        let mut manager = test_manager();
        assert_eq!(manager.channel_count(), 0);
        manager.check("telegram");
        assert_eq!(manager.channel_count(), 1);
        manager.check("discord");
        assert_eq!(manager.channel_count(), 2);
    }

    #[test]
    fn test_open_circuits_list() {
        let mut manager = test_manager();
        for _ in 0..3 {
            manager.record_failure("telegram");
        }
        manager.record_success("discord");

        let open = manager.open_circuits();
        assert_eq!(open.len(), 1);
        assert!(open.contains(&"telegram".to_string()));
    }

    #[test]
    fn test_verdict_helpers() {
        assert!(RequestVerdict::Allow.is_allowed());
        assert!(RequestVerdict::Probe.is_allowed());
        assert!(
            !RequestVerdict::Block {
                retry_after_secs: 10
            }
            .is_allowed()
        );

        assert!(!RequestVerdict::Allow.is_blocked());
        assert!(
            RequestVerdict::Block {
                retry_after_secs: 5
            }
            .is_blocked()
        );
    }

    #[test]
    fn test_state_display() {
        assert_eq!(format!("{}", CircuitState::Closed), "closed");
        assert_eq!(format!("{}", CircuitState::Open), "open");
        assert_eq!(format!("{}", CircuitState::HalfOpen), "half-open");
    }

    #[test]
    fn test_failure_window_pruning() {
        let mut manager = CircuitBreakerManager::new(CircuitBreakerConfig {
            failure_threshold: 3,
            failure_window_secs: 2,
            cooldown_secs: 5,
            ..Default::default()
        });

        // Add 2 failures
        manager.record_failure("telegram");
        manager.record_failure("telegram");

        // Backdate them outside the window
        if let Some(circuit) = manager.circuits.get_mut("telegram") {
            for f in circuit.failures.iter_mut() {
                *f = Utc::now() - Duration::seconds(5);
            }
        }

        // Third failure should NOT trip the breaker (old ones pruned)
        manager.record_failure("telegram");
        assert_eq!(manager.state("telegram"), CircuitState::Closed);
    }

    #[test]
    fn test_success_while_closed() {
        let mut manager = test_manager();
        manager.record_success("telegram");
        assert_eq!(manager.state("telegram"), CircuitState::Closed);
        if let Some(circuit) = manager.circuits.get("telegram") {
            assert_eq!(circuit.total_successes, 1);
        }
    }

    #[test]
    fn test_config_update() {
        let mut manager = test_manager();
        manager.set_config(CircuitBreakerConfig {
            failure_threshold: 10,
            ..Default::default()
        });
        assert_eq!(manager.config().failure_threshold, 10);
    }

    #[test]
    fn test_default_manager() {
        let manager = CircuitBreakerManager::default();
        assert_eq!(manager.config().failure_threshold, 5);
        assert_eq!(manager.config().cooldown_secs, 30);
        assert_eq!(manager.channel_count(), 0);
    }

    #[test]
    fn test_total_opens_count() {
        let mut manager = test_manager();

        // First open
        for _ in 0..3 {
            manager.record_failure("telegram");
        }
        // Force half-open and reopen
        if let Some(circuit) = manager.circuits.get_mut("telegram") {
            circuit.opened_at = Some(Utc::now() - Duration::seconds(15));
        }
        manager.check("telegram");
        manager.record_failure("telegram"); // reopen

        if let Some(circuit) = manager.circuits.get("telegram") {
            assert_eq!(circuit.total_opens, 2);
        }
    }

    #[test]
    fn test_recovery_resets_failures() {
        let mut manager = test_manager();
        for _ in 0..3 {
            manager.record_failure("telegram");
        }
        // Force half-open
        if let Some(circuit) = manager.circuits.get_mut("telegram") {
            circuit.opened_at = Some(Utc::now() - Duration::seconds(15));
        }
        manager.check("telegram");

        // Two successes → closed
        manager.record_success("telegram");
        manager.record_success("telegram");
        assert_eq!(manager.state("telegram"), CircuitState::Closed);

        // Failure list should be cleared after recovery
        if let Some(circuit) = manager.circuits.get("telegram") {
            assert!(circuit.failures.is_empty());
            assert_eq!(circuit.current_cooldown_secs, 0);
        }
    }
}
