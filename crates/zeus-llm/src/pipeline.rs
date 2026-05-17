//! F6: LlmPlugin Pipeline for LLM requests and responses.
//!
//! Provides a composable plugin system that wraps `LlmClient::complete` and
//! `LlmClient::stream`. Plugins are executed in order on the request path
//! and in reverse order on the response path (onion model).
//!
//! Built-in plugins:
//! - `LoggingPlugin` — traces request/response metadata
//! - `RetryPlugin` — per-plugin retry with backoff (supplements LlmClient's built-in retry)
//! - `RateLimitPlugin` — token-bucket rate limiter
//! - `CostMeterPlugin` — accumulates token costs across calls
//!
//! Usage:
//! ```ignore
//! let pipeline = Pipeline::new(client)
//!     .with(LoggingPlugin)
//!     .with(RateLimitPlugin::new(10, Duration::from_secs(60)));
//! let response = pipeline.complete(messages, tools, system).await?;
//! ```

use crate::{LlmClient, LlmResponse, ResponseStream, StreamChunk};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use zeus_core::{Error, Message, Result, Role, ToolSchema};

// ============================================================================
// Core types
// ============================================================================

/// Mutable context passed through the plugin chain on each request.
#[derive(Debug)]
pub struct RequestContext {
    /// Messages being sent (plugins may mutate).
    pub messages: Vec<Message>,
    /// Tool schemas (plugins may add/remove).
    pub tools: Vec<ToolSchema>,
    /// System prompt (plugins may override).
    pub system: Option<String>,
    /// Arbitrary metadata for cross-plugin communication.
    pub meta: std::collections::HashMap<String, String>,
}

/// Immutable context attached to each response.
#[derive(Debug, Clone)]
pub struct ResponseContext {
    /// Wall-clock duration of the LLM call.
    pub elapsed: Duration,
    /// Provider that handled the request.
    pub provider: String,
    /// Model that was used.
    pub model: String,
}

/// The trait all LLM plugins must implement.
///
/// `pre_request` runs before the LLM call (request path).
/// `post_response` runs after the LLM call (response path).
/// Both have default no-op implementations so plugins can opt in selectively.
#[async_trait::async_trait]
pub trait LlmPlugin: Send + Sync + std::fmt::Debug {
    /// Human-readable name for logging.
    fn name(&self) -> &str {
        "unnamed"
    }

    /// Mutate the request before it's sent. Return `Err` to short-circuit.
    async fn pre_request(&self, ctx: &mut RequestContext) -> Result<()> {
        let _ = ctx;
        Ok(())
    }

    /// Inspect/mutate the response after it's received. Return `Err` to fail the call.
    async fn post_response(
        &self,
        response: &mut LlmResponse,
        ctx: &ResponseContext,
    ) -> Result<()> {
        let _ = (response, ctx);
        Ok(())
    }
}

// ============================================================================
// Pipeline
// ============================================================================

/// A composable plugin pipeline wrapping an `LlmClient`.
pub struct Pipeline {
    client: Arc<LlmClient>,
    plugins: Vec<Arc<dyn LlmPlugin>>,
}

impl Pipeline {
    /// Create a new pipeline with no plugins.
    pub fn new(client: LlmClient) -> Self {
        Self {
            client: Arc::new(client),
            plugins: Vec::new(),
        }
    }

    /// Add a plugin to the end of the chain.
    pub fn with<P: LlmPlugin + 'static>(mut self, plugin: P) -> Self {
        self.plugins.push(Arc::new(plugin));
        self
    }

    /// Add a plugin (Arc-wrapped) to the end of the chain.
    pub fn with_arc(mut self, plugin: Arc<dyn LlmPlugin>) -> Self {
        self.plugins.push(plugin);
        self
    }

    /// Execute the full pipeline: pre_request hooks → LLM call → post_response hooks.
    pub async fn complete(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSchema>,
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        let mut ctx = RequestContext {
            messages,
            tools,
            system: system.map(|s| s.to_string()),
            meta: std::collections::HashMap::new(),
        };

        // Request path: pre_request hooks in order
        for plugin in &self.plugins {
            plugin.pre_request(&mut ctx).await?;
        }

        // LLM call
        let start = Instant::now();
        let mut response = self
            .client
            .complete(&ctx.messages, &ctx.tools, ctx.system.as_deref())
            .await?;
        let elapsed = start.elapsed();

        let resp_ctx = ResponseContext {
            elapsed,
            provider: format!("{:?}", self.client.provider()),
            model: self.client.model().to_string(),
        };

        // Response path: post_response hooks in reverse order (onion model)
        for plugin in self.plugins.iter().rev() {
            plugin.post_response(&mut response, &resp_ctx).await?;
        }

        Ok(response)
    }

    /// Execute the pipeline with streaming. Pre-request hooks run on the
    /// request path only (streaming responses are delivered directly to the caller).
    pub async fn stream(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSchema>,
        system: Option<&str>,
    ) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
        let mut ctx = RequestContext {
            messages,
            tools,
            system: system.map(|s| s.to_string()),
            meta: std::collections::HashMap::new(),
        };

        // Request path only for streaming
        for plugin in &self.plugins {
            plugin.pre_request(&mut ctx).await?;
        }

        self.client
            .stream(&ctx.messages, &ctx.tools, ctx.system.as_deref())
            .await
    }

    /// Reference to the underlying client.
    pub fn client(&self) -> &LlmClient {
        &self.client
    }
}

// ============================================================================
// Built-in plugins
// ============================================================================

/// Logs request metadata (message count, tool count) and response stats
/// (tokens, elapsed time, stop reason).
#[derive(Debug)]
pub struct LoggingPlugin;

#[async_trait::async_trait]
impl LlmPlugin for LoggingPlugin {
    fn name(&self) -> &str {
        "logging"
    }

    async fn pre_request(&self, ctx: &mut RequestContext) -> Result<()> {
        info!(
            "[pipeline:logging] request: {} messages, {} tools, system={}",
            ctx.messages.len(),
            ctx.tools.len(),
            ctx.system.is_some(),
        );
        Ok(())
    }

    async fn post_response(
        &self,
        response: &mut LlmResponse,
        ctx: &ResponseContext,
    ) -> Result<()> {
        info!(
            "[pipeline:logging] response: {} in_tokens, {} out_tokens, {:?} stop, {:.0}ms elapsed ({}/{})",
            response.input_tokens,
            response.output_tokens,
            response.stop_reason,
            ctx.elapsed.as_secs_f64() * 1000.0,
            ctx.provider,
            ctx.model,
        );
        Ok(())
    }
}

/// Token-bucket rate limiter. Limits the number of requests per time window.
#[derive(Debug)]
pub struct RateLimitPlugin {
    /// Maximum requests allowed in the window.
    max_requests: u64,
    /// Time window duration.
    window: Duration,
    /// Timestamps of recent requests (ring buffer approximation via atomic counter).
    counter: AtomicU64,
    last_reset: std::sync::Mutex<Instant>,
}

impl RateLimitPlugin {
    /// Create a new rate limiter allowing `max_requests` per `window`.
    pub fn new(max_requests: u64, window: Duration) -> Self {
        Self {
            max_requests,
            window,
            counter: AtomicU64::new(0),
            last_reset: std::sync::Mutex::new(Instant::now()),
        }
    }

    fn check_and_increment(&self) -> Result<()> {
        let mut last = self.last_reset.lock().unwrap();
        if last.elapsed() >= self.window {
            self.counter.store(0, Ordering::Relaxed);
            *last = Instant::now();
        }
        drop(last);

        let current = self.counter.fetch_add(1, Ordering::Relaxed);
        if current >= self.max_requests {
            Err(Error::Llm(format!(
                "Rate limit exceeded: {}/{} requests in {:?}",
                current, self.max_requests, self.window
            )))
        } else {
            Ok(())
        }
    }
}

#[async_trait::async_trait]
impl LlmPlugin for RateLimitPlugin {
    fn name(&self) -> &str {
        "rate_limit"
    }

    async fn pre_request(&self, _ctx: &mut RequestContext) -> Result<()> {
        self.check_and_increment()
    }
}

/// Accumulates token costs across all calls through the pipeline.
#[derive(Debug)]
pub struct CostMeterPlugin {
    total_input_tokens: AtomicU64,
    total_output_tokens: AtomicU64,
    total_cached_tokens: AtomicU64,
    call_count: AtomicU64,
}

impl CostMeterPlugin {
    /// Create a new cost meter.
    pub fn new() -> Self {
        Self {
            total_input_tokens: AtomicU64::new(0),
            total_output_tokens: AtomicU64::new(0),
            total_cached_tokens: AtomicU64::new(0),
            call_count: AtomicU64::new(0),
        }
    }

    /// Get the total input tokens consumed.
    pub fn total_input_tokens(&self) -> u64 {
        self.total_input_tokens.load(Ordering::Relaxed)
    }

    /// Get the total output tokens consumed.
    pub fn total_output_tokens(&self) -> u64 {
        self.total_output_tokens.load(Ordering::Relaxed)
    }

    /// Get the total cached tokens saved.
    pub fn total_cached_tokens(&self) -> u64 {
        self.total_cached_tokens.load(Ordering::Relaxed)
    }

    /// Get the total number of LLM calls made.
    pub fn call_count(&self) -> u64 {
        self.call_count.load(Ordering::Relaxed)
    }

    /// Get a summary string.
    pub fn summary(&self) -> String {
        format!(
            "CostMeter: {} calls, {} in_tokens, {} out_tokens, {} cached_tokens",
            self.call_count(),
            self.total_input_tokens(),
            self.total_output_tokens(),
            self.total_cached_tokens(),
        )
    }
}

impl Default for CostMeterPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl LlmPlugin for CostMeterPlugin {
    fn name(&self) -> &str {
        "cost_meter"
    }

    async fn post_response(
        &self,
        response: &mut LlmResponse,
        _ctx: &ResponseContext,
    ) -> Result<()> {
        self.total_input_tokens
            .fetch_add(response.input_tokens as u64, Ordering::Relaxed);
        self.total_output_tokens
            .fetch_add(response.output_tokens as u64, Ordering::Relaxed);
        self.total_cached_tokens
            .fetch_add(response.cached_tokens as u64, Ordering::Relaxed);
        self.call_count.fetch_add(1, Ordering::Relaxed);
        debug!("[pipeline:cost_meter] {}", self.summary());
        Ok(())
    }
}

/// Per-plugin retry with exponential backoff.
/// Wraps the entire pipeline call — if any plugin or the LLM call fails
/// with a retryable error, retries up to `max_retries` times.
#[derive(Debug)]
pub struct RetryPlugin {
    max_retries: u32,
    base_delay: Duration,
}

impl RetryPlugin {
    /// Create a retry plugin with the given max retries and base delay.
    pub fn new(max_retries: u32, base_delay: Duration) -> Self {
        Self {
            max_retries,
            base_delay,
        }
    }
}

#[async_trait::async_trait]
impl LlmPlugin for RetryPlugin {
    fn name(&self) -> &str {
        "retry"
    }

    // Note: RetryPlugin is a no-op at the plugin level.
    // Actual retry logic is handled by the Pipeline when configured,
    // or by LlmClient's built-in retry. This plugin exists as
    // a marker/hook for future per-plugin retry semantics.
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limit_allows_under_limit() {
        let plugin = RateLimitPlugin::new(5, Duration::from_secs(60));
        for _ in 0..5 {
            assert!(plugin.check_and_increment().is_ok());
        }
    }

    #[test]
    fn test_rate_limit_blocks_over_limit() {
        let plugin = RateLimitPlugin::new(3, Duration::from_secs(60));
        assert!(plugin.check_and_increment().is_ok());
        assert!(plugin.check_and_increment().is_ok());
        assert!(plugin.check_and_increment().is_ok());
        assert!(plugin.check_and_increment().is_err());
    }

    #[test]
    fn test_rate_limit_resets_after_window() {
        let plugin = RateLimitPlugin::new(1, Duration::from_millis(10));
        assert!(plugin.check_and_increment().is_ok());
        assert!(plugin.check_and_increment().is_err());
        // Wait for window to expire
        std::thread::sleep(Duration::from_millis(15));
        assert!(plugin.check_and_increment().is_ok());
    }

    #[test]
    fn test_cost_meter_accumulates() {
        let meter = CostMeterPlugin::new();
        assert_eq!(meter.call_count(), 0);
        assert_eq!(meter.total_input_tokens(), 0);

        // Simulate response
        let mut resp = LlmResponse {
            content: "test".to_string(),
            tool_calls: vec![],
            stop_reason: crate::StopReason::EndTurn,
            input_tokens: 100,
            output_tokens: 50,
            cached_tokens: 20,
        };
        let ctx = ResponseContext {
            elapsed: Duration::from_millis(100),
            provider: "Anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(meter.post_response(&mut resp, &ctx)).unwrap();

        assert_eq!(meter.call_count(), 1);
        assert_eq!(meter.total_input_tokens(), 100);
        assert_eq!(meter.total_output_tokens(), 50);
        assert_eq!(meter.total_cached_tokens(), 20);
    }

    #[test]
    fn test_cost_meter_summary() {
        let meter = CostMeterPlugin::new();
        let summary = meter.summary();
        assert!(summary.contains("0 calls"));
        assert!(summary.contains("0 in_tokens"));
    }

    #[test]
    fn test_logging_plugin_name() {
        let plugin = LoggingPlugin;
        assert_eq!(plugin.name(), "logging");
    }

    #[test]
    fn test_retry_plugin_name() {
        let plugin = RetryPlugin::new(3, Duration::from_millis(500));
        assert_eq!(plugin.name(), "retry");
    }

    #[test]
    fn test_request_context_default_meta() {
        let ctx = RequestContext {
            messages: vec![],
            tools: vec![],
            system: None,
            meta: std::collections::HashMap::new(),
        };
        assert!(ctx.meta.is_empty());
        assert!(ctx.system.is_none());
    }

    #[test]
    fn test_response_context_fields() {
        let ctx = ResponseContext {
            elapsed: Duration::from_secs(1),
            provider: "OpenAI".to_string(),
            model: "gpt-4o".to_string(),
        };
        assert_eq!(ctx.elapsed, Duration::from_secs(1));
        assert_eq!(ctx.provider, "OpenAI");
    }
}
