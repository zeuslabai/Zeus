//! Resource Monitor for sandbox executions.
//!
//! Tracks and enforces resource limits across sandbox executions:
//!
//! - **ResourceMonitor** — aggregates resource usage across executions
//! - **ResourceSnapshot** — point-in-time resource measurement
//! - **ResourceBudget** — configurable per-agent or global resource budgets
//! - **UsageReport** — detailed usage breakdown for reporting

use std::collections::HashMap;

use chrono::{DateTime, Utc};

// ============================================================================
// Resource types
// ============================================================================

/// Type of resource being tracked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceType {
    CpuTime,
    Memory,
    WallClock,
    NetworkBytes,
    DiskBytes,
    Executions,
}

impl ResourceType {
    /// Unit label for display.
    pub fn unit(&self) -> &'static str {
        match self {
            ResourceType::CpuTime => "ms",
            ResourceType::Memory => "bytes",
            ResourceType::WallClock => "ms",
            ResourceType::NetworkBytes => "bytes",
            ResourceType::DiskBytes => "bytes",
            ResourceType::Executions => "count",
        }
    }

    /// Display name.
    pub fn name(&self) -> &'static str {
        match self {
            ResourceType::CpuTime => "CPU Time",
            ResourceType::Memory => "Memory",
            ResourceType::WallClock => "Wall Clock",
            ResourceType::NetworkBytes => "Network I/O",
            ResourceType::DiskBytes => "Disk I/O",
            ResourceType::Executions => "Executions",
        }
    }
}

// ============================================================================
// Resource snapshot
// ============================================================================

/// A single resource usage measurement from one execution.
#[derive(Debug, Clone)]
pub struct ResourceSnapshot {
    pub execution_id: String,
    pub agent_id: String,
    pub policy_id: Option<String>,
    pub cpu_time_ms: u64,
    pub memory_peak_bytes: u64,
    pub wall_clock_ms: u64,
    pub network_bytes: u64,
    pub disk_bytes: u64,
    pub recorded_at: DateTime<Utc>,
    pub success: bool,
}

impl ResourceSnapshot {
    /// Create a new snapshot.
    pub fn new(execution_id: &str, agent_id: &str) -> Self {
        Self {
            execution_id: execution_id.to_string(),
            agent_id: agent_id.to_string(),
            policy_id: None,
            cpu_time_ms: 0,
            memory_peak_bytes: 0,
            wall_clock_ms: 0,
            network_bytes: 0,
            disk_bytes: 0,
            recorded_at: Utc::now(),
            success: true,
        }
    }

    /// Set CPU time.
    pub fn with_cpu(mut self, ms: u64) -> Self {
        self.cpu_time_ms = ms;
        self
    }

    /// Set memory peak.
    pub fn with_memory(mut self, bytes: u64) -> Self {
        self.memory_peak_bytes = bytes;
        self
    }

    /// Set wall clock time.
    pub fn with_wall_clock(mut self, ms: u64) -> Self {
        self.wall_clock_ms = ms;
        self
    }

    /// Set network bytes.
    pub fn with_network(mut self, bytes: u64) -> Self {
        self.network_bytes = bytes;
        self
    }

    /// Set disk bytes.
    pub fn with_disk(mut self, bytes: u64) -> Self {
        self.disk_bytes = bytes;
        self
    }

    /// Set policy ID.
    pub fn with_policy(mut self, policy_id: &str) -> Self {
        self.policy_id = Some(policy_id.to_string());
        self
    }

    /// Mark as failed.
    pub fn with_failure(mut self) -> Self {
        self.success = false;
        self
    }
}

// ============================================================================
// Resource budget
// ============================================================================

/// A resource budget with limits and current usage.
#[derive(Debug, Clone)]
pub struct ResourceBudget {
    pub name: String,
    pub agent_id: Option<String>,
    pub limits: HashMap<ResourceType, u64>,
    pub usage: HashMap<ResourceType, u64>,
    pub period_start: DateTime<Utc>,
    pub period_secs: u64,
}

impl ResourceBudget {
    /// Create a new budget.
    pub fn new(name: &str, period_secs: u64) -> Self {
        Self {
            name: name.to_string(),
            agent_id: None,
            limits: HashMap::new(),
            usage: HashMap::new(),
            period_start: Utc::now(),
            period_secs,
        }
    }

    /// Create an agent-specific budget.
    pub fn for_agent(name: &str, agent_id: &str, period_secs: u64) -> Self {
        let mut budget = Self::new(name, period_secs);
        budget.agent_id = Some(agent_id.to_string());
        budget
    }

    /// Set a limit for a resource type.
    pub fn set_limit(&mut self, resource: ResourceType, limit: u64) {
        self.limits.insert(resource, limit);
    }

    /// Add usage for a resource type.
    pub fn add_usage(&mut self, resource: ResourceType, amount: u64) {
        *self.usage.entry(resource).or_insert(0) += amount;
    }

    /// Check if a resource type is within budget.
    pub fn is_within_limit(&self, resource: ResourceType) -> bool {
        match (self.limits.get(&resource), self.usage.get(&resource)) {
            (Some(&limit), Some(&used)) => used <= limit,
            (Some(_), None) => true, // no usage yet
            (None, _) => true,       // no limit set
        }
    }

    /// Check if all resources are within budget.
    pub fn is_within_all_limits(&self) -> bool {
        self.limits.keys().all(|r| self.is_within_limit(*r))
    }

    /// Get remaining budget for a resource type.
    pub fn remaining(&self, resource: ResourceType) -> Option<u64> {
        let limit = self.limits.get(&resource)?;
        let used = self.usage.get(&resource).unwrap_or(&0);
        Some(limit.saturating_sub(*used))
    }

    /// Usage percentage for a resource type (0.0–100.0).
    pub fn usage_pct(&self, resource: ResourceType) -> Option<f64> {
        let limit = *self.limits.get(&resource)? as f64;
        if limit == 0.0 {
            return Some(0.0);
        }
        let used = *self.usage.get(&resource).unwrap_or(&0) as f64;
        Some((used / limit) * 100.0)
    }

    /// Check if the budget period has expired.
    pub fn is_period_expired(&self) -> bool {
        let elapsed = Utc::now()
            .signed_duration_since(self.period_start)
            .num_seconds();
        elapsed >= self.period_secs as i64
    }

    /// Reset usage counters and start a new period.
    pub fn reset(&mut self) {
        self.usage.clear();
        self.period_start = Utc::now();
    }
}

// ============================================================================
// Usage report
// ============================================================================

/// Aggregated usage report.
#[derive(Debug, Clone)]
pub struct UsageReport {
    pub agent_id: Option<String>,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub total_executions: usize,
    pub successful_executions: usize,
    pub failed_executions: usize,
    pub total_cpu_ms: u64,
    pub total_memory_peak: u64,
    pub total_wall_clock_ms: u64,
    pub total_network_bytes: u64,
    pub total_disk_bytes: u64,
    pub avg_cpu_ms: f64,
    pub avg_wall_clock_ms: f64,
}

// ============================================================================
// Resource Monitor
// ============================================================================

/// Monitors and tracks resource usage across sandbox executions.
pub struct ResourceMonitor {
    snapshots: Vec<ResourceSnapshot>,
    budgets: HashMap<String, ResourceBudget>,
    max_history: usize,
}

impl ResourceMonitor {
    /// Create a new resource monitor.
    pub fn new(max_history: usize) -> Self {
        Self {
            snapshots: Vec::new(),
            budgets: HashMap::new(),
            max_history,
        }
    }

    /// Create with default settings (1000 snapshot history).
    pub fn with_defaults() -> Self {
        Self::new(1000)
    }

    // -- Recording ----------------------------------------------------------

    /// Record a resource snapshot from an execution.
    pub fn record(&mut self, snapshot: ResourceSnapshot) {
        // Update budgets
        for budget in self.budgets.values_mut() {
            let matches_agent = budget
                .agent_id
                .as_ref()
                .map(|a| a == &snapshot.agent_id)
                .unwrap_or(true); // global budgets match all agents

            if matches_agent {
                budget.add_usage(ResourceType::CpuTime, snapshot.cpu_time_ms);
                budget.add_usage(ResourceType::Memory, snapshot.memory_peak_bytes);
                budget.add_usage(ResourceType::WallClock, snapshot.wall_clock_ms);
                budget.add_usage(ResourceType::NetworkBytes, snapshot.network_bytes);
                budget.add_usage(ResourceType::DiskBytes, snapshot.disk_bytes);
                budget.add_usage(ResourceType::Executions, 1);
            }
        }

        self.snapshots.push(snapshot);

        // Trim history
        if self.snapshots.len() > self.max_history {
            let excess = self.snapshots.len() - self.max_history;
            self.snapshots.drain(..excess);
        }
    }

    // -- Budget management --------------------------------------------------

    /// Add a resource budget.
    pub fn add_budget(&mut self, budget: ResourceBudget) {
        self.budgets.insert(budget.name.clone(), budget);
    }

    /// Get a budget by name.
    pub fn get_budget(&self, name: &str) -> Option<&ResourceBudget> {
        self.budgets.get(name)
    }

    /// Get a mutable budget by name.
    pub fn get_budget_mut(&mut self, name: &str) -> Option<&mut ResourceBudget> {
        self.budgets.get_mut(name)
    }

    /// Check if an agent is within all applicable budgets.
    pub fn check_budgets(&self, agent_id: &str) -> Vec<BudgetViolation> {
        let mut violations = Vec::new();

        for budget in self.budgets.values() {
            let matches = budget
                .agent_id
                .as_ref()
                .map(|a| a == agent_id)
                .unwrap_or(true);

            if !matches {
                continue;
            }

            for (&resource, &limit) in &budget.limits {
                let used = budget.usage.get(&resource).unwrap_or(&0);
                if *used > limit {
                    violations.push(BudgetViolation {
                        budget_name: budget.name.clone(),
                        resource,
                        limit,
                        used: *used,
                    });
                }
            }
        }

        violations
    }

    /// Reset expired budgets.
    pub fn reset_expired_budgets(&mut self) -> Vec<String> {
        let mut reset = Vec::new();
        for budget in self.budgets.values_mut() {
            if budget.is_period_expired() {
                budget.reset();
                reset.push(budget.name.clone());
            }
        }
        reset
    }

    /// Remove a budget.
    pub fn remove_budget(&mut self, name: &str) -> bool {
        self.budgets.remove(name).is_some()
    }

    /// List all budget names.
    pub fn budget_names(&self) -> Vec<&str> {
        self.budgets.keys().map(|k| k.as_str()).collect()
    }

    // -- Queries ------------------------------------------------------------

    /// Total snapshots recorded.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    /// Snapshots for a specific agent.
    pub fn snapshots_for_agent(&self, agent_id: &str) -> Vec<&ResourceSnapshot> {
        self.snapshots
            .iter()
            .filter(|s| s.agent_id == agent_id)
            .collect()
    }

    /// Generate a usage report for a time range.
    pub fn report(
        &self,
        agent_id: Option<&str>,
        since: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> UsageReport {
        let matching: Vec<&ResourceSnapshot> = self
            .snapshots
            .iter()
            .filter(|s| {
                s.recorded_at >= since
                    && s.recorded_at <= until
                    && agent_id.map(|a| a == s.agent_id).unwrap_or(true)
            })
            .collect();

        let total = matching.len();
        let successful = matching.iter().filter(|s| s.success).count();
        let failed = total - successful;

        let total_cpu: u64 = matching.iter().map(|s| s.cpu_time_ms).sum();
        let total_mem: u64 = matching
            .iter()
            .map(|s| s.memory_peak_bytes)
            .max()
            .unwrap_or(0);
        let total_wall: u64 = matching.iter().map(|s| s.wall_clock_ms).sum();
        let total_net: u64 = matching.iter().map(|s| s.network_bytes).sum();
        let total_disk: u64 = matching.iter().map(|s| s.disk_bytes).sum();

        let avg_cpu = if total > 0 {
            total_cpu as f64 / total as f64
        } else {
            0.0
        };
        let avg_wall = if total > 0 {
            total_wall as f64 / total as f64
        } else {
            0.0
        };

        UsageReport {
            agent_id: agent_id.map(|s| s.to_string()),
            period_start: since,
            period_end: until,
            total_executions: total,
            successful_executions: successful,
            failed_executions: failed,
            total_cpu_ms: total_cpu,
            total_memory_peak: total_mem,
            total_wall_clock_ms: total_wall,
            total_network_bytes: total_net,
            total_disk_bytes: total_disk,
            avg_cpu_ms: avg_cpu,
            avg_wall_clock_ms: avg_wall,
        }
    }

    /// Get the top N agents by total CPU usage.
    pub fn top_agents_by_cpu(&self, n: usize) -> Vec<(String, u64)> {
        let mut agent_cpu: HashMap<String, u64> = HashMap::new();
        for s in &self.snapshots {
            *agent_cpu.entry(s.agent_id.clone()).or_insert(0) += s.cpu_time_ms;
        }
        let mut sorted: Vec<_> = agent_cpu.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        sorted.truncate(n);
        sorted
    }

    /// Get the top N agents by total executions.
    pub fn top_agents_by_executions(&self, n: usize) -> Vec<(String, usize)> {
        let mut agent_count: HashMap<String, usize> = HashMap::new();
        for s in &self.snapshots {
            *agent_count.entry(s.agent_id.clone()).or_insert(0) += 1;
        }
        let mut sorted: Vec<_> = agent_count.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        sorted.truncate(n);
        sorted
    }

    /// Overall success rate.
    pub fn success_rate(&self) -> f64 {
        if self.snapshots.is_empty() {
            return 1.0;
        }
        let success_count = self.snapshots.iter().filter(|s| s.success).count();
        success_count as f64 / self.snapshots.len() as f64
    }
}

impl Default for ResourceMonitor {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// A budget violation for reporting.
#[derive(Debug, Clone)]
pub struct BudgetViolation {
    pub budget_name: String,
    pub resource: ResourceType,
    pub limit: u64,
    pub used: u64,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_snapshot(exec_id: &str, agent: &str) -> ResourceSnapshot {
        ResourceSnapshot::new(exec_id, agent)
            .with_cpu(100)
            .with_memory(1024 * 1024)
            .with_wall_clock(200)
    }

    // -- ResourceType -------------------------------------------------------

    #[test]
    fn test_resource_type_unit() {
        assert_eq!(ResourceType::CpuTime.unit(), "ms");
        assert_eq!(ResourceType::Memory.unit(), "bytes");
        assert_eq!(ResourceType::Executions.unit(), "count");
    }

    #[test]
    fn test_resource_type_name() {
        assert_eq!(ResourceType::CpuTime.name(), "CPU Time");
        assert_eq!(ResourceType::Memory.name(), "Memory");
        assert_eq!(ResourceType::NetworkBytes.name(), "Network I/O");
    }

    // -- ResourceSnapshot ---------------------------------------------------

    #[test]
    fn test_snapshot_new() {
        let s = ResourceSnapshot::new("exec-1", "agent-1");
        assert_eq!(s.execution_id, "exec-1");
        assert_eq!(s.agent_id, "agent-1");
        assert!(s.success);
        assert_eq!(s.cpu_time_ms, 0);
    }

    #[test]
    fn test_snapshot_builder() {
        let s = ResourceSnapshot::new("e1", "a1")
            .with_cpu(50)
            .with_memory(2048)
            .with_wall_clock(100)
            .with_network(512)
            .with_disk(1024)
            .with_policy("pol-1")
            .with_failure();
        assert_eq!(s.cpu_time_ms, 50);
        assert_eq!(s.memory_peak_bytes, 2048);
        assert_eq!(s.wall_clock_ms, 100);
        assert_eq!(s.network_bytes, 512);
        assert_eq!(s.disk_bytes, 1024);
        assert_eq!(s.policy_id.as_deref(), Some("pol-1"));
        assert!(!s.success);
    }

    // -- ResourceBudget -----------------------------------------------------

    #[test]
    fn test_budget_new() {
        let b = ResourceBudget::new("global", 3600);
        assert_eq!(b.name, "global");
        assert_eq!(b.period_secs, 3600);
        assert!(b.agent_id.is_none());
        assert!(b.limits.is_empty());
    }

    #[test]
    fn test_budget_for_agent() {
        let b = ResourceBudget::for_agent("agent-budget", "agent-1", 3600);
        assert_eq!(b.agent_id.as_deref(), Some("agent-1"));
    }

    #[test]
    fn test_budget_set_limit_and_check() {
        let mut b = ResourceBudget::new("test", 3600);
        b.set_limit(ResourceType::CpuTime, 1000);
        assert!(b.is_within_limit(ResourceType::CpuTime));

        b.add_usage(ResourceType::CpuTime, 500);
        assert!(b.is_within_limit(ResourceType::CpuTime));

        b.add_usage(ResourceType::CpuTime, 600);
        assert!(!b.is_within_limit(ResourceType::CpuTime));
    }

    #[test]
    fn test_budget_remaining() {
        let mut b = ResourceBudget::new("test", 3600);
        b.set_limit(ResourceType::Executions, 10);
        assert_eq!(b.remaining(ResourceType::Executions), Some(10));

        b.add_usage(ResourceType::Executions, 3);
        assert_eq!(b.remaining(ResourceType::Executions), Some(7));

        // No limit set for CpuTime
        assert_eq!(b.remaining(ResourceType::CpuTime), None);
    }

    #[test]
    fn test_budget_usage_pct() {
        let mut b = ResourceBudget::new("test", 3600);
        b.set_limit(ResourceType::CpuTime, 1000);
        b.add_usage(ResourceType::CpuTime, 250);
        let pct = b.usage_pct(ResourceType::CpuTime).unwrap();
        assert!((pct - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_budget_is_within_all_limits() {
        let mut b = ResourceBudget::new("test", 3600);
        b.set_limit(ResourceType::CpuTime, 1000);
        b.set_limit(ResourceType::Executions, 10);
        assert!(b.is_within_all_limits());

        b.add_usage(ResourceType::Executions, 15);
        assert!(!b.is_within_all_limits());
    }

    #[test]
    fn test_budget_reset() {
        let mut b = ResourceBudget::new("test", 3600);
        b.set_limit(ResourceType::CpuTime, 1000);
        b.add_usage(ResourceType::CpuTime, 500);
        b.reset();
        assert_eq!(b.remaining(ResourceType::CpuTime), Some(1000));
    }

    #[test]
    fn test_budget_period_expired() {
        let mut b = ResourceBudget::new("test", 3600);
        assert!(!b.is_period_expired());

        // Backdate period start
        b.period_start = Utc::now() - chrono::Duration::seconds(7200);
        assert!(b.is_period_expired());
    }

    // -- ResourceMonitor ----------------------------------------------------

    #[test]
    fn test_monitor_new() {
        let m = ResourceMonitor::with_defaults();
        assert_eq!(m.snapshot_count(), 0);
        assert!((m.success_rate() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_monitor_record() {
        let mut m = ResourceMonitor::with_defaults();
        m.record(test_snapshot("e1", "a1"));
        m.record(test_snapshot("e2", "a1"));
        assert_eq!(m.snapshot_count(), 2);
    }

    #[test]
    fn test_monitor_history_limit() {
        let mut m = ResourceMonitor::new(3);
        for i in 0..5 {
            m.record(test_snapshot(&format!("e{i}"), "a1"));
        }
        assert_eq!(m.snapshot_count(), 3);
    }

    #[test]
    fn test_monitor_snapshots_for_agent() {
        let mut m = ResourceMonitor::with_defaults();
        m.record(test_snapshot("e1", "a1"));
        m.record(test_snapshot("e2", "a2"));
        m.record(test_snapshot("e3", "a1"));
        assert_eq!(m.snapshots_for_agent("a1").len(), 2);
        assert_eq!(m.snapshots_for_agent("a2").len(), 1);
        assert_eq!(m.snapshots_for_agent("a3").len(), 0);
    }

    #[test]
    fn test_monitor_success_rate() {
        let mut m = ResourceMonitor::with_defaults();
        m.record(test_snapshot("e1", "a1"));
        m.record(test_snapshot("e2", "a1").with_failure());
        assert!((m.success_rate() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_monitor_top_agents_by_cpu() {
        let mut m = ResourceMonitor::with_defaults();
        m.record(ResourceSnapshot::new("e1", "a1").with_cpu(100));
        m.record(ResourceSnapshot::new("e2", "a1").with_cpu(200));
        m.record(ResourceSnapshot::new("e3", "a2").with_cpu(500));

        let top = m.top_agents_by_cpu(2);
        assert_eq!(top[0].0, "a2");
        assert_eq!(top[0].1, 500);
        assert_eq!(top[1].0, "a1");
        assert_eq!(top[1].1, 300);
    }

    #[test]
    fn test_monitor_top_agents_by_executions() {
        let mut m = ResourceMonitor::with_defaults();
        m.record(test_snapshot("e1", "a1"));
        m.record(test_snapshot("e2", "a1"));
        m.record(test_snapshot("e3", "a1"));
        m.record(test_snapshot("e4", "a2"));

        let top = m.top_agents_by_executions(2);
        assert_eq!(top[0].0, "a1");
        assert_eq!(top[0].1, 3);
    }

    // -- Budget integration -------------------------------------------------

    #[test]
    fn test_monitor_budget_tracking() {
        let mut m = ResourceMonitor::with_defaults();
        let mut budget = ResourceBudget::new("global", 3600);
        budget.set_limit(ResourceType::Executions, 5);
        budget.set_limit(ResourceType::CpuTime, 1000);
        m.add_budget(budget);

        m.record(ResourceSnapshot::new("e1", "a1").with_cpu(100));
        m.record(ResourceSnapshot::new("e2", "a1").with_cpu(200));

        let b = m.get_budget("global").unwrap();
        assert_eq!(b.remaining(ResourceType::Executions), Some(3));
        assert_eq!(b.remaining(ResourceType::CpuTime), Some(700));
    }

    #[test]
    fn test_monitor_budget_agent_specific() {
        let mut m = ResourceMonitor::with_defaults();
        let mut budget = ResourceBudget::for_agent("a1-budget", "a1", 3600);
        budget.set_limit(ResourceType::Executions, 10);
        m.add_budget(budget);

        // a1 execution should count
        m.record(test_snapshot("e1", "a1"));
        // a2 execution should NOT count
        m.record(test_snapshot("e2", "a2"));

        let b = m.get_budget("a1-budget").unwrap();
        assert_eq!(b.remaining(ResourceType::Executions), Some(9));
    }

    #[test]
    fn test_monitor_check_budgets() {
        let mut m = ResourceMonitor::with_defaults();
        let mut budget = ResourceBudget::new("global", 3600);
        budget.set_limit(ResourceType::Executions, 2);
        m.add_budget(budget);

        m.record(test_snapshot("e1", "a1"));
        m.record(test_snapshot("e2", "a1"));
        m.record(test_snapshot("e3", "a1"));

        let violations = m.check_budgets("a1");
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].resource, ResourceType::Executions);
        assert_eq!(violations[0].limit, 2);
        assert_eq!(violations[0].used, 3);
    }

    #[test]
    fn test_monitor_no_violations() {
        let mut m = ResourceMonitor::with_defaults();
        let mut budget = ResourceBudget::new("global", 3600);
        budget.set_limit(ResourceType::Executions, 100);
        m.add_budget(budget);

        m.record(test_snapshot("e1", "a1"));
        assert!(m.check_budgets("a1").is_empty());
    }

    #[test]
    fn test_monitor_remove_budget() {
        let mut m = ResourceMonitor::with_defaults();
        m.add_budget(ResourceBudget::new("test", 3600));
        assert!(m.remove_budget("test"));
        assert!(!m.remove_budget("nonexistent"));
    }

    #[test]
    fn test_monitor_budget_names() {
        let mut m = ResourceMonitor::with_defaults();
        m.add_budget(ResourceBudget::new("budget-a", 3600));
        m.add_budget(ResourceBudget::new("budget-b", 3600));
        let names = m.budget_names();
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn test_monitor_reset_expired() {
        let mut m = ResourceMonitor::with_defaults();
        let mut budget = ResourceBudget::new("hourly", 3600);
        budget.set_limit(ResourceType::Executions, 100);
        budget.add_usage(ResourceType::Executions, 50);
        // Backdate to expire
        budget.period_start = Utc::now() - chrono::Duration::seconds(7200);
        m.add_budget(budget);

        let reset = m.reset_expired_budgets();
        assert_eq!(reset.len(), 1);
        assert_eq!(reset[0], "hourly");

        let b = m.get_budget("hourly").unwrap();
        assert_eq!(b.remaining(ResourceType::Executions), Some(100));
    }

    // -- Report -------------------------------------------------------------

    #[test]
    fn test_monitor_report() {
        let mut m = ResourceMonitor::with_defaults();
        m.record(
            ResourceSnapshot::new("e1", "a1")
                .with_cpu(100)
                .with_wall_clock(200),
        );
        m.record(
            ResourceSnapshot::new("e2", "a1")
                .with_cpu(300)
                .with_wall_clock(400),
        );
        m.record(
            ResourceSnapshot::new("e3", "a1")
                .with_cpu(200)
                .with_wall_clock(300)
                .with_failure(),
        );

        let since = Utc::now() - chrono::Duration::seconds(60);
        let until = Utc::now() + chrono::Duration::seconds(60);

        let report = m.report(Some("a1"), since, until);
        assert_eq!(report.total_executions, 3);
        assert_eq!(report.successful_executions, 2);
        assert_eq!(report.failed_executions, 1);
        assert_eq!(report.total_cpu_ms, 600);
        assert_eq!(report.total_wall_clock_ms, 900);
        assert!((report.avg_cpu_ms - 200.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_monitor_report_empty() {
        let m = ResourceMonitor::with_defaults();
        let since = Utc::now() - chrono::Duration::seconds(60);
        let until = Utc::now() + chrono::Duration::seconds(60);
        let report = m.report(None, since, until);
        assert_eq!(report.total_executions, 0);
        assert!((report.avg_cpu_ms - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_monitor_report_agent_filter() {
        let mut m = ResourceMonitor::with_defaults();
        m.record(ResourceSnapshot::new("e1", "a1").with_cpu(100));
        m.record(ResourceSnapshot::new("e2", "a2").with_cpu(200));

        let since = Utc::now() - chrono::Duration::seconds(60);
        let until = Utc::now() + chrono::Duration::seconds(60);

        let report = m.report(Some("a1"), since, until);
        assert_eq!(report.total_executions, 1);
        assert_eq!(report.total_cpu_ms, 100);
    }
}
