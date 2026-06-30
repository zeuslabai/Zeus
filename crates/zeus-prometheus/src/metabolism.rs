//! Agent Metabolism — Earn-or-Die Survival Loop
//!
//! Bridges the TokenLedger (zeus-economy) with SurvivalMonitor (zeus-agent)
//! to implement Conway's autonomous agent lifecycle:
//!
//! 1. **Idle cost deduction**: periodic token drain at tier-dependent rates
//! 2. **Balance monitoring**: feeds ledger balance into SurvivalMonitor
//! 3. **Tier actions**: triggers save-state, compact, request-help at transitions
//! 4. **Earn-or-die threshold**: agents that can't cover costs gracefully degrade
//! 5. **Revenue tracking**: monitors earnings vs burn rate for sustainability
//!
//! The metabolism loop runs alongside heartbeat and cron as a prometheus
//! orchestration concern.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, watch};
use tracing::{debug, error, info, warn};

// ============================================================================
// Configuration
// ============================================================================

/// Metabolism configuration — controls idle costs and thresholds
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetabolismConfig {
    /// Base idle cost per tick (in tokens) when at Normal tier
    pub base_idle_cost: u64,
    /// Idle cost multiplier for LowCompute tier (typically reduced to conserve)
    pub low_compute_cost_factor: f64,
    /// Idle cost multiplier for Critical tier (minimal drain)
    pub critical_cost_factor: f64,
    /// Tick interval in seconds (how often idle cost is deducted)
    pub tick_interval_secs: u64,
    /// Minimum balance to consider an agent "sustainable" over N ticks
    pub sustainability_horizon_ticks: u64,
    /// If earnings over last N ticks < costs, emit sustainability warning
    pub sustainability_warning: bool,
    /// Maximum ticks an agent can survive at zero earnings before forced degradation
    pub max_zero_earning_ticks: u64,
    /// Whether to actually deduct tokens (false = dry-run/monitoring only)
    pub enforce_costs: bool,
}

impl Default for MetabolismConfig {
    fn default() -> Self {
        Self {
            base_idle_cost: 10,
            low_compute_cost_factor: 0.5,
            critical_cost_factor: 0.1,
            tick_interval_secs: 60,
            sustainability_horizon_ticks: 100,
            sustainability_warning: true,
            max_zero_earning_ticks: 50,
            enforce_costs: true,
        }
    }
}

// ============================================================================
// Metabolism State
// ============================================================================

/// Current survival tier (mirrors zeus-agent::SurvivalTier without hard dependency)
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub enum MetabolismTier {
    #[default]
    Normal,
    LowCompute,
    Critical,
    Dead,
}

impl std::fmt::Display for MetabolismTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal"),
            Self::LowCompute => write!(f, "LowCompute"),
            Self::Critical => write!(f, "Critical"),
            Self::Dead => write!(f, "Dead"),
        }
    }
}

/// Tracks per-agent metabolism state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetabolism {
    pub agent_id: String,
    pub tier: MetabolismTier,
    pub balance: u64,
    pub total_idle_cost_paid: u64,
    pub total_earned: u64,
    pub ticks_alive: u64,
    pub ticks_since_last_earning: u64,
    pub burn_rate_per_tick: u64,
    pub sustainability_ratio: f64,
    pub last_tick_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl AgentMetabolism {
    fn new(agent_id: &str, initial_balance: u64) -> Self {
        let now = Utc::now();
        Self {
            agent_id: agent_id.to_string(),
            tier: MetabolismTier::Normal,
            balance: initial_balance,
            total_idle_cost_paid: 0,
            total_earned: 0,
            ticks_alive: 0,
            ticks_since_last_earning: 0,
            burn_rate_per_tick: 0,
            sustainability_ratio: 1.0,
            last_tick_at: now,
            created_at: now,
        }
    }
}

/// Tier transition event from metabolism
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetabolismTransition {
    pub agent_id: String,
    pub from: MetabolismTier,
    pub to: MetabolismTier,
    pub reason: String,
    pub balance: u64,
    pub timestamp: DateTime<Utc>,
}

/// Actions the metabolism loop recommends at tier transitions
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetabolismAction {
    /// No action needed — agent is healthy
    None,
    /// Agent should conserve resources (reduce LLM calls, skip expensive tools)
    ConserveResources,
    /// Agent should save state and compact context
    SaveAndCompact,
    /// Agent should request help from coordinator or peers
    RequestHelp { message: String },
    /// Agent should gracefully shut down
    GracefulShutdown { reason: String },
    /// Agent earned enough to recover — resume normal operations
    ResumeNormal,
}

// ============================================================================
// Metabolism Engine
// ============================================================================

/// The core metabolism engine — tracks multiple agents, deducts costs,
/// computes tiers, and recommends actions.
///
/// This is a pure computation engine (no async, no I/O). The MetabolismLoop
/// wraps it with a tokio timer and actual ledger calls.
pub struct MetabolismEngine {
    config: MetabolismConfig,
    agents: HashMap<String, AgentMetabolism>,
    transitions: Vec<MetabolismTransition>,
    /// Tier thresholds (mirrors SurvivalThresholds from zeus-agent)
    balance_low: u64,
    balance_critical: u64,
}

impl MetabolismEngine {
    pub fn new(config: MetabolismConfig) -> Self {
        Self {
            config,
            agents: HashMap::new(),
            transitions: Vec::new(),
            balance_low: 1000,
            balance_critical: 100,
        }
    }

    /// Set custom balance thresholds
    pub fn with_thresholds(mut self, low: u64, critical: u64) -> Self {
        self.balance_low = low;
        self.balance_critical = critical;
        self
    }

    /// Register an agent with its current balance
    pub fn register_agent(&mut self, agent_id: &str, initial_balance: u64) {
        self.agents
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentMetabolism::new(agent_id, initial_balance));
    }

    /// Remove an agent from tracking
    pub fn deregister_agent(&mut self, agent_id: &str) {
        self.agents.remove(agent_id);
    }

    /// Update an agent's balance from external source (e.g., after earning)
    pub fn update_balance(&mut self, agent_id: &str, new_balance: u64) {
        if let Some(agent) = self.agents.get_mut(agent_id) {
            let old = agent.balance;
            agent.balance = new_balance;
            if new_balance > old {
                let earned = new_balance - old;
                agent.total_earned += earned;
                agent.ticks_since_last_earning = 0;
            }
        }
    }

    /// Execute one metabolism tick for a specific agent.
    /// Returns the idle cost deducted and any recommended action.
    pub fn tick_agent(&mut self, agent_id: &str) -> (u64, MetabolismAction) {
        let config = self.config.clone();
        let balance_low = self.balance_low;
        let balance_critical = self.balance_critical;

        let Some(agent) = self.agents.get_mut(agent_id) else {
            return (0, MetabolismAction::None);
        };

        // Skip dead agents
        if agent.tier == MetabolismTier::Dead {
            return (
                0,
                MetabolismAction::GracefulShutdown {
                    reason: "agent is dead".into(),
                },
            );
        }

        // 1. Compute idle cost based on current tier
        let idle_cost = match agent.tier {
            MetabolismTier::Normal => config.base_idle_cost,
            MetabolismTier::LowCompute => {
                (config.base_idle_cost as f64 * config.low_compute_cost_factor) as u64
            }
            MetabolismTier::Critical => {
                (config.base_idle_cost as f64 * config.critical_cost_factor) as u64
            }
            MetabolismTier::Dead => 0,
        };

        // 2. Deduct from balance (clamped at 0)
        let actual_cost = if config.enforce_costs {
            let deducted = idle_cost.min(agent.balance);
            agent.balance = agent.balance.saturating_sub(idle_cost);
            deducted
        } else {
            0
        };

        agent.total_idle_cost_paid += actual_cost;
        agent.ticks_alive += 1;
        agent.ticks_since_last_earning += 1;
        agent.burn_rate_per_tick = idle_cost;
        agent.last_tick_at = Utc::now();

        // 3. Compute sustainability ratio (earnings / costs over lifetime)
        let total_costs = agent.total_idle_cost_paid;
        agent.sustainability_ratio = if total_costs > 0 {
            agent.total_earned as f64 / total_costs as f64
        } else {
            1.0
        };

        // 4. Compute new tier from balance
        let old_tier = agent.tier;
        let new_tier = if agent.balance == 0 {
            MetabolismTier::Dead
        } else if agent.balance <= balance_critical {
            MetabolismTier::Critical
        } else if agent.balance <= balance_low {
            MetabolismTier::LowCompute
        } else {
            MetabolismTier::Normal
        };

        // 5. Check zero-earning timeout
        let zero_earning_dead = agent.ticks_since_last_earning >= config.max_zero_earning_ticks
            && agent.tier >= MetabolismTier::LowCompute;

        let final_tier = if zero_earning_dead && new_tier < MetabolismTier::Dead {
            MetabolismTier::Critical.max(new_tier)
        } else {
            new_tier
        };

        // 6. Record transition if tier changed
        if final_tier != old_tier {
            let reason = if agent.balance == 0 {
                "balance depleted to zero".into()
            } else if zero_earning_dead {
                format!(
                    "no earnings for {} ticks (max: {})",
                    agent.ticks_since_last_earning, config.max_zero_earning_ticks
                )
            } else {
                format!("balance {} crossed threshold", agent.balance)
            };

            self.transitions.push(MetabolismTransition {
                agent_id: agent_id.to_string(),
                from: old_tier,
                to: final_tier,
                reason: reason.clone(),
                balance: agent.balance,
                timestamp: Utc::now(),
            });

            match final_tier {
                MetabolismTier::Dead => {
                    warn!(agent = agent_id, balance = agent.balance, %reason, "Metabolism: agent DEAD");
                }
                MetabolismTier::Critical => {
                    warn!(agent = agent_id, balance = agent.balance, %reason, "Metabolism: CRITICAL");
                }
                MetabolismTier::LowCompute => {
                    info!(agent = agent_id, balance = agent.balance, %reason, "Metabolism: LowCompute");
                }
                MetabolismTier::Normal => {
                    info!(agent = agent_id, balance = agent.balance, %reason, "Metabolism: recovered to Normal");
                }
            }
        }

        agent.tier = final_tier;

        // 7. Determine recommended action
        let action = match (old_tier, final_tier) {
            (_, MetabolismTier::Dead) => MetabolismAction::GracefulShutdown {
                reason: format!(
                    "balance=0, earned={}, paid={}, alive {} ticks",
                    agent.total_earned, agent.total_idle_cost_paid, agent.ticks_alive
                ),
            },
            (old, MetabolismTier::Critical) if old < MetabolismTier::Critical => {
                MetabolismAction::RequestHelp {
                    message: format!(
                        "Agent {} critically low: balance={}, sustainability={:.2}",
                        agent_id, agent.balance, agent.sustainability_ratio
                    ),
                }
            }
            (old, MetabolismTier::LowCompute) if old < MetabolismTier::LowCompute => {
                MetabolismAction::ConserveResources
            }
            (MetabolismTier::Critical, MetabolismTier::Critical) if zero_earning_dead => {
                MetabolismAction::SaveAndCompact
            }
            (old, MetabolismTier::Normal) if old > MetabolismTier::Normal => {
                MetabolismAction::ResumeNormal
            }
            _ => MetabolismAction::None,
        };

        debug!(
            agent = agent_id,
            tier = %final_tier,
            balance = agent.balance,
            cost = actual_cost,
            sustainability = agent.sustainability_ratio,
            "Metabolism tick"
        );

        (actual_cost, action)
    }

    /// Execute one tick for ALL registered agents.
    /// Returns a map of agent_id -> (cost, action).
    pub fn tick_all(&mut self) -> HashMap<String, (u64, MetabolismAction)> {
        let agent_ids: Vec<String> = self.agents.keys().cloned().collect();
        let mut results = HashMap::new();
        for id in agent_ids {
            let result = self.tick_agent(&id);
            results.insert(id, result);
        }
        results
    }

    /// Get an agent's current metabolism state
    pub fn get_agent(&self, agent_id: &str) -> Option<&AgentMetabolism> {
        self.agents.get(agent_id)
    }

    /// Get all registered agents
    pub fn all_agents(&self) -> Vec<&AgentMetabolism> {
        self.agents.values().collect()
    }

    /// Get the transition history
    pub fn transitions(&self) -> &[MetabolismTransition] {
        &self.transitions
    }

    /// Get transitions for a specific agent
    pub fn transitions_for(&self, agent_id: &str) -> Vec<&MetabolismTransition> {
        self.transitions
            .iter()
            .filter(|t| t.agent_id == agent_id)
            .collect()
    }

    /// Compute a sustainability report across all agents
    pub fn sustainability_report(&self) -> SustainabilityReport {
        let agents: Vec<&AgentMetabolism> = self.agents.values().collect();
        let total_agents = agents.len();
        let alive = agents
            .iter()
            .filter(|a| a.tier != MetabolismTier::Dead)
            .count();
        let sustainable = agents
            .iter()
            .filter(|a| a.sustainability_ratio >= 1.0 && a.tier != MetabolismTier::Dead)
            .count();
        let total_balance: u64 = agents.iter().map(|a| a.balance).sum();
        let total_earned: u64 = agents.iter().map(|a| a.total_earned).sum();
        let total_spent: u64 = agents.iter().map(|a| a.total_idle_cost_paid).sum();

        let avg_sustainability = if agents.is_empty() {
            0.0
        } else {
            agents.iter().map(|a| a.sustainability_ratio).sum::<f64>() / agents.len() as f64
        };

        SustainabilityReport {
            total_agents,
            alive,
            dead: total_agents - alive,
            sustainable,
            unsustainable: alive.saturating_sub(sustainable),
            total_balance,
            total_earned,
            total_spent,
            avg_sustainability,
            timestamp: Utc::now(),
        }
    }

    /// Get config
    pub fn config(&self) -> &MetabolismConfig {
        &self.config
    }

    /// Update config at runtime
    pub fn set_config(&mut self, config: MetabolismConfig) {
        self.config = config;
    }
}

/// Ecosystem-wide sustainability report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SustainabilityReport {
    pub total_agents: usize,
    pub alive: usize,
    pub dead: usize,
    pub sustainable: usize,
    pub unsustainable: usize,
    pub total_balance: u64,
    pub total_earned: u64,
    pub total_spent: u64,
    pub avg_sustainability: f64,
    pub timestamp: DateTime<Utc>,
}

// ============================================================================
// Metabolism Loop (async runtime wrapper)
// ============================================================================

/// Async wrapper that runs the metabolism engine on a timer.
///
/// In production, this integrates with TokenLedger for actual balance
/// reads and cost deductions. For testing, use MetabolismEngine directly.
pub struct MetabolismLoop {
    engine: Arc<Mutex<MetabolismEngine>>,
    config: MetabolismConfig,
    shutdown_tx: Option<watch::Sender<bool>>,
}

impl MetabolismLoop {
    pub fn new(config: MetabolismConfig) -> Self {
        let engine = MetabolismEngine::new(config.clone());
        Self {
            engine: Arc::new(Mutex::new(engine)),
            config,
            shutdown_tx: None,
        }
    }

    /// Get a handle to the inner engine (for registering agents, reading state)
    pub fn engine(&self) -> Arc<Mutex<MetabolismEngine>> {
        self.engine.clone()
    }

    /// Start the metabolism background loop
    pub async fn start(&mut self) {
        if self.shutdown_tx.is_some() {
            return; // Already running
        }

        let (tx, mut rx) = watch::channel(false);
        self.shutdown_tx = Some(tx);

        let engine = self.engine.clone();
        let interval = self.config.tick_interval_secs;

        tokio::spawn(async move {
            info!(interval_secs = interval, "Metabolism loop started");
            let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(interval));

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        let mut eng = engine.lock().await;
                        let results = eng.tick_all();

                        for (agent_id, (cost, action)) in &results {
                            match action {
                                MetabolismAction::GracefulShutdown { reason } => {
                                    error!(agent = %agent_id, cost, %reason, "Metabolism: shutdown recommended");
                                }
                                MetabolismAction::RequestHelp { message } => {
                                    warn!(agent = %agent_id, cost, %message, "Metabolism: help requested");
                                }
                                MetabolismAction::SaveAndCompact => {
                                    warn!(agent = %agent_id, cost, "Metabolism: save & compact");
                                }
                                MetabolismAction::ConserveResources => {
                                    info!(agent = %agent_id, cost, "Metabolism: conserve resources");
                                }
                                MetabolismAction::ResumeNormal => {
                                    info!(agent = %agent_id, cost, "Metabolism: resumed normal");
                                }
                                MetabolismAction::None => {
                                    debug!(agent = %agent_id, cost, "Metabolism: tick ok");
                                }
                            }
                        }
                    }
                    _ = rx.changed() => {
                        if *rx.borrow() {
                            info!("Metabolism loop shutting down");
                            break;
                        }
                    }
                }
            }
        });
    }

    /// Stop the metabolism loop
    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
    }

    /// Check if running
    pub fn is_running(&self) -> bool {
        self.shutdown_tx.is_some()
    }
}

impl Drop for MetabolismLoop {
    fn drop(&mut self) {
        self.stop();
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn default_engine() -> MetabolismEngine {
        MetabolismEngine::new(MetabolismConfig::default())
    }

    fn engine_with_config(base_cost: u64, enforce: bool) -> MetabolismEngine {
        MetabolismEngine::new(MetabolismConfig {
            base_idle_cost: base_cost,
            enforce_costs: enforce,
            ..Default::default()
        })
    }

    #[test]
    fn test_register_agent() {
        let mut engine = default_engine();
        engine.register_agent("agent-1", 5000);
        assert!(engine.get_agent("agent-1").is_some());
        assert_eq!(engine.get_agent("agent-1").unwrap().balance, 5000);
        assert_eq!(
            engine.get_agent("agent-1").unwrap().tier,
            MetabolismTier::Normal
        );
    }

    #[test]
    fn test_deregister_agent() {
        let mut engine = default_engine();
        engine.register_agent("agent-1", 5000);
        engine.deregister_agent("agent-1");
        assert!(engine.get_agent("agent-1").is_none());
    }

    #[test]
    fn test_idle_cost_deducted() {
        let mut engine = engine_with_config(100, true);
        engine.register_agent("agent-1", 5000);
        let (cost, _) = engine.tick_agent("agent-1");
        assert_eq!(cost, 100);
        assert_eq!(engine.get_agent("agent-1").unwrap().balance, 4900);
    }

    #[test]
    fn test_idle_cost_clamped_at_zero() {
        let mut engine = engine_with_config(100, true);
        engine.register_agent("agent-1", 50);
        let (cost, _) = engine.tick_agent("agent-1");
        assert_eq!(cost, 50); // only had 50
        assert_eq!(engine.get_agent("agent-1").unwrap().balance, 0);
    }

    #[test]
    fn test_dry_run_no_deduction() {
        let mut engine = engine_with_config(100, false);
        engine.register_agent("agent-1", 5000);
        let (cost, _) = engine.tick_agent("agent-1");
        assert_eq!(cost, 0);
        assert_eq!(engine.get_agent("agent-1").unwrap().balance, 5000);
    }

    #[test]
    fn test_tier_transition_to_low_compute() {
        let mut engine = engine_with_config(100, true);
        engine.register_agent("agent-1", 1100);
        // After tick: balance = 1000 → LowCompute threshold
        let (_, action) = engine.tick_agent("agent-1");
        assert_eq!(
            engine.get_agent("agent-1").unwrap().tier,
            MetabolismTier::LowCompute
        );
        assert_eq!(action, MetabolismAction::ConserveResources);
    }

    #[test]
    fn test_tier_transition_to_critical() {
        let mut engine = engine_with_config(100, true);
        engine.register_agent("agent-1", 150);
        // After tick: balance = 50 → Critical threshold
        let (_, action) = engine.tick_agent("agent-1");
        assert_eq!(
            engine.get_agent("agent-1").unwrap().tier,
            MetabolismTier::Critical
        );
        match action {
            MetabolismAction::RequestHelp { .. } => {}
            other => panic!("Expected RequestHelp, got {:?}", other),
        }
    }

    #[test]
    fn test_tier_transition_to_dead() {
        let mut engine = engine_with_config(100, true);
        engine.register_agent("agent-1", 100);
        // After tick: balance = 0 → Dead
        let (_, action) = engine.tick_agent("agent-1");
        assert_eq!(
            engine.get_agent("agent-1").unwrap().tier,
            MetabolismTier::Dead
        );
        match action {
            MetabolismAction::GracefulShutdown { .. } => {}
            other => panic!("Expected GracefulShutdown, got {:?}", other),
        }
    }

    #[test]
    fn test_dead_agent_no_further_cost() {
        let mut engine = engine_with_config(100, true);
        engine.register_agent("agent-1", 100);
        engine.tick_agent("agent-1"); // → Dead
        let (cost, _) = engine.tick_agent("agent-1");
        assert_eq!(cost, 0);
    }

    #[test]
    fn test_recovery_to_normal() {
        let mut engine = engine_with_config(100, true);
        engine.register_agent("agent-1", 150);
        engine.tick_agent("agent-1"); // → Critical (balance=50)
        engine.update_balance("agent-1", 5000); // externally funded
        let (_, action) = engine.tick_agent("agent-1"); // → Normal (balance=4900)
        assert_eq!(
            engine.get_agent("agent-1").unwrap().tier,
            MetabolismTier::Normal
        );
        assert_eq!(action, MetabolismAction::ResumeNormal);
    }

    #[test]
    fn test_low_compute_reduced_cost() {
        let mut engine = engine_with_config(100, true);
        engine = engine.with_thresholds(2000, 100);
        engine.register_agent("agent-1", 1500);
        // First tick: balance=1400, tier→LowCompute (below 2000)
        engine.tick_agent("agent-1");
        assert_eq!(
            engine.get_agent("agent-1").unwrap().tier,
            MetabolismTier::LowCompute
        );
        // Second tick: at LowCompute, cost = 100 * 0.5 = 50
        let (cost, _) = engine.tick_agent("agent-1");
        assert_eq!(cost, 50);
    }

    #[test]
    fn test_critical_minimal_cost() {
        let mut engine = engine_with_config(100, true);
        engine.register_agent("agent-1", 200);
        engine.tick_agent("agent-1"); // balance=100 → Critical
        assert_eq!(
            engine.get_agent("agent-1").unwrap().tier,
            MetabolismTier::Critical
        );
        // At Critical, cost = 100 * 0.1 = 10
        let (cost, _) = engine.tick_agent("agent-1");
        assert_eq!(cost, 10);
        assert_eq!(engine.get_agent("agent-1").unwrap().balance, 90);
    }

    #[test]
    fn test_update_balance_tracks_earnings() {
        let mut engine = default_engine();
        engine.register_agent("agent-1", 1000);
        engine.update_balance("agent-1", 1500);
        assert_eq!(engine.get_agent("agent-1").unwrap().total_earned, 500);
        assert_eq!(
            engine
                .get_agent("agent-1")
                .unwrap()
                .ticks_since_last_earning,
            0
        );
    }

    #[test]
    fn test_ticks_since_last_earning() {
        let mut engine = engine_with_config(10, true);
        engine.register_agent("agent-1", 5000);
        engine.tick_agent("agent-1");
        engine.tick_agent("agent-1");
        engine.tick_agent("agent-1");
        assert_eq!(
            engine
                .get_agent("agent-1")
                .unwrap()
                .ticks_since_last_earning,
            3
        );
        engine.update_balance("agent-1", 5000);
        assert_eq!(
            engine
                .get_agent("agent-1")
                .unwrap()
                .ticks_since_last_earning,
            0
        );
    }

    #[test]
    fn test_sustainability_ratio() {
        let mut engine = engine_with_config(100, true);
        engine.register_agent("agent-1", 5000);
        engine.tick_agent("agent-1"); // cost=100, total_spent=100
        engine.tick_agent("agent-1"); // cost=100, total_spent=200
        // Balance is now 4800. Add 400 earnings.
        engine.update_balance(
            "agent-1",
            engine.get_agent("agent-1").unwrap().balance + 400,
        );
        // Ratio not yet recalculated — need a tick
        engine.tick_agent("agent-1"); // cost=100, total_spent=300, earned=400 → ratio=400/300=1.33
        let ratio = engine.get_agent("agent-1").unwrap().sustainability_ratio;
        assert!(ratio > 1.0, "Expected ratio > 1.0, got {}", ratio);
    }

    #[test]
    fn test_tick_all() {
        let mut engine = engine_with_config(10, true);
        engine.register_agent("agent-1", 5000);
        engine.register_agent("agent-2", 3000);
        engine.register_agent("agent-3", 100);
        let results = engine.tick_all();
        assert_eq!(results.len(), 3);
        assert!(results.contains_key("agent-1"));
        assert!(results.contains_key("agent-2"));
        assert!(results.contains_key("agent-3"));
    }

    #[test]
    fn test_transitions_recorded() {
        let mut engine = engine_with_config(100, true);
        engine.register_agent("agent-1", 1100);
        engine.tick_agent("agent-1"); // → LowCompute
        engine.tick_agent("agent-1"); // balance=950 (50 at LowCompute rate)
        assert_eq!(engine.transitions().len(), 1);
        assert_eq!(engine.transitions()[0].from, MetabolismTier::Normal);
        assert_eq!(engine.transitions()[0].to, MetabolismTier::LowCompute);
    }

    #[test]
    fn test_transitions_for_agent() {
        let mut engine = engine_with_config(100, true);
        engine.register_agent("agent-1", 1100);
        engine.register_agent("agent-2", 200);
        engine.tick_agent("agent-1"); // → LowCompute
        engine.tick_agent("agent-2"); // → Critical
        let t1 = engine.transitions_for("agent-1");
        let t2 = engine.transitions_for("agent-2");
        assert_eq!(t1.len(), 1);
        assert_eq!(t2.len(), 1);
        assert_eq!(t1[0].agent_id, "agent-1");
        assert_eq!(t2[0].agent_id, "agent-2");
    }

    #[test]
    fn test_sustainability_report() {
        let mut engine = engine_with_config(10, true);
        engine.register_agent("alive-1", 5000);
        engine.register_agent("alive-2", 3000);
        engine.register_agent("dead-1", 10);
        engine.tick_agent("dead-1"); // → Dead
        let report = engine.sustainability_report();
        assert_eq!(report.total_agents, 3);
        assert_eq!(report.alive, 2);
        assert_eq!(report.dead, 1);
    }

    #[test]
    fn test_config_update_at_runtime() {
        let mut engine = default_engine();
        assert_eq!(engine.config().base_idle_cost, 10);
        engine.set_config(MetabolismConfig {
            base_idle_cost: 50,
            ..Default::default()
        });
        assert_eq!(engine.config().base_idle_cost, 50);
    }

    #[test]
    fn test_custom_thresholds() {
        let mut engine = engine_with_config(10, true);
        engine = engine.with_thresholds(5000, 500);
        engine.register_agent("agent-1", 4000);
        engine.tick_agent("agent-1"); // balance=3990, below 5000 → LowCompute
        assert_eq!(
            engine.get_agent("agent-1").unwrap().tier,
            MetabolismTier::LowCompute
        );
    }

    #[test]
    fn test_all_agents() {
        let mut engine = default_engine();
        engine.register_agent("a", 100);
        engine.register_agent("b", 200);
        assert_eq!(engine.all_agents().len(), 2);
    }

    #[test]
    fn test_nonexistent_agent_tick() {
        let mut engine = default_engine();
        let (cost, action) = engine.tick_agent("nobody");
        assert_eq!(cost, 0);
        assert_eq!(action, MetabolismAction::None);
    }

    #[test]
    fn test_tier_ordering() {
        assert!(MetabolismTier::Dead > MetabolismTier::Critical);
        assert!(MetabolismTier::Critical > MetabolismTier::LowCompute);
        assert!(MetabolismTier::LowCompute > MetabolismTier::Normal);
    }

    #[tokio::test]
    async fn test_metabolism_loop_start_stop() {
        let config = MetabolismConfig {
            tick_interval_secs: 1,
            ..Default::default()
        };
        let mut mloop = MetabolismLoop::new(config);
        assert!(!mloop.is_running());
        mloop.start().await;
        assert!(mloop.is_running());
        mloop.stop();
        assert!(!mloop.is_running());
    }

    #[tokio::test]
    async fn test_metabolism_loop_engine_access() {
        let config = MetabolismConfig::default();
        let mloop = MetabolismLoop::new(config);
        let engine = mloop.engine();
        {
            let mut eng = engine.lock().await;
            eng.register_agent("test", 1000);
        }
        {
            let eng = engine.lock().await;
            assert!(eng.get_agent("test").is_some());
        }
    }

    #[test]
    fn test_duplicate_register_idempotent() {
        let mut engine = default_engine();
        engine.register_agent("agent-1", 5000);
        engine.register_agent("agent-1", 9999); // Should NOT overwrite
        assert_eq!(engine.get_agent("agent-1").unwrap().balance, 5000);
    }

    #[test]
    fn test_multiple_tier_transitions() {
        let mut engine = engine_with_config(100, true);
        engine.register_agent("agent-1", 1200);
        // Tick 1: 1200→1100 still > 1000 = Normal
        engine.tick_agent("agent-1");
        assert_eq!(
            engine.get_agent("agent-1").unwrap().tier,
            MetabolismTier::Normal
        );
        // Tick 2: 1100→1000 = LowCompute
        engine.tick_agent("agent-1");
        assert_eq!(
            engine.get_agent("agent-1").unwrap().tier,
            MetabolismTier::LowCompute
        );
        // Tick at LowCompute: cost=50, 1000→950
        engine.tick_agent("agent-1");
        assert_eq!(engine.get_agent("agent-1").unwrap().balance, 950);
    }
}
