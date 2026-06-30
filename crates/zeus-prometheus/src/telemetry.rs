//! OpenTelemetry Observability Module
//!
//! Provides OTEL metrics and tracing export for Zeus.
//! Supports OTLP (gRPC) export to collectors like Jaeger, Tempo, or Prometheus.

use opentelemetry::{KeyValue, global};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::TracerProvider;
use opentelemetry_sdk::{Resource, runtime};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::info;

/// Telemetry configuration
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Service name for OTEL resource
    pub service_name: String,
    /// Service version
    pub service_version: String,
    /// OTLP endpoint — overridden by `ZEUS_OTLP_URL` env var (default: `http://localhost:4317`)
    pub otlp_endpoint: String,
    /// Enable tracing export
    pub enable_tracing: bool,
    /// Enable metrics export
    pub enable_metrics: bool,
    /// Metrics export interval in seconds
    pub metrics_interval_secs: u64,
    /// Batch export timeout in seconds
    pub export_timeout_secs: u64,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            service_name: "zeus".to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            otlp_endpoint: std::env::var("ZEUS_OTLP_URL")
                .unwrap_or_else(|_| "http://localhost:4317".to_string()),
            enable_tracing: true,
            enable_metrics: true,
            metrics_interval_secs: 60,
            export_timeout_secs: 10,
        }
    }
}

/// OpenTelemetry telemetry manager
pub struct Telemetry {
    config: TelemetryConfig,
    tracer_provider: Option<TracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
    /// Custom metrics storage
    metrics: Arc<RwLock<TelemetryMetrics>>,
}

/// Custom Zeus metrics
#[derive(Debug, Default, Clone)]
pub struct TelemetryMetrics {
    /// Total LLM calls
    pub llm_calls_total: u64,
    /// Total LLM tokens used
    pub llm_tokens_total: u64,
    /// Total tool executions
    pub tool_executions_total: u64,
    /// Tool execution errors
    pub tool_errors_total: u64,
    /// Active sessions
    pub active_sessions: u64,
    /// Total messages processed
    pub messages_total: u64,
    /// Average response time (ms)
    pub avg_response_time_ms: f64,
    /// Memory usage bytes
    pub memory_usage_bytes: u64,
}

impl Telemetry {
    /// Create a new telemetry instance
    pub fn new(config: TelemetryConfig) -> Self {
        Self {
            config,
            tracer_provider: None,
            meter_provider: None,
            metrics: Arc::new(RwLock::new(TelemetryMetrics::default())),
        }
    }

    /// Initialize OpenTelemetry with OTLP export
    pub async fn init(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let resource = Resource::new(vec![
            KeyValue::new("service.name", self.config.service_name.clone()),
            KeyValue::new("service.version", self.config.service_version.clone()),
            KeyValue::new("service.namespace", "zeus"),
        ]);

        // Initialize tracing
        if self.config.enable_tracing {
            let tracer_provider = self.init_tracer(&resource).await?;
            self.tracer_provider = Some(tracer_provider.clone());

            // Set global tracer provider
            global::set_tracer_provider(tracer_provider);
            info!(
                endpoint = %self.config.otlp_endpoint,
                "OpenTelemetry tracing initialized"
            );
        }

        // Initialize metrics
        if self.config.enable_metrics {
            let meter_provider = self.init_metrics(&resource).await?;
            self.meter_provider = Some(meter_provider.clone());

            // Set global meter provider
            global::set_meter_provider(meter_provider);
            info!(
                endpoint = %self.config.otlp_endpoint,
                interval_secs = self.config.metrics_interval_secs,
                "OpenTelemetry metrics initialized"
            );
        }

        Ok(())
    }

    /// Initialize tracer provider with OTLP exporter
    async fn init_tracer(
        &self,
        resource: &Resource,
    ) -> Result<TracerProvider, Box<dyn std::error::Error + Send + Sync>> {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&self.config.otlp_endpoint)
            .with_timeout(Duration::from_secs(self.config.export_timeout_secs))
            .build()?;

        let provider = TracerProvider::builder()
            .with_batch_exporter(exporter, runtime::Tokio)
            .with_resource(resource.clone())
            .build();

        Ok(provider)
    }

    /// Initialize meter provider with OTLP exporter
    async fn init_metrics(
        &self,
        resource: &Resource,
    ) -> Result<SdkMeterProvider, Box<dyn std::error::Error + Send + Sync>> {
        let exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_endpoint(&self.config.otlp_endpoint)
            .with_timeout(Duration::from_secs(self.config.export_timeout_secs))
            .build()?;

        let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter, runtime::Tokio)
            .with_interval(Duration::from_secs(self.config.metrics_interval_secs))
            .build();

        let provider = SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(resource.clone())
            .build();

        Ok(provider)
    }

    /// Get a tracer for instrumenting code
    pub fn tracer(&self, name: &'static str) -> opentelemetry::global::BoxedTracer {
        global::tracer(name)
    }

    /// Get a meter for recording metrics
    pub fn meter(&self, name: &'static str) -> opentelemetry::metrics::Meter {
        global::meter(name)
    }

    /// Record an LLM call
    pub async fn record_llm_call(&self, tokens: u64, duration_ms: u64) {
        let mut metrics = self.metrics.write().await;
        metrics.llm_calls_total += 1;
        metrics.llm_tokens_total += tokens;

        // Update average response time
        let total_calls = metrics.llm_calls_total as f64;
        metrics.avg_response_time_ms = ((metrics.avg_response_time_ms * (total_calls - 1.0))
            + duration_ms as f64)
            / total_calls;

        // Record to OTEL meter
        if self.config.enable_metrics {
            let meter = self.meter("zeus.llm");
            let counter = meter.u64_counter("zeus.llm.calls.total").build();
            counter.add(1, &[]);

            let token_counter = meter.u64_counter("zeus.llm.tokens.total").build();
            token_counter.add(tokens, &[]);

            let histogram = meter.f64_histogram("zeus.llm.response.duration").build();
            histogram.record(duration_ms as f64, &[]);
        }
    }

    /// Record a tool execution
    pub async fn record_tool_execution(&self, tool_name: &str, success: bool, duration_ms: u64) {
        let mut metrics = self.metrics.write().await;
        metrics.tool_executions_total += 1;
        if !success {
            metrics.tool_errors_total += 1;
        }

        if self.config.enable_metrics {
            let meter = self.meter("zeus.tools");
            let counter = meter.u64_counter("zeus.tools.executions.total").build();
            counter.add(
                1,
                &[
                    KeyValue::new("tool", tool_name.to_string()),
                    KeyValue::new("success", success.to_string()),
                ],
            );

            let histogram = meter.f64_histogram("zeus.tools.duration").build();
            histogram.record(
                duration_ms as f64,
                &[KeyValue::new("tool", tool_name.to_string())],
            );
        }
    }

    /// Record a message processed
    pub async fn record_message(&self, channel: &str) {
        let mut metrics = self.metrics.write().await;
        metrics.messages_total += 1;

        if self.config.enable_metrics {
            let meter = self.meter("zeus.messages");
            let counter = meter.u64_counter("zeus.messages.total").build();
            counter.add(1, &[KeyValue::new("channel", channel.to_string())]);
        }
    }

    /// Update active session count
    pub async fn set_active_sessions(&self, count: u64) {
        let mut metrics = self.metrics.write().await;
        metrics.active_sessions = count;

        if self.config.enable_metrics {
            let meter = self.meter("zeus.sessions");
            let gauge = meter.u64_gauge("zeus.sessions.active").build();
            gauge.record(count, &[]);
        }
    }

    /// Get current metrics snapshot
    pub async fn get_metrics(&self) -> TelemetryMetrics {
        self.metrics.read().await.clone()
    }

    /// Shutdown telemetry exporters gracefully
    pub fn shutdown(&mut self) {
        if let Some(ref provider) = self.tracer_provider
            && let Err(e) = provider.shutdown()
        {
            tracing::error!(error = %e, "Failed to shutdown tracer provider");
        }

        if let Some(ref provider) = self.meter_provider
            && let Err(e) = provider.shutdown()
        {
            tracing::error!(error = %e, "Failed to shutdown meter provider");
        }

        info!("OpenTelemetry shutdown complete");
    }
}

impl Drop for Telemetry {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telemetry_config_default() {
        let config = TelemetryConfig::default();
        assert_eq!(config.service_name, "zeus");
        assert!(config.enable_tracing);
        assert!(config.enable_metrics);
        assert_eq!(config.metrics_interval_secs, 60);
    }

    #[tokio::test]
    async fn test_telemetry_metrics_recording() {
        let config = TelemetryConfig {
            enable_metrics: false, // Don't try to connect
            enable_tracing: false,
            ..Default::default()
        };
        let telemetry = Telemetry::new(config);

        // Record some metrics
        telemetry.record_llm_call(100, 500).await;
        telemetry.record_llm_call(150, 600).await;
        telemetry.record_tool_execution("shell", true, 100).await;
        telemetry
            .record_tool_execution("read_file", false, 50)
            .await;
        telemetry.record_message("telegram").await;
        telemetry.set_active_sessions(5).await;

        let metrics = telemetry.get_metrics().await;
        assert_eq!(metrics.llm_calls_total, 2);
        assert_eq!(metrics.llm_tokens_total, 250);
        assert_eq!(metrics.tool_executions_total, 2);
        assert_eq!(metrics.tool_errors_total, 1);
        assert_eq!(metrics.messages_total, 1);
        assert_eq!(metrics.active_sessions, 5);
    }

    #[test]
    fn test_telemetry_new() {
        let config = TelemetryConfig::default();
        let telemetry = Telemetry::new(config);
        assert!(telemetry.tracer_provider.is_none());
        assert!(telemetry.meter_provider.is_none());
    }
}
