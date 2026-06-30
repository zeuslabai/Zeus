//! FallbackProvider — wraps multiple LLM providers with automatic failover.
//!
//! On each call (complete or stream), tries providers in order. If one fails
//! (timeout, network error, auth error), automatically falls through to the next.
//! Includes health probing to skip known-unhealthy providers.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use zeus_core::{Config, Error, Message, Result, ToolSchema};

use crate::{LlmClient, LlmResponse};

// ============================================================================
// ProviderHealth
// ============================================================================

/// Health status of a single provider.
#[derive(Debug, Clone)]
pub struct ProviderHealth {
    /// The model string (e.g., "anthropic/claude-sonnet-4-20250514")
    pub model_string: String,
    /// Whether this provider is considered healthy
    pub healthy: bool,
    /// Number of consecutive failures
    pub consecutive_failures: u32,
    /// Maximum consecutive failures before marking unhealthy
    pub max_failures: u32,
}

impl ProviderHealth {
    fn new(model_string: String) -> Self {
        Self {
            model_string,
            healthy: true,
            consecutive_failures: 0,
            max_failures: 3,
        }
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.healthy = true;
    }

    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        if self.consecutive_failures >= self.max_failures {
            self.healthy = false;
            warn!(
                "Provider '{}' marked unhealthy after {} consecutive failures",
                self.model_string, self.consecutive_failures
            );
        }
    }
}

// ============================================================================
// FallbackProvider
// ============================================================================

/// Wraps multiple LlmClient instances and provides automatic failover.
///
/// On failure, tries the next provider in the list. Tracks health per provider
/// and skips unhealthy ones (but still tries them as last resort).
pub struct FallbackProvider {
    /// Ordered list of (client, health) pairs
    providers: Vec<(LlmClient, Arc<RwLock<ProviderHealth>>)>,
    /// Timeout per individual provider attempt
    attempt_timeout: Duration,
    /// Model string of the provider that served the most recent successful request.
    /// `None` until the first successful call.
    last_used_provider: Arc<RwLock<Option<String>>>,
}

impl FallbackProvider {
    /// Create a FallbackProvider from a list of model strings.
    ///
    /// Each model string is in "provider/model" format (same as config.model).
    /// Returns error only if zero providers could be created.
    pub fn from_model_strings(model_strings: &[String]) -> Result<Self> {
        let mut providers = Vec::new();

        for model_str in model_strings {
            let temp_config = Config {
                model: model_str.clone(),
                ..Config::default()
            };
            match LlmClient::from_config(&temp_config) {
                Ok(client) => {
                    let health = Arc::new(RwLock::new(ProviderHealth::new(model_str.clone())));
                    providers.push((client, health));
                    debug!("FallbackProvider: added provider '{}'", model_str);
                }
                Err(e) => {
                    warn!(
                        "FallbackProvider: skipping '{}' (init failed: {})",
                        model_str, e
                    );
                }
            }
        }

        if providers.is_empty() {
            return Err(Error::Llm(
                "FallbackProvider: no providers could be initialized".to_string(),
            ));
        }

        info!(
            "FallbackProvider initialized with {} providers",
            providers.len()
        );

        Ok(Self {
            providers,
            attempt_timeout: Duration::from_secs(60),
            last_used_provider: Arc::new(RwLock::new(None)),
        })
    }

    /// Create from config — uses primary model + fallback_models list.
    pub fn from_config(config: &Config) -> Result<Self> {
        let mut model_strings = vec![config.model.clone()];
        if let Some(ref fallbacks) = config.fallback_models {
            model_strings.extend(fallbacks.iter().cloned());
        }
        Self::from_model_strings(&model_strings)
    }

    /// Set the per-attempt timeout (default: 60s).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.attempt_timeout = timeout;
        self
    }

    /// Number of configured providers.
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    /// Returns the model string of the provider that served the last successful request.
    pub async fn last_used_provider(&self) -> Option<String> {
        self.last_used_provider.read().await.clone()
    }

    /// Get health status of all providers.
    pub async fn health_status(&self) -> Vec<ProviderHealth> {
        let mut statuses = Vec::new();
        for (_, health) in &self.providers {
            statuses.push(health.read().await.clone());
        }
        statuses
    }

    /// Probe all providers with a minimal request to check connectivity.
    ///
    /// Sends a tiny completion request to each provider. Marks providers
    /// as healthy/unhealthy based on whether they respond.
    pub async fn probe_health(&self) {
        let test_messages = vec![Message::user("ping")];
        let test_tools: Vec<ToolSchema> = vec![];

        for (client, health) in &self.providers {
            let model_str = health.read().await.model_string.clone();
            debug!("Probing health of '{}'", model_str);

            let result = tokio::time::timeout(
                Duration::from_secs(15),
                client.complete(&test_messages, &test_tools, Some("Reply with pong.")),
            )
            .await;

            let mut h = health.write().await;
            match result {
                Ok(Ok(_)) => {
                    h.record_success();
                    info!("Provider '{}': healthy", model_str);
                }
                Ok(Err(e)) => {
                    h.record_failure();
                    warn!("Provider '{}': probe failed: {}", model_str, e);
                }
                Err(_) => {
                    h.record_failure();
                    warn!("Provider '{}': probe timed out", model_str);
                }
            }
        }
    }

    /// Complete with automatic failover.
    ///
    /// Retries each provider up to `max_retries` times before switching to the next.
    /// Tries healthy providers first (in order), then unhealthy ones as last resort.
    pub async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<LlmResponse> {
        self.complete_with_notify(messages, tools, system, None).await
    }

    /// Complete with failover, optionally notifying a channel when switching providers.
    pub async fn complete_with_notify(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
        notify: Option<&tokio::sync::mpsc::Sender<String>>,
    ) -> Result<LlmResponse> {
        let order = self.provider_order().await;
        let max_retries = 3usize;

        let mut last_error = Error::Llm("No providers available".to_string());

        for (attempt_idx, idx) in order.iter().enumerate() {
            let (client, health) = &self.providers[*idx];
            let model_str = health.read().await.model_string.clone();

            if attempt_idx > 0 {
                let prev = self.last_used_provider.read().await.clone();
                let msg = match prev {
                    Some(ref p) => format!(
                        "⚠️ Switched from provider '{}' to '{}' after failure.",
                        p, model_str
                    ),
                    None => format!(
                        "⚠️ Switched to provider '{}' after previous provider failed.",
                        model_str
                    ),
                };
                warn!("FallbackProvider: {}", msg);
                if let Some(tx) = notify {
                    let _ = tx.try_send(msg);
                }
            }

            for retry in 0..max_retries {
                debug!(
                    "FallbackProvider: trying '{}' for complete (attempt {}/{})",
                    model_str,
                    retry + 1,
                    max_retries
                );

                let result = tokio::time::timeout(
                    self.attempt_timeout,
                    client.complete(messages, tools, system),
                )
                .await;

                match result {
                    Ok(Ok(response)) => {
                        health.write().await.record_success();
                        *self.last_used_provider.write().await = Some(model_str.clone());
                        return Ok(response);
                    }
                    Ok(Err(e)) => {
                        warn!(
                            "FallbackProvider: '{}' failed (attempt {}/{}): {}",
                            model_str,
                            retry + 1,
                            max_retries,
                            e
                        );
                        last_error = e;
                    }
                    Err(_) => {
                        warn!(
                            "FallbackProvider: '{}' timed out (attempt {}/{})",
                            model_str,
                            retry + 1,
                            max_retries
                        );
                        last_error =
                            Error::Llm(format!("Provider '{}' timed out", model_str));
                    }
                }
            }

            // All retries exhausted for this provider — mark unhealthy and move on
            health.write().await.record_failure();
        }

        Err(last_error)
    }

    /// Stream with automatic failover.
    ///
    /// Retries each provider up to `max_retries` times before switching to the next.
    /// Tries healthy providers first; on failure falls through to next.
    pub async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
    ) -> Result<(
        tokio::sync::mpsc::Receiver<String>,
        tokio::task::JoinHandle<LlmResponse>,
    )> {
        self.stream_with_notify(messages, tools, system, None).await
    }

    /// Stream with failover, optionally notifying a channel when switching providers.
    pub async fn stream_with_notify(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        system: Option<&str>,
        notify: Option<&tokio::sync::mpsc::Sender<String>>,
    ) -> Result<(
        tokio::sync::mpsc::Receiver<String>,
        tokio::task::JoinHandle<LlmResponse>,
    )> {
        let order = self.provider_order().await;
        let max_retries = 3usize;

        let mut last_error = Error::Llm("No providers available".to_string());

        for (attempt_idx, idx) in order.iter().enumerate() {
            let (client, health) = &self.providers[*idx];
            let model_str = health.read().await.model_string.clone();

            if attempt_idx > 0 {
                let prev = self.last_used_provider.read().await.clone();
                let msg = match prev {
                    Some(ref p) => format!(
                        "⚠️ Switched from provider '{}' to '{}' after failure.",
                        p, model_str
                    ),
                    None => format!(
                        "⚠️ Switched to provider '{}' after previous provider failed.",
                        model_str
                    ),
                };
                warn!("FallbackProvider: {}", msg);
                if let Some(tx) = notify {
                    let _ = tx.try_send(msg);
                }
            }

            for retry in 0..max_retries {
                debug!(
                    "FallbackProvider: trying '{}' for stream (attempt {}/{})",
                    model_str,
                    retry + 1,
                    max_retries
                );

                match client.stream(messages, tools, system).await {
                    Ok(result) => {
                        health.write().await.record_success();
                        *self.last_used_provider.write().await = Some(model_str.clone());
                        return Ok(result);
                    }
                    Err(e) => {
                        warn!(
                            "FallbackProvider: '{}' stream failed (attempt {}/{}): {}",
                            model_str,
                            retry + 1,
                            max_retries,
                            e
                        );
                        last_error = e;
                    }
                }
            }

            // All retries exhausted for this provider — mark unhealthy and move on
            health.write().await.record_failure();
        }

        Err(last_error)
    }

    /// Get provider indices in priority order: healthy first, then unhealthy.
    async fn provider_order(&self) -> Vec<usize> {
        let mut healthy = Vec::new();
        let mut unhealthy = Vec::new();

        for (idx, (_, health)) in self.providers.iter().enumerate() {
            if health.read().await.healthy {
                healthy.push(idx);
            } else {
                unhealthy.push(idx);
            }
        }

        healthy.extend(unhealthy);
        healthy
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_health_initial() {
        let h = ProviderHealth::new("test/model".to_string());
        assert!(h.healthy);
        assert_eq!(h.consecutive_failures, 0);
    }

    #[test]
    fn test_provider_health_success_resets() {
        let mut h = ProviderHealth::new("test/model".to_string());
        h.record_failure();
        h.record_failure();
        assert_eq!(h.consecutive_failures, 2);
        h.record_success();
        assert_eq!(h.consecutive_failures, 0);
        assert!(h.healthy);
    }

    #[test]
    fn test_provider_health_marks_unhealthy() {
        let mut h = ProviderHealth::new("test/model".to_string());
        h.record_failure();
        h.record_failure();
        assert!(h.healthy); // still healthy at 2
        h.record_failure();
        assert!(!h.healthy); // unhealthy at 3 (max_failures default)
    }

    #[test]
    fn test_provider_health_recovery() {
        let mut h = ProviderHealth::new("test/model".to_string());
        for _ in 0..5 {
            h.record_failure();
        }
        assert!(!h.healthy);
        h.record_success();
        assert!(h.healthy);
        assert_eq!(h.consecutive_failures, 0);
    }

    #[test]
    fn test_fallback_from_empty_list() {
        let result = FallbackProvider::from_model_strings(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_fallback_from_invalid_models() {
        // Empty model strings should fail to create clients
        let result = FallbackProvider::from_model_strings(&["".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_fallback_from_model_strings_ollama() {
        // Ollama doesn't need API keys, so it should initialize
        let result = FallbackProvider::from_model_strings(&["ollama/llama3.2".to_string()]);
        assert!(result.is_ok());
        let fb = result.unwrap();
        assert_eq!(fb.provider_count(), 1);
    }

    #[test]
    fn test_fallback_with_timeout() {
        let fb = FallbackProvider::from_model_strings(&["ollama/llama3.2".to_string()])
            .unwrap()
            .with_timeout(Duration::from_secs(30));
        assert_eq!(fb.attempt_timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_fallback_from_config() {
        let mut config = Config::default();
        config.model = "ollama/llama3.2".to_string();
        config.fallback_models = Some(vec!["ollama/mistral".to_string()]);

        let result = FallbackProvider::from_config(&config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().provider_count(), 2);
    }

    #[test]
    fn test_fallback_from_config_no_fallbacks() {
        let mut config = Config::default();
        config.model = "ollama/llama3.2".to_string();
        config.fallback_models = None;

        let result = FallbackProvider::from_config(&config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().provider_count(), 1);
    }

    #[tokio::test]
    async fn test_health_status() {
        let fb = FallbackProvider::from_model_strings(&["ollama/llama3.2".to_string()]).unwrap();
        let statuses = fb.health_status().await;
        assert_eq!(statuses.len(), 1);
        assert!(statuses[0].healthy);
        assert_eq!(statuses[0].model_string, "ollama/llama3.2");
    }

    #[tokio::test]
    async fn test_provider_order_all_healthy() {
        let fb = FallbackProvider::from_model_strings(&[
            "ollama/llama3.2".to_string(),
            "ollama/mistral".to_string(),
        ])
        .unwrap();
        let order = fb.provider_order().await;
        assert_eq!(order, vec![0, 1]);
    }

    #[tokio::test]
    async fn test_provider_order_first_unhealthy() {
        let fb = FallbackProvider::from_model_strings(&[
            "ollama/llama3.2".to_string(),
            "ollama/mistral".to_string(),
        ])
        .unwrap();
        // Mark first as unhealthy
        {
            let mut h = fb.providers[0].1.write().await;
            for _ in 0..3 {
                h.record_failure();
            }
        }
        let order = fb.provider_order().await;
        // Second (healthy) comes first, then first (unhealthy)
        assert_eq!(order, vec![1, 0]);
    }

    #[tokio::test]
    async fn test_complete_no_providers_error() {
        // Requires a live Ollama instance — skip in CI unless ZEUS_TEST_LIVE is set
        if std::env::var("ZEUS_TEST_LIVE").is_err() {
            return;
        }
        // Create provider but mark all unhealthy — still tries them
        let fb =
            FallbackProvider::from_model_strings(&["ollama/nonexistent-model-xyz".to_string()])
                .unwrap();
        // complete will fail because ollama probably isn't running or model doesn't exist
        // but it should return an error, not panic
        let result = fb
            .complete(&[Message::user("test")], &[], Some("test"))
            .await;
        // Either succeeds (if ollama happens to be running) or fails gracefully
        if result.is_err() {
            let err = result.unwrap_err().to_string();
            assert!(!err.is_empty());
        }
    }

    // ========================================================================
    // Integration tests for complete_with_notify retry/switch semantics.
    //
    // These stand up a local mock HTTP server returning HTTP 400 (non-retryable
    // per `is_retryable_status` in lib.rs, so complete_ollama's inner 2^n
    // backoff loop is skipped — keeps tests under a second) and point
    // LlmClient instances at it via OLLAMA_HOST. Env mutations are serialized
    // through ENV_LOCK to avoid racing parallel tests.
    // ========================================================================

    use std::sync::Mutex as StdMutex;

    /// Serializes tests that mutate OLLAMA_HOST env var.
    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    /// Spawn a minimal HTTP mock that returns a canned response to every connection.
    /// Returns the base URL (e.g. `http://127.0.0.1:54321`) and the server JoinHandle.
    /// The handle should be aborted at end of test to release the listener.
    async fn spawn_mock_http(
        status_line: &'static str,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock server should bind");
        let port = listener
            .local_addr()
            .expect("mock listener should have addr")
            .port();
        let url = format!("http://127.0.0.1:{}", port);

        let handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((mut stream, _)) => {
                        let mut buf = vec![0u8; 8192];
                        let _ = stream.read(&mut buf).await;
                        let response = format!(
                            "{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            status_line,
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        let _ = stream.shutdown().await;
                    }
                    Err(_) => break,
                }
            }
        });

        (url, handle)
    }

    #[tokio::test]
    async fn test_complete_with_notify_switches_providers_on_failure() {
        use crate::{LlmClient, Provider};
        use tokio::sync::mpsc;

        // HTTP 400 is NOT in is_retryable_status → complete_ollama bails on
        // the first response instead of running its inner 2^n backoff retry.
        let (mock_url, mock_handle) = spawn_mock_http(
            "HTTP/1.1 400 Bad Request",
            r#"{"error":"mock bad request"}"#,
        )
        .await;

        // LlmClient::new reads OLLAMA_HOST at construct time and caches it in
        // self.base_url. Serialize the env mutation across parallel tests.
        let guard = ENV_LOCK.lock().expect("env lock not poisoned");
        unsafe {
            std::env::set_var("OLLAMA_HOST", &mock_url);
        }

        let client_a = LlmClient::new(Provider::Ollama, "mock-a".to_string())
            .expect("client a should construct");
        let client_b = LlmClient::new(Provider::Ollama, "mock-b".to_string())
            .expect("client b should construct");

        unsafe {
            std::env::remove_var("OLLAMA_HOST");
        }
        drop(guard);

        // Build a FallbackProvider directly (bypasses from_config's Config::default
        // which would otherwise override our OLLAMA_HOST-based base URL).
        let fb = FallbackProvider {
            providers: vec![
                (
                    client_a,
                    Arc::new(RwLock::new(ProviderHealth::new(
                        "ollama/mock-a".to_string(),
                    ))),
                ),
                (
                    client_b,
                    Arc::new(RwLock::new(ProviderHealth::new(
                        "ollama/mock-b".to_string(),
                    ))),
                ),
            ],
            attempt_timeout: Duration::from_secs(2),
            last_used_provider: Arc::new(RwLock::new(None)),
        };

        let (tx, mut rx) = mpsc::channel::<String>(16);
        let result = fb
            .complete_with_notify(&[Message::user("ping")], &[], Some("sys"), Some(&tx))
            .await;

        // Both providers returned 400 on every retry → overall error.
        assert!(
            result.is_err(),
            "expected error when every provider returns 400"
        );

        // Drain the notify channel. When FallbackProvider moves from provider A
        // to provider B it should push a "Switched to provider" message.
        let mut notifications = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            notifications.push(msg);
        }
        assert!(
            notifications.iter().any(|m| m.contains("Switched to provider")),
            "expected at least one 'Switched to provider' notification, got: {:?}",
            notifications
        );

        // One `record_failure` per provider per complete() call (after its
        // max_retries=3 inner loop exhausts). So after a single complete_with_notify
        // both providers should have consecutive_failures == 1.
        let statuses = fb.health_status().await;
        assert_eq!(statuses.len(), 2);
        for status in &statuses {
            assert_eq!(
                status.consecutive_failures, 1,
                "expected 1 failure per provider after one exhausted complete() call, got: {:?}",
                status
            );
            assert!(
                status.healthy,
                "1 failure is below max_failures=3, provider should still be healthy"
            );
        }

        mock_handle.abort();
    }

    #[tokio::test]
    async fn test_complete_with_notify_first_notify_fires_only_on_switch() {
        // Verifies the notify channel receives NOTHING if the very first
        // provider in the order succeeds on first try (i.e. the `attempt_idx > 0`
        // guard around the switch message is honored).
        use crate::{LlmClient, Provider};
        use tokio::sync::mpsc;

        // Mock returns a valid Ollama chat response.
        // NOTE: Ollama now routes through the OpenAI-compatible /v1/chat/completions
        // path (LlmClient::complete → complete_openai), so the response must use the
        // OpenAI shape ({choices:[{message:{content}}]}), NOT the legacy native-Ollama
        // shape ({message:{content},eval_count}). The old fixture left content="" and
        // failed the content==pong assert (#109).
        let (mock_url, mock_handle) = spawn_mock_http(
            "HTTP/1.1 200 OK",
            r#"{"choices":[{"message":{"content":"pong","tool_calls":[]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#,
        )
        .await;

        let guard = ENV_LOCK.lock().expect("env lock not poisoned");
        unsafe {
            std::env::set_var("OLLAMA_HOST", &mock_url);
        }
        let client = LlmClient::new(Provider::Ollama, "mock-ok".to_string())
            .expect("client should construct");
        unsafe {
            std::env::remove_var("OLLAMA_HOST");
        }
        drop(guard);

        let fb = FallbackProvider {
            providers: vec![(
                client,
                Arc::new(RwLock::new(ProviderHealth::new(
                    "ollama/mock-ok".to_string(),
                ))),
            )],
            attempt_timeout: Duration::from_secs(2),
            last_used_provider: Arc::new(RwLock::new(None)),
        };

        let (tx, mut rx) = mpsc::channel::<String>(16);
        let result = fb
            .complete_with_notify(&[Message::user("ping")], &[], None, Some(&tx))
            .await;

        assert!(result.is_ok(), "expected success from 200 mock");
        let response = result.unwrap();
        assert_eq!(response.content, "pong");

        // No switch happened → no notify messages.
        assert!(
            rx.try_recv().is_err(),
            "notify channel should be empty when first provider succeeds"
        );

        // Success path should have reset consecutive_failures to 0.
        let statuses = fb.health_status().await;
        assert_eq!(statuses[0].consecutive_failures, 0);
        assert!(statuses[0].healthy);

        mock_handle.abort();
    }

    #[tokio::test]
    async fn test_provider_order_all_unhealthy_preserves_original_order() {
        let fb = FallbackProvider::from_model_strings(&[
            "ollama/a".to_string(),
            "ollama/b".to_string(),
            "ollama/c".to_string(),
        ])
        .expect("all-ollama construction should succeed");

        // Drive every provider to unhealthy via max_failures consecutive failures.
        for (_, health) in &fb.providers {
            let mut h = health.write().await;
            for _ in 0..3 {
                h.record_failure();
            }
        }

        let order = fb.provider_order().await;
        assert_eq!(
            order,
            vec![0, 1, 2],
            "when all providers are unhealthy, order should preserve the original index sequence"
        );
    }

    #[tokio::test]
    async fn test_provider_order_mixed_preserves_within_groups() {
        let fb = FallbackProvider::from_model_strings(&[
            "ollama/a".to_string(), // stays healthy
            "ollama/b".to_string(), // marked unhealthy
            "ollama/c".to_string(), // stays healthy
            "ollama/d".to_string(), // marked unhealthy
        ])
        .expect("construction should succeed");

        for idx in [1usize, 3] {
            let mut h = fb.providers[idx].1.write().await;
            for _ in 0..3 {
                h.record_failure();
            }
        }

        let order = fb.provider_order().await;
        assert_eq!(
            order,
            vec![0, 2, 1, 3],
            "healthy providers first preserving internal order, then unhealthy preserving internal order"
        );
    }
}
