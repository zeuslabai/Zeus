//! Self-Monitoring Engine
//!
//! Tracks system health metrics, detects anomalies, and suggests
//! corrective actions for self-improvement. All metric storage is
//! in-memory using ring buffers for fast, bounded operation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use tracing::{debug, warn};

// ============================================================================
// Types
// ============================================================================

/// Overall health status of the system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    /// Everything is operating within normal parameters.
    Healthy,
    /// The system is functional but one or more metrics are outside normal bounds.
    Degraded(String),
    /// The system has significant issues that need attention.
    Unhealthy(String),
}

/// The kind of metric being recorded.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MetricType {
    /// Latency of LLM inference calls (milliseconds).
    LlmLatency,
    /// Success rate of tool executions (0.0 - 1.0).
    ToolSuccessRate,
    /// Rate of errors encountered (0.0 - 1.0).
    ErrorRate,
    /// Memory usage of the process (bytes or percentage).
    MemoryUsage,
    /// Quality score of responses (user-rated or heuristic).
    ResponseQuality,
}

impl MetricType {
    /// String key for use in maps and reports.
    fn key(&self) -> &'static str {
        match self {
            Self::LlmLatency => "llm_latency_ms",
            Self::ToolSuccessRate => "tool_success_rate",
            Self::ErrorRate => "error_rate",
            Self::MemoryUsage => "memory_usage",
            Self::ResponseQuality => "response_quality",
        }
    }
}

/// A single metric data point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    /// What kind of metric this is.
    pub metric_type: MetricType,
    /// The numeric value of the metric.
    pub value: f64,
    /// When this metric was recorded.
    pub timestamp: DateTime<Utc>,
}

/// A detected anomaly where a metric deviates from its running average.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anomaly {
    /// Which metric exhibited the anomaly.
    pub metric_type: MetricType,
    /// The expected (average) value.
    pub expected: f64,
    /// The actual observed value.
    pub actual: f64,
    /// How severe the anomaly is.
    pub severity: AnomalySeverity,
    /// Human-readable description of the anomaly.
    pub message: String,
}

/// Severity classification for anomalies.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AnomalySeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// Complete health report produced by a health check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    /// Overall system health status.
    pub status: HealthStatus,
    /// Current metric averages keyed by metric name.
    pub metrics: HashMap<String, f64>,
    /// Any detected anomalies.
    pub anomalies: Vec<Anomaly>,
    /// Actionable suggestions for improvement.
    pub suggestions: Vec<String>,
    /// When this report was generated.
    pub timestamp: DateTime<Utc>,
}

/// Configuration for the monitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorConfig {
    /// How often health checks should run, in seconds (informational; not enforced here).
    #[serde(default = "default_check_interval_secs")]
    pub check_interval_secs: u64,
    /// Error rate above which the system is considered degraded (0.0 - 1.0).
    #[serde(default = "default_error_rate_threshold")]
    pub error_rate_threshold: f32,
    /// LLM latency above which the system is considered degraded (milliseconds).
    #[serde(default = "default_latency_threshold_ms")]
    pub latency_threshold_ms: u64,
    /// Tool/LLM success rate below which the system is considered degraded (0.0 - 1.0).
    #[serde(default = "default_min_success_rate")]
    pub min_success_rate: f32,
}

fn default_check_interval_secs() -> u64 {
    60
}
fn default_error_rate_threshold() -> f32 {
    0.3
}
fn default_latency_threshold_ms() -> u64 {
    30_000
}
fn default_min_success_rate() -> f32 {
    0.7
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            check_interval_secs: default_check_interval_secs(),
            error_rate_threshold: default_error_rate_threshold(),
            latency_threshold_ms: default_latency_threshold_ms(),
            min_success_rate: default_min_success_rate(),
        }
    }
}

/// Maximum number of metric entries to retain per metric type.
const MAX_METRICS_PER_TYPE: usize = 1000;

/// Maximum number of recent error messages to keep.
const MAX_RECENT_ERRORS: usize = 100;

// ============================================================================
// Internal state
// ============================================================================

/// Interior mutable state for the monitor.
struct MonitorState {
    /// Ring buffers of metric values, keyed by metric type.
    metrics: HashMap<MetricType, VecDeque<Metric>>,
    /// Counters for LLM calls.
    llm_total: u64,
    llm_successes: u64,
    /// Counters for tool calls.
    tool_total: u64,
    tool_successes: u64,
    /// Total error counter.
    error_count: u64,
    /// Total event counter (errors + successes) for error rate calculation.
    event_count: u64,
    /// Recent error messages.
    recent_errors: VecDeque<String>,
}

impl MonitorState {
    fn new() -> Self {
        Self {
            metrics: HashMap::new(),
            llm_total: 0,
            llm_successes: 0,
            tool_total: 0,
            tool_successes: 0,
            error_count: 0,
            event_count: 0,
            recent_errors: VecDeque::new(),
        }
    }
}

// ============================================================================
// Monitor
// ============================================================================

/// Self-monitoring engine that tracks system health and detects anomalies.
pub struct Monitor {
    config: MonitorConfig,
    state: Mutex<MonitorState>,
}

impl Monitor {
    /// Create a new monitor with the given configuration.
    pub fn new(config: MonitorConfig) -> Self {
        Self {
            config,
            state: Mutex::new(MonitorState::new()),
        }
    }

    /// Record a generic metric data point.
    ///
    /// The metric is stored in a ring buffer that holds at most
    /// [`MAX_METRICS_PER_TYPE`] entries per metric type.
    pub fn record_metric(&self, metric: Metric) {
        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(e) => {
                warn!("Monitor lock poisoned: {}", e);
                return;
            }
        };

        let buffer = state
            .metrics
            .entry(metric.metric_type.clone())
            .or_insert_with(|| VecDeque::with_capacity(MAX_METRICS_PER_TYPE));

        if buffer.len() >= MAX_METRICS_PER_TYPE {
            buffer.pop_front();
        }
        buffer.push_back(metric);
    }

    /// Convenience: record an LLM call with its duration and outcome.
    ///
    /// Records a `MetricType::LlmLatency` metric and updates the LLM
    /// success/failure counters.
    pub fn record_llm_call(&self, duration_ms: u64, success: bool) {
        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(e) => {
                warn!("Monitor lock poisoned: {}", e);
                return;
            }
        };

        state.llm_total += 1;
        state.event_count += 1;
        if success {
            state.llm_successes += 1;
        } else {
            state.error_count += 1;
        }

        // Record latency metric.
        let buffer = state
            .metrics
            .entry(MetricType::LlmLatency)
            .or_insert_with(|| VecDeque::with_capacity(MAX_METRICS_PER_TYPE));
        if buffer.len() >= MAX_METRICS_PER_TYPE {
            buffer.pop_front();
        }
        buffer.push_back(Metric {
            metric_type: MetricType::LlmLatency,
            value: duration_ms as f64,
            timestamp: Utc::now(),
        });
    }

    /// Convenience: record a tool execution with its name, outcome, and duration.
    pub fn record_tool_call(&self, tool_name: &str, success: bool, duration_ms: u64) {
        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(e) => {
                warn!("Monitor lock poisoned: {}", e);
                return;
            }
        };

        state.tool_total += 1;
        state.event_count += 1;
        if success {
            state.tool_successes += 1;
        } else {
            state.error_count += 1;
            let msg = format!("Tool '{}' failed ({}ms)", tool_name, duration_ms);
            if state.recent_errors.len() >= MAX_RECENT_ERRORS {
                state.recent_errors.pop_front();
            }
            state.recent_errors.push_back(msg);
        }

        debug!(
            "Recorded tool call: {} success={} duration={}ms",
            tool_name, success, duration_ms
        );
    }

    /// Record a general error event.
    pub fn record_error(&self, error: &str) {
        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(e) => {
                warn!("Monitor lock poisoned: {}", e);
                return;
            }
        };

        state.error_count += 1;
        state.event_count += 1;

        if state.recent_errors.len() >= MAX_RECENT_ERRORS {
            state.recent_errors.pop_front();
        }
        state.recent_errors.push_back(error.to_string());
    }

    /// Perform a health check and return a comprehensive report.
    ///
    /// Analyzes recent metrics against the configured thresholds and detects
    /// anomalies by comparing recent values to their running averages.
    pub fn health_check(&self) -> HealthReport {
        let state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => {
                return HealthReport {
                    status: HealthStatus::Unhealthy("Monitor lock poisoned".to_string()),
                    metrics: HashMap::new(),
                    anomalies: vec![],
                    suggestions: vec![
                        "Restart the monitor to recover from lock poisoning".to_string(),
                    ],
                    timestamp: Utc::now(),
                };
            }
        };

        let mut metrics_summary: HashMap<String, f64> = HashMap::new();
        let mut anomalies: Vec<Anomaly> = Vec::new();
        let mut suggestions: Vec<String> = Vec::new();
        let mut worst_status = HealthStatus::Healthy;

        // --- Error rate ---
        let error_rate = if state.event_count > 0 {
            state.error_count as f64 / state.event_count as f64
        } else {
            0.0
        };
        metrics_summary.insert("error_rate".to_string(), error_rate);

        if error_rate > self.config.error_rate_threshold as f64 {
            let msg = format!(
                "Error rate {:.1}% exceeds threshold {:.1}%",
                error_rate * 100.0,
                self.config.error_rate_threshold * 100.0
            );
            if error_rate > 0.5 {
                worst_status = merge_status(worst_status, HealthStatus::Unhealthy(msg.clone()));
                anomalies.push(Anomaly {
                    metric_type: MetricType::ErrorRate,
                    expected: self.config.error_rate_threshold as f64,
                    actual: error_rate,
                    severity: AnomalySeverity::Critical,
                    message: msg,
                });
            } else {
                worst_status = merge_status(worst_status, HealthStatus::Degraded(msg.clone()));
                anomalies.push(Anomaly {
                    metric_type: MetricType::ErrorRate,
                    expected: self.config.error_rate_threshold as f64,
                    actual: error_rate,
                    severity: AnomalySeverity::High,
                    message: msg,
                });
            }
            suggestions.push("Review recent errors and address root causes".to_string());
        }

        // --- LLM success rate ---
        let llm_success_rate = if state.llm_total > 0 {
            state.llm_successes as f64 / state.llm_total as f64
        } else {
            1.0 // no calls yet = healthy
        };
        metrics_summary.insert("llm_success_rate".to_string(), llm_success_rate);

        if state.llm_total > 0 && llm_success_rate < self.config.min_success_rate as f64 {
            let msg = format!(
                "LLM success rate {:.1}% below minimum {:.1}%",
                llm_success_rate * 100.0,
                self.config.min_success_rate * 100.0
            );
            worst_status = merge_status(worst_status, HealthStatus::Degraded(msg.clone()));
            anomalies.push(Anomaly {
                metric_type: MetricType::ToolSuccessRate,
                expected: self.config.min_success_rate as f64,
                actual: llm_success_rate,
                severity: AnomalySeverity::High,
                message: msg,
            });
            suggestions.push("Check LLM provider connectivity and API key validity".to_string());
        }

        // --- Tool success rate ---
        let tool_success_rate = if state.tool_total > 0 {
            state.tool_successes as f64 / state.tool_total as f64
        } else {
            1.0
        };
        metrics_summary.insert("tool_success_rate".to_string(), tool_success_rate);

        if state.tool_total > 0 && tool_success_rate < self.config.min_success_rate as f64 {
            let msg = format!(
                "Tool success rate {:.1}% below minimum {:.1}%",
                tool_success_rate * 100.0,
                self.config.min_success_rate * 100.0
            );
            worst_status = merge_status(worst_status, HealthStatus::Degraded(msg.clone()));
            anomalies.push(Anomaly {
                metric_type: MetricType::ToolSuccessRate,
                expected: self.config.min_success_rate as f64,
                actual: tool_success_rate,
                severity: AnomalySeverity::Medium,
                message: msg,
            });
            suggestions.push("Review failing tools and their input parameters".to_string());
        }

        // --- LLM latency ---
        if let Some(latency_buf) = state.metrics.get(&MetricType::LlmLatency)
            && !latency_buf.is_empty()
        {
            let avg_latency: f64 =
                latency_buf.iter().map(|m| m.value).sum::<f64>() / latency_buf.len() as f64;
            metrics_summary.insert("avg_llm_latency_ms".to_string(), avg_latency);

            if avg_latency > self.config.latency_threshold_ms as f64 {
                let msg = format!(
                    "Average LLM latency {:.0}ms exceeds threshold {}ms",
                    avg_latency, self.config.latency_threshold_ms
                );
                worst_status = merge_status(worst_status, HealthStatus::Degraded(msg.clone()));
                anomalies.push(Anomaly {
                    metric_type: MetricType::LlmLatency,
                    expected: self.config.latency_threshold_ms as f64,
                    actual: avg_latency,
                    severity: AnomalySeverity::Medium,
                    message: msg,
                });
                suggestions.push(
                    "Consider using a faster model or checking network conditions".to_string(),
                );
            }

            // Detect latency spikes: if the most recent value is more than 3x the average.
            if let Some(latest) = latency_buf.back()
                && avg_latency > 0.0
                && latest.value > avg_latency * 3.0
            {
                let msg = format!(
                    "LLM latency spike: latest {:.0}ms is {:.1}x the average {:.0}ms",
                    latest.value,
                    latest.value / avg_latency,
                    avg_latency
                );
                anomalies.push(Anomaly {
                    metric_type: MetricType::LlmLatency,
                    expected: avg_latency,
                    actual: latest.value,
                    severity: AnomalySeverity::Low,
                    message: msg,
                });
            }
        }

        // --- Per-type metric averages ---
        for (metric_type, buffer) in &state.metrics {
            if !buffer.is_empty() {
                let avg = buffer.iter().map(|m| m.value).sum::<f64>() / buffer.len() as f64;
                // Insert under the metric type's key if not already present.
                metrics_summary
                    .entry(metric_type.key().to_string())
                    .or_insert(avg);
            }
        }

        // --- Totals ---
        metrics_summary.insert("llm_total_calls".to_string(), state.llm_total as f64);
        metrics_summary.insert("tool_total_calls".to_string(), state.tool_total as f64);
        metrics_summary.insert("total_errors".to_string(), state.error_count as f64);

        HealthReport {
            status: worst_status,
            metrics: metrics_summary,
            anomalies,
            suggestions,
            timestamp: Utc::now(),
        }
    }

    /// Return current metric averages as a map.
    pub fn get_metrics_summary(&self) -> HashMap<String, f64> {
        let state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return HashMap::new(),
        };

        let mut summary = HashMap::new();

        for (metric_type, buffer) in &state.metrics {
            if !buffer.is_empty() {
                let avg = buffer.iter().map(|m| m.value).sum::<f64>() / buffer.len() as f64;
                summary.insert(metric_type.key().to_string(), avg);
            }
        }

        if state.llm_total > 0 {
            summary.insert(
                "llm_success_rate".to_string(),
                state.llm_successes as f64 / state.llm_total as f64,
            );
        }
        if state.tool_total > 0 {
            summary.insert(
                "tool_success_rate".to_string(),
                state.tool_successes as f64 / state.tool_total as f64,
            );
        }
        if state.event_count > 0 {
            summary.insert(
                "error_rate".to_string(),
                state.error_count as f64 / state.event_count as f64,
            );
        }

        summary
    }

    /// Get recent error messages, most recent first.
    pub fn recent_errors(&self) -> Vec<String> {
        let state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        state.recent_errors.iter().rev().cloned().collect()
    }

    /// Clear all metrics and counters. Useful after a self-correction cycle.
    pub fn reset(&self) {
        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(e) => {
                warn!("Monitor lock poisoned during reset: {}", e);
                return;
            }
        };

        state.metrics.clear();
        state.llm_total = 0;
        state.llm_successes = 0;
        state.tool_total = 0;
        state.tool_successes = 0;
        state.error_count = 0;
        state.event_count = 0;
        state.recent_errors.clear();
        debug!("Monitor state reset");
    }

    /// Get the current configuration.
    pub fn config(&self) -> &MonitorConfig {
        &self.config
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Merge two health statuses, keeping the worse one.
///
/// Priority: Unhealthy > Degraded > Healthy.
fn merge_status(current: HealthStatus, new: HealthStatus) -> HealthStatus {
    match (&current, &new) {
        (HealthStatus::Unhealthy(_), _) => current,
        (_, HealthStatus::Unhealthy(_)) => new,
        (HealthStatus::Degraded(_), _) => current,
        (_, HealthStatus::Degraded(_)) => new,
        _ => current,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn default_monitor() -> Monitor {
        Monitor::new(MonitorConfig::default())
    }

    #[test]
    fn test_record_and_check_healthy() {
        let monitor = default_monitor();

        // Record a few successful LLM calls with normal latency.
        monitor.record_llm_call(500, true);
        monitor.record_llm_call(600, true);
        monitor.record_tool_call("read_file", true, 50);

        let report = monitor.health_check();
        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.anomalies.is_empty());
        assert!(report.suggestions.is_empty());
    }

    #[test]
    fn test_high_error_rate_detection() {
        let monitor = Monitor::new(MonitorConfig {
            error_rate_threshold: 0.3,
            ..Default::default()
        });

        // 4 errors out of 5 events = 80% error rate.
        monitor.record_error("err1");
        monitor.record_error("err2");
        monitor.record_error("err3");
        monitor.record_error("err4");
        monitor.record_llm_call(100, true); // 1 success

        let report = monitor.health_check();
        assert!(matches!(report.status, HealthStatus::Unhealthy(_)));

        let error_anomaly = report
            .anomalies
            .iter()
            .find(|a| a.metric_type == MetricType::ErrorRate)
            .expect("Should detect error rate anomaly");
        assert_eq!(error_anomaly.severity, AnomalySeverity::Critical);
        assert!(!report.suggestions.is_empty());
    }

    #[test]
    fn test_high_latency_detection() {
        let monitor = Monitor::new(MonitorConfig {
            latency_threshold_ms: 1000,
            ..Default::default()
        });

        // Record latency above the threshold.
        monitor.record_llm_call(5000, true);
        monitor.record_llm_call(6000, true);
        monitor.record_llm_call(4000, true);

        let report = monitor.health_check();
        assert!(matches!(report.status, HealthStatus::Degraded(_)));

        let latency_anomaly = report
            .anomalies
            .iter()
            .find(|a| a.metric_type == MetricType::LlmLatency)
            .expect("Should detect latency anomaly");
        assert_eq!(latency_anomaly.severity, AnomalySeverity::Medium);
        assert!(
            report
                .suggestions
                .iter()
                .any(|s| s.contains("faster model"))
        );
    }

    #[test]
    fn test_low_success_rate() {
        let monitor = Monitor::new(MonitorConfig {
            min_success_rate: 0.7,
            // Set error_rate_threshold high so only success rate triggers degradation.
            error_rate_threshold: 0.99,
            ..Default::default()
        });

        // 1 out of 5 LLM calls succeed = 20% success rate.
        monitor.record_llm_call(100, false);
        monitor.record_llm_call(100, false);
        monitor.record_llm_call(100, false);
        monitor.record_llm_call(100, false);
        monitor.record_llm_call(100, true);

        let report = monitor.health_check();
        assert!(
            matches!(report.status, HealthStatus::Degraded(_)),
            "Expected Degraded, got {:?}",
            report.status
        );
        assert!(
            report
                .suggestions
                .iter()
                .any(|s| s.contains("LLM provider"))
        );
    }

    #[test]
    fn test_anomaly_detection() {
        let monitor = Monitor::new(MonitorConfig {
            latency_threshold_ms: 100_000, // high threshold so avg doesn't trigger
            ..Default::default()
        });

        // Record several normal latencies then one spike.
        for _ in 0..10 {
            monitor.record_llm_call(100, true);
        }
        // Spike: 3100ms is > 3x the ~100ms average.
        monitor.record_llm_call(3100, true);

        let report = monitor.health_check();
        // The average won't exceed threshold so status should still be healthy,
        // but there should be a spike anomaly detected.
        let spike = report
            .anomalies
            .iter()
            .find(|a| a.message.contains("spike"));
        assert!(spike.is_some(), "Should detect latency spike anomaly");
        assert_eq!(spike.unwrap().severity, AnomalySeverity::Low);
    }

    #[test]
    fn test_record_llm_call() {
        let monitor = default_monitor();

        monitor.record_llm_call(250, true);
        monitor.record_llm_call(350, false);

        let summary = monitor.get_metrics_summary();
        assert!(summary.contains_key("llm_latency_ms"));
        // Average of 250 and 350 = 300.
        assert!((summary["llm_latency_ms"] - 300.0).abs() < 0.01);

        // LLM success rate: 1/2 = 0.5.
        assert!((summary["llm_success_rate"] - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_record_tool_call() {
        let monitor = default_monitor();

        monitor.record_tool_call("shell", true, 100);
        monitor.record_tool_call("shell", false, 200);
        monitor.record_tool_call("read_file", true, 50);

        let summary = monitor.get_metrics_summary();
        // 2 successes out of 3 tool calls.
        assert!((summary["tool_success_rate"] - 2.0 / 3.0).abs() < 0.01);

        // The failed tool call should appear in recent errors.
        let errors = monitor.recent_errors();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("shell"));
    }

    #[test]
    fn test_reset_clears_metrics() {
        let monitor = default_monitor();

        monitor.record_llm_call(100, true);
        monitor.record_llm_call(200, false);
        monitor.record_tool_call("shell", false, 50);
        monitor.record_error("something bad");

        // Verify we have data.
        assert!(!monitor.get_metrics_summary().is_empty());
        assert!(!monitor.recent_errors().is_empty());

        // Reset.
        monitor.reset();

        // Everything should be cleared.
        assert!(monitor.get_metrics_summary().is_empty());
        assert!(monitor.recent_errors().is_empty());

        let report = monitor.health_check();
        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.anomalies.is_empty());
    }

    #[test]
    fn test_default_config() {
        let config = MonitorConfig::default();
        assert_eq!(config.check_interval_secs, 60);
        assert!((config.error_rate_threshold - 0.3).abs() < 0.001);
        assert_eq!(config.latency_threshold_ms, 30_000);
        assert!((config.min_success_rate - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_health_report_suggestions() {
        let monitor = Monitor::new(MonitorConfig {
            error_rate_threshold: 0.1,
            min_success_rate: 0.9,
            latency_threshold_ms: 100,
            ..Default::default()
        });

        // Trigger all three problem conditions.
        // High error rate.
        monitor.record_error("e1");
        monitor.record_error("e2");
        // Low LLM success rate and high latency.
        monitor.record_llm_call(500, false);
        monitor.record_llm_call(600, false);
        monitor.record_llm_call(400, true);
        // Low tool success rate.
        monitor.record_tool_call("shell", false, 10);
        monitor.record_tool_call("shell", false, 10);
        monitor.record_tool_call("shell", true, 10);

        let report = monitor.health_check();

        // Should have suggestions for errors, LLM, latency, and tools.
        assert!(
            report.suggestions.len() >= 3,
            "Expected at least 3 suggestions, got {}: {:?}",
            report.suggestions.len(),
            report.suggestions
        );
        assert!(report.suggestions.iter().any(|s| s.contains("root causes")));
        assert!(
            report
                .suggestions
                .iter()
                .any(|s| s.contains("LLM provider"))
        );
        assert!(
            report
                .suggestions
                .iter()
                .any(|s| s.contains("faster model") || s.contains("failing tools"))
        );

        // Status should be Unhealthy due to high error rate.
        assert!(matches!(report.status, HealthStatus::Unhealthy(_)));
    }

    #[test]
    fn test_merge_status() {
        // Healthy + Degraded = Degraded
        let result = merge_status(
            HealthStatus::Healthy,
            HealthStatus::Degraded("test".to_string()),
        );
        assert!(matches!(result, HealthStatus::Degraded(_)));

        // Degraded + Unhealthy = Unhealthy
        let result = merge_status(
            HealthStatus::Degraded("a".to_string()),
            HealthStatus::Unhealthy("b".to_string()),
        );
        assert!(matches!(result, HealthStatus::Unhealthy(_)));

        // Unhealthy + Degraded = Unhealthy (already worst)
        let result = merge_status(
            HealthStatus::Unhealthy("a".to_string()),
            HealthStatus::Degraded("b".to_string()),
        );
        assert!(matches!(result, HealthStatus::Unhealthy(_)));

        // Healthy + Healthy = Healthy
        let result = merge_status(HealthStatus::Healthy, HealthStatus::Healthy);
        assert_eq!(result, HealthStatus::Healthy);
    }

    #[test]
    fn test_recent_errors_ordering() {
        let monitor = default_monitor();

        monitor.record_error("first");
        monitor.record_error("second");
        monitor.record_error("third");

        let errors = monitor.recent_errors();
        assert_eq!(errors.len(), 3);
        // Most recent first.
        assert_eq!(errors[0], "third");
        assert_eq!(errors[1], "second");
        assert_eq!(errors[2], "first");
    }

    #[test]
    fn test_empty_monitor_healthy() {
        let monitor = default_monitor();
        let report = monitor.health_check();

        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.anomalies.is_empty());
        assert!(report.suggestions.is_empty());
    }

    #[test]
    fn test_monitor_multiple_operations() {
        let monitor = default_monitor();

        // Record many mixed operations
        for i in 0..50 {
            monitor.record_llm_call(100 + i * 10, true);
            monitor.record_tool_call("read_file", true, 20 + i);
        }

        let report = monitor.health_check();
        assert_eq!(report.status, HealthStatus::Healthy);

        let summary = monitor.get_metrics_summary();
        assert!(summary.contains_key("llm_latency_ms"));
        assert!(summary.contains_key("llm_success_rate"));
        assert!(summary.contains_key("tool_success_rate"));
        // All successful, so rates should be 1.0
        assert!((summary["llm_success_rate"] - 1.0).abs() < 0.01);
        assert!((summary["tool_success_rate"] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_monitor_all_failures() {
        let monitor = Monitor::new(MonitorConfig {
            error_rate_threshold: 0.3,
            min_success_rate: 0.7,
            ..Default::default()
        });

        // All LLM calls fail
        for _ in 0..10 {
            monitor.record_llm_call(100, false);
        }

        let report = monitor.health_check();
        // Should be Unhealthy due to 100% error rate
        assert!(
            matches!(report.status, HealthStatus::Unhealthy(_)),
            "Expected Unhealthy, got {:?}",
            report.status
        );

        let summary = monitor.get_metrics_summary();
        assert!((summary["llm_success_rate"] - 0.0).abs() < 0.01);
        assert!((summary["error_rate"] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_monitor_latency_spikes() {
        let monitor = Monitor::new(MonitorConfig {
            latency_threshold_ms: 100_000, // high threshold so avg won't trigger
            ..Default::default()
        });

        // Record several normal latencies
        for _ in 0..20 {
            monitor.record_llm_call(100, true);
        }
        // Add a massive spike
        monitor.record_llm_call(50_000, true);

        let report = monitor.health_check();
        // Should detect the spike anomaly
        let spike = report
            .anomalies
            .iter()
            .find(|a| a.message.contains("spike"));
        assert!(spike.is_some(), "Should detect latency spike");
        assert_eq!(spike.unwrap().severity, AnomalySeverity::Low);
    }

    #[test]
    fn test_monitor_reset_and_rerecord() {
        let monitor = default_monitor();

        // Record some data
        monitor.record_llm_call(500, true);
        monitor.record_llm_call(600, false);
        monitor.record_error("test error");
        assert!(!monitor.get_metrics_summary().is_empty());
        assert!(!monitor.recent_errors().is_empty());

        // Reset
        monitor.reset();
        assert!(monitor.get_metrics_summary().is_empty());
        assert!(monitor.recent_errors().is_empty());

        // Record new data after reset
        monitor.record_llm_call(200, true);
        monitor.record_tool_call("shell", true, 50);

        let summary = monitor.get_metrics_summary();
        assert!(summary.contains_key("llm_latency_ms"));
        assert!((summary["llm_success_rate"] - 1.0).abs() < 0.01);
        assert!((summary["tool_success_rate"] - 1.0).abs() < 0.01);
        // No errors after reset
        assert!(monitor.recent_errors().is_empty());
    }

    #[test]
    fn test_monitor_config_custom() {
        let config = MonitorConfig {
            check_interval_secs: 120,
            error_rate_threshold: 0.5,
            latency_threshold_ms: 10_000,
            min_success_rate: 0.9,
        };
        let monitor = Monitor::new(config);

        let cfg = monitor.config();
        assert_eq!(cfg.check_interval_secs, 120);
        assert!((cfg.error_rate_threshold - 0.5).abs() < 0.001);
        assert_eq!(cfg.latency_threshold_ms, 10_000);
        assert!((cfg.min_success_rate - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_health_report_with_warnings() {
        let monitor = Monitor::new(MonitorConfig {
            error_rate_threshold: 0.2,
            min_success_rate: 0.8,
            latency_threshold_ms: 500,
            ..Default::default()
        });

        // Create a scenario with degraded (not unhealthy) error rate
        // 2 errors out of 8 events = 25% error rate > 0.2 threshold but < 50%
        monitor.record_llm_call(100, true);
        monitor.record_llm_call(100, true);
        monitor.record_llm_call(100, true);
        monitor.record_llm_call(100, true);
        monitor.record_llm_call(100, true);
        monitor.record_llm_call(100, true);
        monitor.record_error("warning error 1");
        monitor.record_error("warning error 2");

        let report = monitor.health_check();
        // Should be Degraded (not Unhealthy since error rate < 50%)
        assert!(
            matches!(report.status, HealthStatus::Degraded(_)),
            "Expected Degraded, got {:?}",
            report.status
        );
        assert!(!report.suggestions.is_empty());
        // Should have at least one anomaly about error rate
        assert!(
            report
                .anomalies
                .iter()
                .any(|a| a.metric_type == MetricType::ErrorRate)
        );
    }
}
