//! Compute Provisioning — Resource quotas and budget management for agents
//!
//! Manages per-agent resource allocation in the Conway autonomous ecosystem:
//!
//! 1. **LLM call budgets**: max calls per period, token caps, model tier restrictions
//! 2. **Tool execution quotas**: limits on expensive tools (browser, spawn, voice)
//! 3. **Compute windows**: time-boxed execution budgets that reset periodically
//! 4. **Dynamic scaling**: budget adjusts based on survival tier and priority
//! 5. **Usage tracking**: real-time consumption monitoring with overage alerts
//!
//! Integrates with MetabolismEngine — tier transitions can restrict compute budgets.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

// ============================================================================
// Resource Quota
// ============================================================================

/// Per-agent resource quota for a compute window
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceQuota {
    /// Maximum LLM calls allowed in the current window
    pub max_llm_calls: u64,
    /// Maximum tokens (input + output) in the current window
    pub max_tokens: u64,
    /// Maximum tool executions in the current window
    pub max_tool_calls: u64,
    /// Maximum expensive tool calls (browser, spawn, voice)
    pub max_expensive_tool_calls: u64,
    /// Maximum wall-clock seconds for any single operation
    pub max_operation_secs: u64,
    /// Model tier restriction (0=any, 1=small only, 2=small+medium)
    pub model_tier_limit: u8,
}

impl Default for ResourceQuota {
    fn default() -> Self {
        Self {
            max_llm_calls: 100,
            max_tokens: 500_000,
            max_tool_calls: 200,
            max_expensive_tool_calls: 20,
            max_operation_secs: 300,
            model_tier_limit: 0,
        }
    }
}

/// Reduced quota for LowCompute tier
impl ResourceQuota {
    pub fn low_compute() -> Self {
        Self {
            max_llm_calls: 30,
            max_tokens: 100_000,
            max_tool_calls: 50,
            max_expensive_tool_calls: 0,
            max_operation_secs: 120,
            model_tier_limit: 1,
        }
    }

    /// Minimal quota for Critical tier
    pub fn critical() -> Self {
        Self {
            max_llm_calls: 5,
            max_tokens: 20_000,
            max_tool_calls: 10,
            max_expensive_tool_calls: 0,
            max_operation_secs: 60,
            model_tier_limit: 1,
        }
    }

    /// Scale quota by a factor (for priority adjustments)
    pub fn scale(&self, factor: f64) -> Self {
        Self {
            max_llm_calls: (self.max_llm_calls as f64 * factor) as u64,
            max_tokens: (self.max_tokens as f64 * factor) as u64,
            max_tool_calls: (self.max_tool_calls as f64 * factor) as u64,
            max_expensive_tool_calls: (self.max_expensive_tool_calls as f64 * factor) as u64,
            max_operation_secs: self.max_operation_secs, // don't scale timeouts
            model_tier_limit: self.model_tier_limit,
        }
    }
}

// ============================================================================
// Usage Tracking
// ============================================================================

/// Real-time usage counters for an agent's current compute window
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageCounters {
    pub llm_calls: u64,
    pub tokens_used: u64,
    pub tool_calls: u64,
    pub expensive_tool_calls: u64,
    pub errors: u64,
    pub longest_operation_secs: u64,
}

impl UsageCounters {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Result of checking whether a resource request is allowed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuotaCheck {
    /// Request is allowed
    Allowed,
    /// Request denied — quota exceeded
    Denied {
        resource: String,
        used: u64,
        limit: u64,
    },
    /// Request allowed but nearing limit (>80%)
    Warning {
        resource: String,
        used: u64,
        limit: u64,
    },
}

impl QuotaCheck {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed | Self::Warning { .. })
    }
}

// ============================================================================
// Compute Window
// ============================================================================

/// Time period for quota enforcement
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WindowDuration {
    /// Quotas reset every hour
    #[default]
    Hourly,
    /// Quotas reset every day
    Daily,
    /// Quotas reset every week
    Weekly,
    /// Custom duration in seconds
    Custom(u64),
}

impl WindowDuration {
    fn as_duration(&self) -> Duration {
        match self {
            Self::Hourly => Duration::hours(1),
            Self::Daily => Duration::days(1),
            Self::Weekly => Duration::weeks(1),
            Self::Custom(secs) => Duration::seconds(*secs as i64),
        }
    }
}

// ============================================================================
// Agent Compute State
// ============================================================================

/// Per-agent compute provisioning state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCompute {
    pub agent_id: String,
    pub quota: ResourceQuota,
    pub usage: UsageCounters,
    pub window_start: DateTime<Utc>,
    pub window_duration: WindowDuration,
    pub priority: f64,
    pub total_windows: u64,
    pub total_overages: u64,
}

impl AgentCompute {
    fn new(agent_id: &str, quota: ResourceQuota, window: WindowDuration, priority: f64) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            quota,
            usage: UsageCounters::default(),
            window_start: Utc::now(),
            window_duration: window,
            priority,
            total_windows: 0,
            total_overages: 0,
        }
    }

    /// Check if the current window has expired
    fn window_expired(&self) -> bool {
        Utc::now() > self.window_start + self.window_duration.as_duration()
    }

    /// Reset counters for a new window
    fn rotate_window(&mut self) {
        self.usage.reset();
        self.window_start = Utc::now();
        self.total_windows += 1;
    }

    /// Utilization percentage (0.0 to 1.0+) across all dimensions, taking the max
    fn utilization(&self) -> f64 {
        let llm_util = if self.quota.max_llm_calls > 0 {
            self.usage.llm_calls as f64 / self.quota.max_llm_calls as f64
        } else {
            0.0
        };
        let token_util = if self.quota.max_tokens > 0 {
            self.usage.tokens_used as f64 / self.quota.max_tokens as f64
        } else {
            0.0
        };
        let tool_util = if self.quota.max_tool_calls > 0 {
            self.usage.tool_calls as f64 / self.quota.max_tool_calls as f64
        } else {
            0.0
        };
        llm_util.max(token_util).max(tool_util)
    }
}

// ============================================================================
// Compute Provisioner
// ============================================================================

/// Manages compute quotas for all agents in the ecosystem
pub struct ComputeProvisioner {
    agents: HashMap<String, AgentCompute>,
    default_quota: ResourceQuota,
    default_window: WindowDuration,
    /// Tools classified as "expensive"
    expensive_tools: Vec<String>,
}

impl ComputeProvisioner {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            default_quota: ResourceQuota::default(),
            default_window: WindowDuration::Hourly,
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

    /// Set default quota for new agents
    pub fn with_default_quota(mut self, quota: ResourceQuota) -> Self {
        self.default_quota = quota;
        self
    }

    /// Set default window duration
    pub fn with_window(mut self, window: WindowDuration) -> Self {
        self.default_window = window;
        self
    }

    /// Set expensive tool list
    pub fn with_expensive_tools(mut self, tools: Vec<String>) -> Self {
        self.expensive_tools = tools;
        self
    }

    /// Register an agent with default quota
    pub fn register_agent(&mut self, agent_id: &str, priority: f64) {
        let quota = self.default_quota.scale(priority.max(0.1));
        self.agents
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentCompute::new(agent_id, quota, self.default_window, priority));
    }

    /// Register an agent with a custom quota
    pub fn register_agent_with_quota(
        &mut self,
        agent_id: &str,
        quota: ResourceQuota,
        priority: f64,
    ) {
        self.agents
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentCompute::new(agent_id, quota, self.default_window, priority));
    }

    /// Deregister an agent
    pub fn deregister_agent(&mut self, agent_id: &str) {
        self.agents.remove(agent_id);
    }

    /// Apply tier-based quota adjustment (called when metabolism tier changes)
    pub fn apply_tier_quota(&mut self, agent_id: &str, tier: &str) {
        if let Some(agent) = self.agents.get_mut(agent_id) {
            let base = self.default_quota.scale(agent.priority.max(0.1));
            agent.quota = match tier {
                "Normal" => base,
                "LowCompute" => ResourceQuota::low_compute(),
                "Critical" => ResourceQuota::critical(),
                "Dead" => ResourceQuota {
                    max_llm_calls: 0,
                    max_tokens: 0,
                    max_tool_calls: 0,
                    max_expensive_tool_calls: 0,
                    max_operation_secs: 0,
                    model_tier_limit: 0,
                },
                _ => base,
            };
            info!(agent = agent_id, tier, "Compute quota adjusted for tier");
        }
    }

    /// Check if an LLM call is allowed for an agent
    pub fn check_llm_call(&mut self, agent_id: &str, estimated_tokens: u64) -> QuotaCheck {
        self.maybe_rotate(agent_id);
        let Some(agent) = self.agents.get(agent_id) else {
            return QuotaCheck::Allowed; // unregistered = no limits
        };

        if agent.usage.llm_calls >= agent.quota.max_llm_calls {
            return QuotaCheck::Denied {
                resource: "llm_calls".into(),
                used: agent.usage.llm_calls,
                limit: agent.quota.max_llm_calls,
            };
        }

        if agent.usage.tokens_used + estimated_tokens > agent.quota.max_tokens {
            return QuotaCheck::Denied {
                resource: "tokens".into(),
                used: agent.usage.tokens_used,
                limit: agent.quota.max_tokens,
            };
        }

        // Check for 80% warning
        let llm_pct = if agent.quota.max_llm_calls > 0 {
            (agent.usage.llm_calls as f64 / agent.quota.max_llm_calls as f64) * 100.0
        } else {
            0.0
        };
        if llm_pct >= 80.0 {
            return QuotaCheck::Warning {
                resource: "llm_calls".into(),
                used: agent.usage.llm_calls,
                limit: agent.quota.max_llm_calls,
            };
        }

        QuotaCheck::Allowed
    }

    /// Check if a tool call is allowed
    pub fn check_tool_call(&mut self, agent_id: &str, tool_name: &str) -> QuotaCheck {
        self.maybe_rotate(agent_id);
        let Some(agent) = self.agents.get(agent_id) else {
            return QuotaCheck::Allowed;
        };

        if agent.usage.tool_calls >= agent.quota.max_tool_calls {
            return QuotaCheck::Denied {
                resource: "tool_calls".into(),
                used: agent.usage.tool_calls,
                limit: agent.quota.max_tool_calls,
            };
        }

        let is_expensive = self.expensive_tools.iter().any(|t| t == tool_name);
        if is_expensive && agent.usage.expensive_tool_calls >= agent.quota.max_expensive_tool_calls
        {
            return QuotaCheck::Denied {
                resource: "expensive_tool_calls".into(),
                used: agent.usage.expensive_tool_calls,
                limit: agent.quota.max_expensive_tool_calls,
            };
        }

        QuotaCheck::Allowed
    }

    /// Record an LLM call
    pub fn record_llm_call(&mut self, agent_id: &str, tokens: u64) {
        self.maybe_rotate(agent_id);
        if let Some(agent) = self.agents.get_mut(agent_id) {
            agent.usage.llm_calls += 1;
            agent.usage.tokens_used += tokens;
            if agent.usage.llm_calls > agent.quota.max_llm_calls
                || agent.usage.tokens_used > agent.quota.max_tokens
            {
                agent.total_overages += 1;
                warn!(
                    agent = agent_id,
                    llm_calls = agent.usage.llm_calls,
                    tokens = agent.usage.tokens_used,
                    "Compute overage recorded"
                );
            }
            debug!(
                agent = agent_id,
                llm_calls = agent.usage.llm_calls,
                tokens = agent.usage.tokens_used,
                "LLM call recorded"
            );
        }
    }

    /// Record a tool call
    pub fn record_tool_call(&mut self, agent_id: &str, tool_name: &str) {
        self.maybe_rotate(agent_id);
        if let Some(agent) = self.agents.get_mut(agent_id) {
            agent.usage.tool_calls += 1;
            if self.expensive_tools.iter().any(|t| t == tool_name) {
                agent.usage.expensive_tool_calls += 1;
            }
        }
    }

    /// Record an error
    pub fn record_error(&mut self, agent_id: &str) {
        if let Some(agent) = self.agents.get_mut(agent_id) {
            agent.usage.errors += 1;
        }
    }

    /// Get an agent's compute state
    pub fn get_agent(&self, agent_id: &str) -> Option<&AgentCompute> {
        self.agents.get(agent_id)
    }

    /// Get all agents
    pub fn all_agents(&self) -> Vec<&AgentCompute> {
        self.agents.values().collect()
    }

    /// Get utilization across all agents
    pub fn ecosystem_utilization(&self) -> f64 {
        if self.agents.is_empty() {
            return 0.0;
        }
        let total: f64 = self.agents.values().map(|a| a.utilization()).sum();
        total / self.agents.len() as f64
    }

    /// Generate a compute report
    pub fn compute_report(&self) -> ComputeReport {
        let agents: Vec<AgentComputeSummary> = self
            .agents
            .values()
            .map(|a| AgentComputeSummary {
                agent_id: a.agent_id.clone(),
                utilization: a.utilization(),
                llm_calls_used: a.usage.llm_calls,
                llm_calls_limit: a.quota.max_llm_calls,
                tokens_used: a.usage.tokens_used,
                tokens_limit: a.quota.max_tokens,
                tool_calls_used: a.usage.tool_calls,
                tool_calls_limit: a.quota.max_tool_calls,
                errors: a.usage.errors,
                total_overages: a.total_overages,
                priority: a.priority,
            })
            .collect();

        let avg_util = self.ecosystem_utilization();

        ComputeReport {
            agents,
            avg_utilization: avg_util,
            timestamp: Utc::now(),
        }
    }

    // -- internals --

    fn maybe_rotate(&mut self, agent_id: &str) {
        if let Some(agent) = self.agents.get_mut(agent_id)
            && agent.window_expired()
        {
            debug!(agent = agent_id, "Compute window rotated");
            agent.rotate_window();
        }
    }
}

impl Default for ComputeProvisioner {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-agent summary for reports
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentComputeSummary {
    pub agent_id: String,
    pub utilization: f64,
    pub llm_calls_used: u64,
    pub llm_calls_limit: u64,
    pub tokens_used: u64,
    pub tokens_limit: u64,
    pub tool_calls_used: u64,
    pub tool_calls_limit: u64,
    pub errors: u64,
    pub total_overages: u64,
    pub priority: f64,
}

/// Ecosystem-wide compute report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeReport {
    pub agents: Vec<AgentComputeSummary>,
    pub avg_utilization: f64,
    pub timestamp: DateTime<Utc>,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_agent() {
        let mut prov = ComputeProvisioner::new();
        prov.register_agent("agent-1", 1.0);
        assert!(prov.get_agent("agent-1").is_some());
    }

    #[test]
    fn test_deregister_agent() {
        let mut prov = ComputeProvisioner::new();
        prov.register_agent("agent-1", 1.0);
        prov.deregister_agent("agent-1");
        assert!(prov.get_agent("agent-1").is_none());
    }

    #[test]
    fn test_llm_call_allowed() {
        let mut prov = ComputeProvisioner::new();
        prov.register_agent("agent-1", 1.0);
        let check = prov.check_llm_call("agent-1", 1000);
        assert!(check.is_allowed());
    }

    #[test]
    fn test_llm_call_denied_at_limit() {
        let mut prov = ComputeProvisioner::new().with_default_quota(ResourceQuota {
            max_llm_calls: 2,
            ..Default::default()
        });
        prov.register_agent("agent-1", 1.0);
        prov.record_llm_call("agent-1", 100);
        prov.record_llm_call("agent-1", 100);
        let check = prov.check_llm_call("agent-1", 100);
        assert_eq!(
            check,
            QuotaCheck::Denied {
                resource: "llm_calls".into(),
                used: 2,
                limit: 2,
            }
        );
    }

    #[test]
    fn test_token_limit_denied() {
        let mut prov = ComputeProvisioner::new().with_default_quota(ResourceQuota {
            max_tokens: 1000,
            ..Default::default()
        });
        prov.register_agent("agent-1", 1.0);
        prov.record_llm_call("agent-1", 800);
        let check = prov.check_llm_call("agent-1", 500);
        assert_eq!(
            check,
            QuotaCheck::Denied {
                resource: "tokens".into(),
                used: 800,
                limit: 1000,
            }
        );
    }

    #[test]
    fn test_tool_call_allowed() {
        let mut prov = ComputeProvisioner::new();
        prov.register_agent("agent-1", 1.0);
        let check = prov.check_tool_call("agent-1", "read_file");
        assert!(check.is_allowed());
    }

    #[test]
    fn test_tool_call_denied_at_limit() {
        let mut prov = ComputeProvisioner::new().with_default_quota(ResourceQuota {
            max_tool_calls: 1,
            ..Default::default()
        });
        prov.register_agent("agent-1", 1.0);
        prov.record_tool_call("agent-1", "read_file");
        let check = prov.check_tool_call("agent-1", "write_file");
        assert_eq!(
            check,
            QuotaCheck::Denied {
                resource: "tool_calls".into(),
                used: 1,
                limit: 1,
            }
        );
    }

    #[test]
    fn test_expensive_tool_denied() {
        let mut prov = ComputeProvisioner::new().with_default_quota(ResourceQuota {
            max_expensive_tool_calls: 1,
            ..Default::default()
        });
        prov.register_agent("agent-1", 1.0);
        prov.record_tool_call("agent-1", "spawn");
        let check = prov.check_tool_call("agent-1", "browser_navigate");
        assert_eq!(
            check,
            QuotaCheck::Denied {
                resource: "expensive_tool_calls".into(),
                used: 1,
                limit: 1,
            }
        );
    }

    #[test]
    fn test_non_expensive_tool_ok_when_expensive_exhausted() {
        let mut prov = ComputeProvisioner::new().with_default_quota(ResourceQuota {
            max_expensive_tool_calls: 0,
            max_tool_calls: 100,
            ..Default::default()
        });
        prov.register_agent("agent-1", 1.0);
        let check = prov.check_tool_call("agent-1", "read_file");
        assert!(check.is_allowed());
    }

    #[test]
    fn test_warning_at_80_percent() {
        let mut prov = ComputeProvisioner::new().with_default_quota(ResourceQuota {
            max_llm_calls: 10,
            ..Default::default()
        });
        prov.register_agent("agent-1", 1.0);
        for _ in 0..8 {
            prov.record_llm_call("agent-1", 10);
        }
        let check = prov.check_llm_call("agent-1", 10);
        match check {
            QuotaCheck::Warning {
                resource,
                used,
                limit,
            } => {
                assert_eq!(resource, "llm_calls");
                assert_eq!(used, 8);
                assert_eq!(limit, 10);
            }
            other => panic!("Expected Warning, got {:?}", other),
        }
    }

    #[test]
    fn test_priority_scales_quota() {
        let mut prov = ComputeProvisioner::new().with_default_quota(ResourceQuota {
            max_llm_calls: 100,
            ..Default::default()
        });
        prov.register_agent("high", 2.0);
        prov.register_agent("low", 0.5);
        assert_eq!(prov.get_agent("high").unwrap().quota.max_llm_calls, 200);
        assert_eq!(prov.get_agent("low").unwrap().quota.max_llm_calls, 50);
    }

    #[test]
    fn test_tier_quota_adjustment() {
        let mut prov = ComputeProvisioner::new();
        prov.register_agent("agent-1", 1.0);
        prov.apply_tier_quota("agent-1", "LowCompute");
        assert_eq!(prov.get_agent("agent-1").unwrap().quota.max_llm_calls, 30);
        assert_eq!(
            prov.get_agent("agent-1")
                .unwrap()
                .quota
                .max_expensive_tool_calls,
            0
        );
    }

    #[test]
    fn test_tier_dead_zeroes_all() {
        let mut prov = ComputeProvisioner::new();
        prov.register_agent("agent-1", 1.0);
        prov.apply_tier_quota("agent-1", "Dead");
        let q = &prov.get_agent("agent-1").unwrap().quota;
        assert_eq!(q.max_llm_calls, 0);
        assert_eq!(q.max_tokens, 0);
        assert_eq!(q.max_tool_calls, 0);
    }

    #[test]
    fn test_error_recording() {
        let mut prov = ComputeProvisioner::new();
        prov.register_agent("agent-1", 1.0);
        prov.record_error("agent-1");
        prov.record_error("agent-1");
        assert_eq!(prov.get_agent("agent-1").unwrap().usage.errors, 2);
    }

    #[test]
    fn test_overage_tracking() {
        let mut prov = ComputeProvisioner::new().with_default_quota(ResourceQuota {
            max_llm_calls: 1,
            ..Default::default()
        });
        prov.register_agent("agent-1", 1.0);
        prov.record_llm_call("agent-1", 100);
        prov.record_llm_call("agent-1", 100); // overage
        assert_eq!(prov.get_agent("agent-1").unwrap().total_overages, 1);
    }

    #[test]
    fn test_utilization() {
        let mut prov = ComputeProvisioner::new().with_default_quota(ResourceQuota {
            max_llm_calls: 100,
            max_tokens: 500_000,
            max_tool_calls: 200,
            ..Default::default()
        });
        prov.register_agent("agent-1", 1.0);
        for _ in 0..50 {
            prov.record_llm_call("agent-1", 1000);
        }
        let util = prov.get_agent("agent-1").unwrap().utilization();
        assert!((util - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_ecosystem_utilization() {
        let mut prov = ComputeProvisioner::new().with_default_quota(ResourceQuota {
            max_llm_calls: 100,
            max_tokens: 1_000_000,
            max_tool_calls: 200,
            ..Default::default()
        });
        prov.register_agent("a", 1.0);
        prov.register_agent("b", 1.0);
        for _ in 0..50 {
            prov.record_llm_call("a", 100);
        }
        // a = 50%, b = 0% → avg 25%
        let util = prov.ecosystem_utilization();
        assert!((util - 0.25).abs() < 0.01);
    }

    #[test]
    fn test_compute_report() {
        let mut prov = ComputeProvisioner::new();
        prov.register_agent("a", 1.0);
        prov.register_agent("b", 1.5);
        prov.record_llm_call("a", 1000);
        let report = prov.compute_report();
        assert_eq!(report.agents.len(), 2);
    }

    #[test]
    fn test_unregistered_agent_allowed() {
        let mut prov = ComputeProvisioner::new();
        let check = prov.check_llm_call("nobody", 5000);
        assert_eq!(check, QuotaCheck::Allowed);
    }

    #[test]
    fn test_custom_expensive_tools() {
        let mut prov = ComputeProvisioner::new()
            .with_expensive_tools(vec!["my_tool".into()])
            .with_default_quota(ResourceQuota {
                max_expensive_tool_calls: 0,
                max_tool_calls: 100,
                ..Default::default()
            });
        prov.register_agent("agent-1", 1.0);
        let check = prov.check_tool_call("agent-1", "my_tool");
        assert_eq!(
            check,
            QuotaCheck::Denied {
                resource: "expensive_tool_calls".into(),
                used: 0,
                limit: 0,
            }
        );
    }

    #[test]
    fn test_duplicate_register_idempotent() {
        let mut prov = ComputeProvisioner::new();
        prov.register_agent("agent-1", 1.0);
        prov.record_llm_call("agent-1", 100);
        prov.register_agent("agent-1", 2.0); // should NOT overwrite
        assert_eq!(prov.get_agent("agent-1").unwrap().usage.llm_calls, 1);
        assert!((prov.get_agent("agent-1").unwrap().priority - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_quota_scale() {
        let q = ResourceQuota::default();
        let scaled = q.scale(2.0);
        assert_eq!(scaled.max_llm_calls, 200);
        assert_eq!(scaled.max_tokens, 1_000_000);
        assert_eq!(scaled.max_operation_secs, q.max_operation_secs); // not scaled
    }

    #[test]
    fn test_all_agents() {
        let mut prov = ComputeProvisioner::new();
        prov.register_agent("a", 1.0);
        prov.register_agent("b", 1.0);
        prov.register_agent("c", 1.0);
        assert_eq!(prov.all_agents().len(), 3);
    }

    #[test]
    fn test_window_rotation() {
        let mut prov = ComputeProvisioner::new().with_default_quota(ResourceQuota {
            max_llm_calls: 5,
            ..Default::default()
        });
        prov.register_agent("agent-1", 1.0);
        prov.record_llm_call("agent-1", 100);
        prov.record_llm_call("agent-1", 100);
        assert_eq!(prov.get_agent("agent-1").unwrap().usage.llm_calls, 2);
        // Manually expire the window by backdating window_start
        if let Some(agent) = prov.agents.get_mut("agent-1") {
            agent.window_start = Utc::now() - Duration::hours(2);
        }
        // Next check triggers rotation — counters reset
        let _check = prov.check_llm_call("agent-1", 100);
        assert_eq!(prov.get_agent("agent-1").unwrap().usage.llm_calls, 0);
        assert_eq!(prov.get_agent("agent-1").unwrap().total_windows, 1);
    }
}
