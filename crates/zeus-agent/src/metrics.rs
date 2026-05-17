//! Lightweight metrics collection for Zeus agent operations.
//!
//! Uses atomic counters with no external dependencies required.
//! These metrics can be exported to OTEL, Prometheus, or any metrics backend
//! by reading the atomic values and forwarding them to the appropriate exporter.
//!
//! # OTEL Integration
//!
//! To connect to OpenTelemetry, you can periodically read these metrics and
//! push them via the OTEL SDK. Because the counters are plain atomics, there
//! is zero overhead when no exporter is attached:
//!
//! ```ignore
//! // Example: exporting to OTEL (requires opentelemetry crate)
//! let meter = global::meter("zeus");
//! let counter = meter.u64_counter("zeus.requests").init();
//! counter.add(metrics.request_count.load(Ordering::Relaxed), &[]);
//! ```

use std::sync::atomic::{AtomicU64, Ordering};

/// Lightweight metrics collection for Zeus agent operations.
///
/// Uses atomic counters for thread-safe, lock-free metric recording.
/// Can be exported to OTEL, Prometheus, or any metrics backend.
pub struct AgentMetrics {
    /// Total agent requests processed.
    pub request_count: AtomicU64,
    /// Total tool executions.
    pub tool_execution_count: AtomicU64,
    /// Total LLM tokens used (approximate).
    pub llm_tokens_used: AtomicU64,
    /// Total inbound channel messages.
    pub channel_messages_inbound: AtomicU64,
    /// Total outbound channel messages.
    pub channel_messages_outbound: AtomicU64,
    /// Total errors encountered.
    pub error_count: AtomicU64,
    /// Sum of request durations in milliseconds (for computing average).
    pub request_duration_ms_sum: AtomicU64,
}

impl AgentMetrics {
    /// Create a new metrics instance with all counters at zero.
    pub fn new() -> Self {
        Self {
            request_count: AtomicU64::new(0),
            tool_execution_count: AtomicU64::new(0),
            llm_tokens_used: AtomicU64::new(0),
            channel_messages_inbound: AtomicU64::new(0),
            channel_messages_outbound: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            request_duration_ms_sum: AtomicU64::new(0),
        }
    }

    /// Record a completed request with its duration in milliseconds.
    pub fn record_request(&self, duration_ms: u64) {
        self.request_count.fetch_add(1, Ordering::Relaxed);
        self.request_duration_ms_sum
            .fetch_add(duration_ms, Ordering::Relaxed);
    }

    /// Record a tool execution.
    pub fn record_tool_execution(&self) {
        self.tool_execution_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record LLM token usage.
    pub fn record_tokens(&self, tokens: u64) {
        self.llm_tokens_used.fetch_add(tokens, Ordering::Relaxed);
    }

    /// Record an inbound channel message.
    pub fn record_inbound_message(&self) {
        self.channel_messages_inbound
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record an outbound channel message.
    pub fn record_outbound_message(&self) {
        self.channel_messages_outbound
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record an error.
    pub fn record_error(&self) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Get average request duration in milliseconds.
    /// Returns 0.0 if no requests have been recorded.
    pub fn avg_request_duration_ms(&self) -> f64 {
        let count = self.request_count.load(Ordering::Relaxed);
        if count == 0 {
            0.0
        } else {
            self.request_duration_ms_sum.load(Ordering::Relaxed) as f64 / count as f64
        }
    }

    /// Export all metrics as a JSON value for API/status endpoints.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "request_count": self.request_count.load(Ordering::Relaxed),
            "tool_execution_count": self.tool_execution_count.load(Ordering::Relaxed),
            "llm_tokens_used": self.llm_tokens_used.load(Ordering::Relaxed),
            "channel_messages_inbound": self.channel_messages_inbound.load(Ordering::Relaxed),
            "channel_messages_outbound": self.channel_messages_outbound.load(Ordering::Relaxed),
            "error_count": self.error_count.load(Ordering::Relaxed),
            "avg_request_duration_ms": self.avg_request_duration_ms(),
        })
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        self.request_count.store(0, Ordering::Relaxed);
        self.tool_execution_count.store(0, Ordering::Relaxed);
        self.llm_tokens_used.store(0, Ordering::Relaxed);
        self.channel_messages_inbound.store(0, Ordering::Relaxed);
        self.channel_messages_outbound.store(0, Ordering::Relaxed);
        self.error_count.store(0, Ordering::Relaxed);
        self.request_duration_ms_sum.store(0, Ordering::Relaxed);
    }
}

impl Default for AgentMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_metrics_default_zeros() {
        let metrics = AgentMetrics::new();
        assert_eq!(metrics.request_count.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.tool_execution_count.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.llm_tokens_used.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.channel_messages_inbound.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.channel_messages_outbound.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.error_count.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.request_duration_ms_sum.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.avg_request_duration_ms(), 0.0);
    }

    #[test]
    fn test_record_request_increments() {
        let metrics = AgentMetrics::new();
        metrics.record_request(100);
        assert_eq!(metrics.request_count.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.request_duration_ms_sum.load(Ordering::Relaxed), 100);

        metrics.record_request(200);
        assert_eq!(metrics.request_count.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.request_duration_ms_sum.load(Ordering::Relaxed), 300);
    }

    #[test]
    fn test_record_tool_execution() {
        let metrics = AgentMetrics::new();
        metrics.record_tool_execution();
        metrics.record_tool_execution();
        metrics.record_tool_execution();
        assert_eq!(metrics.tool_execution_count.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn test_avg_duration_calculation() {
        let metrics = AgentMetrics::new();

        // No requests yet
        assert_eq!(metrics.avg_request_duration_ms(), 0.0);

        metrics.record_request(100);
        assert_eq!(metrics.avg_request_duration_ms(), 100.0);

        metrics.record_request(200);
        assert_eq!(metrics.avg_request_duration_ms(), 150.0);

        metrics.record_request(300);
        assert_eq!(metrics.avg_request_duration_ms(), 200.0);
    }

    #[test]
    fn test_reset_clears_all() {
        let metrics = AgentMetrics::new();
        metrics.record_request(100);
        metrics.record_tool_execution();
        metrics.record_tokens(500);
        metrics.record_inbound_message();
        metrics.record_outbound_message();
        metrics.record_error();

        // Verify non-zero
        assert_ne!(metrics.request_count.load(Ordering::Relaxed), 0);
        assert_ne!(metrics.tool_execution_count.load(Ordering::Relaxed), 0);
        assert_ne!(metrics.llm_tokens_used.load(Ordering::Relaxed), 0);

        metrics.reset();

        assert_eq!(metrics.request_count.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.tool_execution_count.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.llm_tokens_used.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.channel_messages_inbound.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.channel_messages_outbound.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.error_count.load(Ordering::Relaxed), 0);
        assert_eq!(metrics.request_duration_ms_sum.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_to_json_format() {
        let metrics = AgentMetrics::new();
        metrics.record_request(150);
        metrics.record_tool_execution();
        metrics.record_tokens(1000);
        metrics.record_error();

        let json = metrics.to_json();
        assert_eq!(json["request_count"], 1);
        assert_eq!(json["tool_execution_count"], 1);
        assert_eq!(json["llm_tokens_used"], 1000);
        assert_eq!(json["channel_messages_inbound"], 0);
        assert_eq!(json["channel_messages_outbound"], 0);
        assert_eq!(json["error_count"], 1);
        assert_eq!(json["avg_request_duration_ms"], 150.0);
    }

    #[tokio::test]
    async fn test_concurrent_metric_updates() {
        let metrics = Arc::new(AgentMetrics::new());
        let mut handles = vec![];

        // Spawn 10 tasks, each incrementing request_count 100 times
        for _ in 0..10 {
            let m = Arc::clone(&metrics);
            handles.push(tokio::spawn(async move {
                for _ in 0..100 {
                    m.record_request(1);
                }
            }));
        }

        // Spawn 10 tasks, each incrementing tool_execution_count 50 times
        for _ in 0..10 {
            let m = Arc::clone(&metrics);
            handles.push(tokio::spawn(async move {
                for _ in 0..50 {
                    m.record_tool_execution();
                }
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }

        assert_eq!(metrics.request_count.load(Ordering::Relaxed), 1000);
        assert_eq!(metrics.tool_execution_count.load(Ordering::Relaxed), 500);
        assert_eq!(
            metrics.request_duration_ms_sum.load(Ordering::Relaxed),
            1000
        );
    }

    #[test]
    fn test_record_tokens() {
        let metrics = AgentMetrics::new();
        metrics.record_tokens(100);
        metrics.record_tokens(250);
        assert_eq!(metrics.llm_tokens_used.load(Ordering::Relaxed), 350);
    }

    #[test]
    fn test_record_channel_messages() {
        let metrics = AgentMetrics::new();
        metrics.record_inbound_message();
        metrics.record_inbound_message();
        metrics.record_outbound_message();
        assert_eq!(metrics.channel_messages_inbound.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.channel_messages_outbound.load(Ordering::Relaxed), 1);
    }

    // ================================================================
    // New tests
    // ================================================================

    #[test]
    fn test_metrics_record_many_requests() {
        let metrics = AgentMetrics::new();
        for i in 0..100 {
            metrics.record_request(i as u64);
        }
        assert_eq!(metrics.request_count.load(Ordering::Relaxed), 100);
        // Sum of 0..99 = 4950
        assert_eq!(
            metrics.request_duration_ms_sum.load(Ordering::Relaxed),
            4950
        );
        // Average of 0..99 = 49.5
        assert!((metrics.avg_request_duration_ms() - 49.5).abs() < 0.01);
    }

    #[test]
    fn test_metrics_record_tokens_accumulation() {
        let metrics = AgentMetrics::new();
        metrics.record_tokens(100);
        metrics.record_tokens(200);
        metrics.record_tokens(300);
        metrics.record_tokens(400);
        metrics.record_tokens(500);
        assert_eq!(metrics.llm_tokens_used.load(Ordering::Relaxed), 1500);
    }

    #[test]
    fn test_metrics_tool_count() {
        let metrics = AgentMetrics::new();
        // Simulate multiple different tool calls
        for _ in 0..5 {
            metrics.record_tool_execution(); // read_file
        }
        for _ in 0..3 {
            metrics.record_tool_execution(); // shell
        }
        for _ in 0..2 {
            metrics.record_tool_execution(); // write_file
        }
        // Total tool executions regardless of type
        assert_eq!(metrics.tool_execution_count.load(Ordering::Relaxed), 10);
    }

    #[test]
    fn test_metrics_avg_duration_single() {
        let metrics = AgentMetrics::new();
        metrics.record_request(250);
        // Single request: average equals the single value
        assert_eq!(metrics.avg_request_duration_ms(), 250.0);
    }

    #[test]
    fn test_metrics_to_json_all_fields() {
        let metrics = AgentMetrics::new();
        metrics.record_request(100);
        metrics.record_request(200);
        metrics.record_tool_execution();
        metrics.record_tool_execution();
        metrics.record_tool_execution();
        metrics.record_tokens(5000);
        metrics.record_inbound_message();
        metrics.record_inbound_message();
        metrics.record_inbound_message();
        metrics.record_outbound_message();
        metrics.record_outbound_message();
        metrics.record_error();

        let json = metrics.to_json();

        // Verify all expected fields are present and correct
        assert_eq!(json["request_count"], 2);
        assert_eq!(json["tool_execution_count"], 3);
        assert_eq!(json["llm_tokens_used"], 5000);
        assert_eq!(json["channel_messages_inbound"], 3);
        assert_eq!(json["channel_messages_outbound"], 2);
        assert_eq!(json["error_count"], 1);
        assert_eq!(json["avg_request_duration_ms"], 150.0);

        // Verify these are all the top-level keys
        let obj = json.as_object().unwrap();
        assert_eq!(obj.len(), 7);
        assert!(obj.contains_key("request_count"));
        assert!(obj.contains_key("tool_execution_count"));
        assert!(obj.contains_key("llm_tokens_used"));
        assert!(obj.contains_key("channel_messages_inbound"));
        assert!(obj.contains_key("channel_messages_outbound"));
        assert!(obj.contains_key("error_count"));
        assert!(obj.contains_key("avg_request_duration_ms"));
    }

    #[test]
    fn test_metrics_channel_messages_multiple() {
        let metrics = AgentMetrics::new();
        // Simulate a flurry of inbound and outbound messages
        for _ in 0..50 {
            metrics.record_inbound_message();
        }
        for _ in 0..30 {
            metrics.record_outbound_message();
        }
        assert_eq!(metrics.channel_messages_inbound.load(Ordering::Relaxed), 50);
        assert_eq!(
            metrics.channel_messages_outbound.load(Ordering::Relaxed),
            30
        );

        // Verify in JSON too
        let json = metrics.to_json();
        assert_eq!(json["channel_messages_inbound"], 50);
        assert_eq!(json["channel_messages_outbound"], 30);
    }
}
