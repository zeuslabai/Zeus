//! WASM Execution Metrics
//!
//! Tracks per-execution and per-skill resource usage for WASM plugins.
//! `WasmMetricsCollector` wraps `WasmSandbox` execution and captures
//! cpu_time, peak_memory, wall_clock, and instruction counts.
//! `MetricsReport` provides per-skill aggregation (avg, p95, max).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::wasm_sandbox::{WasmExecutionResult, WasmSandbox};

// ---------------------------------------------------------------------------
// Core metrics struct
// ---------------------------------------------------------------------------

/// Resource usage captured for a single WASM execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmExecutionMetrics {
    /// CPU time in milliseconds (approximated from fuel consumed)
    pub cpu_time_ms: f64,
    /// Peak memory usage in bytes during execution
    pub peak_memory_bytes: u64,
    /// Wall-clock time in milliseconds
    pub wall_clock_ms: f64,
    /// Instruction count (fuel consumed as proxy)
    pub instructions_count: u64,
}

impl WasmExecutionMetrics {
    /// Create an empty metrics instance (all zeros)
    pub fn zero() -> Self {
        Self {
            cpu_time_ms: 0.0,
            peak_memory_bytes: 0,
            wall_clock_ms: 0.0,
            instructions_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Execution record — metrics + result together
// ---------------------------------------------------------------------------

/// A single recorded execution with its metrics and result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRecord {
    /// Skill/plugin name
    pub skill_name: String,
    /// Captured resource metrics
    pub metrics: WasmExecutionMetrics,
    /// Execution outcome
    pub result: WasmExecutionResult,
    /// Unix timestamp (seconds) when the execution started
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Aggregated stats for a single skill
// ---------------------------------------------------------------------------

/// Aggregated statistics for a single metric dimension
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricAgg {
    pub avg: f64,
    pub p95: f64,
    pub max: f64,
    pub min: f64,
    pub count: usize,
}

impl MetricAgg {
    /// Compute aggregation from a slice of f64 values
    pub fn from_values(values: &[f64]) -> Self {
        if values.is_empty() {
            return Self {
                avg: 0.0,
                p95: 0.0,
                max: 0.0,
                min: 0.0,
                count: 0,
            };
        }

        let count = values.len();
        let sum: f64 = values.iter().sum();
        let avg = sum / count as f64;

        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let max = sorted[count - 1];
        let min = sorted[0];

        // p95: index = ceil(0.95 * count) - 1
        let p95_idx = ((0.95 * count as f64).ceil() as usize)
            .saturating_sub(1)
            .min(count - 1);
        let p95 = sorted[p95_idx];

        Self {
            avg,
            p95,
            max,
            min,
            count,
        }
    }
}

/// Per-skill aggregated metrics report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetricsReport {
    pub skill_name: String,
    pub cpu_time: MetricAgg,
    pub peak_memory: MetricAgg,
    pub wall_clock: MetricAgg,
    pub instructions: MetricAgg,
    pub success_count: usize,
    pub failure_count: usize,
}

/// Full metrics report across all skills
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsReport {
    /// Per-skill breakdowns
    pub skills: Vec<SkillMetricsReport>,
    /// Total executions tracked
    pub total_executions: usize,
    /// Total wall-clock time across all executions
    pub total_wall_clock_ms: f64,
}

// ---------------------------------------------------------------------------
// Metrics collector
// ---------------------------------------------------------------------------

/// Thread-safe metrics collector that wraps WasmSandbox execution
/// and records resource usage per invocation.
#[derive(Clone)]
pub struct WasmMetricsCollector {
    /// Execution records keyed by skill name
    records: Arc<RwLock<HashMap<String, Vec<ExecutionRecord>>>>,
    /// Maximum number of records to keep per skill (ring buffer)
    max_records_per_skill: usize,
}

impl WasmMetricsCollector {
    /// Create a new collector with default capacity (1000 records per skill)
    pub fn new() -> Self {
        Self {
            records: Arc::new(RwLock::new(HashMap::new())),
            max_records_per_skill: 1000,
        }
    }

    /// Create a collector with custom per-skill capacity
    pub fn with_capacity(max_records_per_skill: usize) -> Self {
        Self {
            records: Arc::new(RwLock::new(HashMap::new())),
            max_records_per_skill,
        }
    }

    /// Execute a skill through the sandbox and record metrics
    pub async fn execute_with_metrics(
        &self,
        sandbox: &WasmSandbox,
        skill_name: &str,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Result<(WasmExecutionResult, WasmExecutionMetrics), crate::wasm_sandbox::WasmError> {
        let wall_start = Instant::now();

        // Execute through sandbox
        let result = sandbox.execute(skill_name, args, env).await?;

        let wall_clock_ms = wall_start.elapsed().as_secs_f64() * 1000.0;

        // Build metrics from result + timing
        let metrics = WasmExecutionMetrics {
            cpu_time_ms: result.duration_ms as f64,
            peak_memory_bytes: result.memory_used,
            wall_clock_ms,
            instructions_count: estimate_instructions(result.duration_ms),
        };

        // Record
        let record = ExecutionRecord {
            skill_name: skill_name.to_string(),
            metrics: metrics.clone(),
            result: result.clone(),
            timestamp: now_epoch(),
        };

        self.add_record(record);

        Ok((result, metrics))
    }

    /// Manually record an execution (for external callers that run their own sandbox)
    pub fn record_execution(
        &self,
        skill_name: &str,
        metrics: WasmExecutionMetrics,
        result: WasmExecutionResult,
    ) {
        let record = ExecutionRecord {
            skill_name: skill_name.to_string(),
            metrics,
            result,
            timestamp: now_epoch(),
        };
        self.add_record(record);
    }

    fn add_record(&self, record: ExecutionRecord) {
        let mut records = self.records.write().unwrap();
        let entries = records.entry(record.skill_name.clone()).or_default();

        entries.push(record);

        // Ring buffer: drop oldest if over capacity
        if entries.len() > self.max_records_per_skill {
            let excess = entries.len() - self.max_records_per_skill;
            entries.drain(0..excess);
        }
    }

    /// Generate a metrics report for all tracked skills
    pub fn report(&self) -> MetricsReport {
        let records = self.records.read().unwrap();
        let mut skills = Vec::new();
        let mut total_executions = 0;
        let mut total_wall_clock = 0.0;

        for (name, entries) in records.iter() {
            if entries.is_empty() {
                continue;
            }

            let cpu_vals: Vec<f64> = entries.iter().map(|e| e.metrics.cpu_time_ms).collect();
            let mem_vals: Vec<f64> = entries
                .iter()
                .map(|e| e.metrics.peak_memory_bytes as f64)
                .collect();
            let wall_vals: Vec<f64> = entries.iter().map(|e| e.metrics.wall_clock_ms).collect();
            let instr_vals: Vec<f64> = entries
                .iter()
                .map(|e| e.metrics.instructions_count as f64)
                .collect();

            let success_count = entries.iter().filter(|e| e.result.exit_code == 0).count();
            let failure_count = entries.len() - success_count;

            total_executions += entries.len();
            total_wall_clock += wall_vals.iter().sum::<f64>();

            skills.push(SkillMetricsReport {
                skill_name: name.clone(),
                cpu_time: MetricAgg::from_values(&cpu_vals),
                peak_memory: MetricAgg::from_values(&mem_vals),
                wall_clock: MetricAgg::from_values(&wall_vals),
                instructions: MetricAgg::from_values(&instr_vals),
                success_count,
                failure_count,
            });
        }

        // Sort by total wall clock (heaviest first)
        skills.sort_by(|a, b| {
            let a_total: f64 = a.wall_clock.avg * a.wall_clock.count as f64;
            let b_total: f64 = b.wall_clock.avg * b.wall_clock.count as f64;
            b_total
                .partial_cmp(&a_total)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        MetricsReport {
            skills,
            total_executions,
            total_wall_clock_ms: total_wall_clock,
        }
    }

    /// Get metrics for a specific skill
    pub fn skill_report(&self, skill_name: &str) -> Option<SkillMetricsReport> {
        let records = self.records.read().unwrap();
        let entries = records.get(skill_name)?;

        if entries.is_empty() {
            return None;
        }

        let cpu_vals: Vec<f64> = entries.iter().map(|e| e.metrics.cpu_time_ms).collect();
        let mem_vals: Vec<f64> = entries
            .iter()
            .map(|e| e.metrics.peak_memory_bytes as f64)
            .collect();
        let wall_vals: Vec<f64> = entries.iter().map(|e| e.metrics.wall_clock_ms).collect();
        let instr_vals: Vec<f64> = entries
            .iter()
            .map(|e| e.metrics.instructions_count as f64)
            .collect();

        let success_count = entries.iter().filter(|e| e.result.exit_code == 0).count();
        let failure_count = entries.len() - success_count;

        Some(SkillMetricsReport {
            skill_name: skill_name.to_string(),
            cpu_time: MetricAgg::from_values(&cpu_vals),
            peak_memory: MetricAgg::from_values(&mem_vals),
            wall_clock: MetricAgg::from_values(&wall_vals),
            instructions: MetricAgg::from_values(&instr_vals),
            success_count,
            failure_count,
        })
    }

    /// Get raw execution records for a skill
    pub fn records_for(&self, skill_name: &str) -> Vec<ExecutionRecord> {
        let records = self.records.read().unwrap();
        records.get(skill_name).cloned().unwrap_or_default()
    }

    /// Clear all recorded metrics
    pub fn clear(&self) {
        let mut records = self.records.write().unwrap();
        records.clear();
    }

    /// Total number of tracked executions across all skills
    pub fn total_executions(&self) -> usize {
        let records = self.records.read().unwrap();
        records.values().map(|v| v.len()).sum()
    }
}

impl Default for WasmMetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Estimate instruction count from execution duration
/// (rough approximation: ~1M instructions per ms on modern hardware)
fn estimate_instructions(duration_ms: u64) -> u64 {
    duration_ms.saturating_mul(1_000_000)
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm_sandbox::WasmExecutionResult;

    fn sample_result(exit_code: i32, duration_ms: u64, memory: u64) -> WasmExecutionResult {
        WasmExecutionResult {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code,
            duration_ms,
            memory_used: memory,
        }
    }

    fn sample_metrics(cpu: f64, mem: u64, wall: f64, instr: u64) -> WasmExecutionMetrics {
        WasmExecutionMetrics {
            cpu_time_ms: cpu,
            peak_memory_bytes: mem,
            wall_clock_ms: wall,
            instructions_count: instr,
        }
    }

    #[test]
    fn test_metrics_zero() {
        let m = WasmExecutionMetrics::zero();
        assert_eq!(m.cpu_time_ms, 0.0);
        assert_eq!(m.peak_memory_bytes, 0);
        assert_eq!(m.wall_clock_ms, 0.0);
        assert_eq!(m.instructions_count, 0);
    }

    #[test]
    fn test_metrics_serialization() {
        let m = sample_metrics(12.5, 65536, 15.0, 12_500_000);
        let json = serde_json::to_string(&m).unwrap();
        let parsed: WasmExecutionMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.cpu_time_ms, 12.5);
        assert_eq!(parsed.peak_memory_bytes, 65536);
        assert_eq!(parsed.instructions_count, 12_500_000);
    }

    #[test]
    fn test_metric_agg_from_values() {
        let values = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        let agg = MetricAgg::from_values(&values);
        assert_eq!(agg.count, 5);
        assert_eq!(agg.avg, 30.0);
        assert_eq!(agg.min, 10.0);
        assert_eq!(agg.max, 50.0);
        assert_eq!(agg.p95, 50.0); // ceil(0.95*5)=5, idx=4 → 50.0
    }

    #[test]
    fn test_metric_agg_single_value() {
        let agg = MetricAgg::from_values(&[42.0]);
        assert_eq!(agg.count, 1);
        assert_eq!(agg.avg, 42.0);
        assert_eq!(agg.min, 42.0);
        assert_eq!(agg.max, 42.0);
        assert_eq!(agg.p95, 42.0);
    }

    #[test]
    fn test_metric_agg_empty() {
        let agg = MetricAgg::from_values(&[]);
        assert_eq!(agg.count, 0);
        assert_eq!(agg.avg, 0.0);
    }

    #[test]
    fn test_collector_record_and_report() {
        let collector = WasmMetricsCollector::new();

        collector.record_execution(
            "skill-a",
            sample_metrics(10.0, 1024, 12.0, 10_000_000),
            sample_result(0, 10, 1024),
        );
        collector.record_execution(
            "skill-a",
            sample_metrics(20.0, 2048, 22.0, 20_000_000),
            sample_result(0, 20, 2048),
        );
        collector.record_execution(
            "skill-b",
            sample_metrics(5.0, 512, 6.0, 5_000_000),
            sample_result(1, 5, 512),
        );

        assert_eq!(collector.total_executions(), 3);

        let report = collector.report();
        assert_eq!(report.total_executions, 3);
        assert_eq!(report.skills.len(), 2);

        let skill_a = report
            .skills
            .iter()
            .find(|s| s.skill_name == "skill-a")
            .unwrap();
        assert_eq!(skill_a.cpu_time.count, 2);
        assert_eq!(skill_a.cpu_time.avg, 15.0);
        assert_eq!(skill_a.success_count, 2);
        assert_eq!(skill_a.failure_count, 0);
    }

    #[test]
    fn test_collector_skill_report() {
        let collector = WasmMetricsCollector::new();

        collector.record_execution(
            "my-skill",
            sample_metrics(50.0, 4096, 55.0, 50_000_000),
            sample_result(0, 50, 4096),
        );

        let report = collector.skill_report("my-skill").unwrap();
        assert_eq!(report.skill_name, "my-skill");
        assert_eq!(report.cpu_time.max, 50.0);
        assert_eq!(report.success_count, 1);

        assert!(collector.skill_report("nonexistent").is_none());
    }

    #[test]
    fn test_collector_ring_buffer() {
        let collector = WasmMetricsCollector::with_capacity(3);

        for i in 0..5 {
            collector.record_execution(
                "ring-skill",
                sample_metrics(i as f64, i * 100, i as f64 + 0.5, i * 1_000_000),
                sample_result(0, i as u64, i * 100),
            );
        }

        let records = collector.records_for("ring-skill");
        assert_eq!(records.len(), 3); // Only last 3 kept
        // First record should be i=2 (oldest surviving)
        assert_eq!(records[0].metrics.cpu_time_ms, 2.0);
    }

    #[test]
    fn test_collector_clear() {
        let collector = WasmMetricsCollector::new();

        collector.record_execution(
            "skill-x",
            sample_metrics(1.0, 100, 1.5, 1_000_000),
            sample_result(0, 1, 100),
        );

        assert_eq!(collector.total_executions(), 1);
        collector.clear();
        assert_eq!(collector.total_executions(), 0);
    }

    #[test]
    fn test_collector_success_failure_count() {
        let collector = WasmMetricsCollector::new();

        collector.record_execution(
            "flaky-skill",
            sample_metrics(10.0, 1024, 12.0, 10_000_000),
            sample_result(0, 10, 1024),
        );
        collector.record_execution(
            "flaky-skill",
            sample_metrics(10.0, 1024, 12.0, 10_000_000),
            sample_result(1, 10, 1024), // failure
        );
        collector.record_execution(
            "flaky-skill",
            sample_metrics(10.0, 1024, 12.0, 10_000_000),
            sample_result(0, 10, 1024),
        );

        let report = collector.skill_report("flaky-skill").unwrap();
        assert_eq!(report.success_count, 2);
        assert_eq!(report.failure_count, 1);
    }

    #[test]
    fn test_report_sorted_by_heaviest() {
        let collector = WasmMetricsCollector::new();

        // skill-light: 1 exec, 5ms wall
        collector.record_execution(
            "skill-light",
            sample_metrics(5.0, 256, 5.0, 5_000_000),
            sample_result(0, 5, 256),
        );

        // skill-heavy: 1 exec, 500ms wall
        collector.record_execution(
            "skill-heavy",
            sample_metrics(500.0, 8192, 500.0, 500_000_000),
            sample_result(0, 500, 8192),
        );

        let report = collector.report();
        assert_eq!(report.skills[0].skill_name, "skill-heavy");
        assert_eq!(report.skills[1].skill_name, "skill-light");
    }

    #[test]
    fn test_estimate_instructions() {
        assert_eq!(estimate_instructions(0), 0);
        assert_eq!(estimate_instructions(1), 1_000_000);
        assert_eq!(estimate_instructions(100), 100_000_000);
    }

    #[test]
    fn test_execution_record_timestamp() {
        let collector = WasmMetricsCollector::new();

        collector.record_execution(
            "ts-skill",
            sample_metrics(1.0, 100, 1.0, 1_000_000),
            sample_result(0, 1, 100),
        );

        let records = collector.records_for("ts-skill");
        assert_eq!(records.len(), 1);
        assert!(records[0].timestamp > 0); // Should be a valid epoch
    }

    #[test]
    fn test_p95_large_dataset() {
        // 20 values: 1..=20
        let values: Vec<f64> = (1..=20).map(|i| i as f64).collect();
        let agg = MetricAgg::from_values(&values);
        assert_eq!(agg.count, 20);
        assert_eq!(agg.avg, 10.5);
        assert_eq!(agg.min, 1.0);
        assert_eq!(agg.max, 20.0);
        // p95 idx = ceil(0.95*20)-1 = ceil(19)-1 = 18 → values[18] = 19.0
        assert_eq!(agg.p95, 19.0);
    }
}
