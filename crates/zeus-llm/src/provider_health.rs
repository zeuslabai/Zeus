//! Provider Health Tracking + Auto-Failover (OpenRouter-inspired)
//!
//! Tracks per-provider health metrics:
//! - Response latency (rolling average)
//! - Error classification (rate_limit, auth, server, timeout)
//! - Auto-recovery after cooldown period
//! - Health dashboard for monitoring

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Error classification for provider failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    /// 429 — rate limited, retry after cooldown
    RateLimit,
    /// 401/403 — auth failure, skip until credentials fixed
    AuthFailure,
    /// 500/502/503 — server error, retry with backoff
    ServerError,
    /// Request timed out
    Timeout,
    /// Network unreachable
    NetworkError,
    /// Unknown/other
    Other,
}

impl ProviderErrorKind {
    /// Classify an error string into a kind.
    pub fn classify(error: &str) -> Self {
        let lower = error.to_lowercase();
        if lower.contains("429") || lower.contains("rate limit") || lower.contains("too many") {
            Self::RateLimit
        } else if lower.contains("401") || lower.contains("403")
            || lower.contains("unauthorized") || lower.contains("forbidden")
            || lower.contains("authentication") || lower.contains("invalid.*key")
        {
            Self::AuthFailure
        } else if lower.contains("500") || lower.contains("502") || lower.contains("503")
            || lower.contains("internal server") || lower.contains("bad gateway")
            || lower.contains("service unavailable")
        {
            Self::ServerError
        } else if lower.contains("timeout") || lower.contains("timed out") {
            Self::Timeout
        } else if lower.contains("network") || lower.contains("connection refused")
            || lower.contains("dns") || lower.contains("unreachable")
        {
            Self::NetworkError
        } else {
            Self::Other
        }
    }

    /// Cooldown duration before retrying this provider.
    pub fn cooldown(&self) -> Duration {
        match self {
            Self::RateLimit => Duration::from_secs(60),     // 1 min — rate limits usually short
            Self::AuthFailure => Duration::from_secs(3600), // 1 hr — needs manual fix
            Self::ServerError => Duration::from_secs(120),  // 2 min — server may recover
            Self::Timeout => Duration::from_secs(30),       // 30s — may be transient
            Self::NetworkError => Duration::from_secs(300),  // 5 min — network issues persist
            Self::Other => Duration::from_secs(60),
        }
    }
}

/// Enhanced health metrics for a single provider.
#[derive(Debug, Clone)]
pub struct ProviderMetrics {
    /// Provider model string (e.g., "anthropic/claude-opus-4-6")
    pub model_string: String,
    /// Whether currently considered healthy
    pub healthy: bool,
    /// Consecutive failure count
    pub consecutive_failures: u32,
    /// Max failures before marking unhealthy
    pub max_failures: u32,
    /// Last error kind (if any)
    pub last_error: Option<ProviderErrorKind>,
    /// When the provider was last marked unhealthy
    pub unhealthy_since: Option<Instant>,
    /// Rolling average response time (ms)
    pub avg_latency_ms: f64,
    /// Total successful requests
    pub total_success: u64,
    /// Total failed requests
    pub total_failures: u64,
    /// Last successful request time
    pub last_success: Option<Instant>,
    /// Latency samples for rolling average (last 10)
    latency_samples: Vec<f64>,
}

impl ProviderMetrics {
    pub fn new(model_string: String) -> Self {
        Self {
            model_string,
            healthy: true,
            consecutive_failures: 0,
            max_failures: 3,
            last_error: None,
            unhealthy_since: None,
            avg_latency_ms: 0.0,
            total_success: 0,
            total_failures: 0,
            last_success: None,
            latency_samples: Vec::with_capacity(10),
        }
    }

    /// Record a successful request with its latency.
    pub fn record_success(&mut self, latency: Duration) {
        self.consecutive_failures = 0;
        self.healthy = true;
        self.unhealthy_since = None;
        self.last_error = None;
        self.total_success += 1;
        self.last_success = Some(Instant::now());

        // Rolling average of last 10 samples
        let ms = latency.as_millis() as f64;
        if self.latency_samples.len() >= 10 {
            self.latency_samples.remove(0);
        }
        self.latency_samples.push(ms);
        self.avg_latency_ms = self.latency_samples.iter().sum::<f64>()
            / self.latency_samples.len() as f64;

        debug!(
            "Provider '{}': success ({}ms, avg {}ms)",
            self.model_string,
            ms as u64,
            self.avg_latency_ms as u64
        );
    }

    /// Record a failed request with error classification.
    pub fn record_failure(&mut self, error: &str) {
        let kind = ProviderErrorKind::classify(error);
        self.consecutive_failures += 1;
        self.total_failures += 1;
        self.last_error = Some(kind);

        if self.consecutive_failures >= self.max_failures {
            self.healthy = false;
            self.unhealthy_since = Some(Instant::now());
            warn!(
                "Provider '{}' marked unhealthy: {} consecutive failures (last: {:?})",
                self.model_string, self.consecutive_failures, kind
            );
        }
    }

    /// Check if an unhealthy provider should be retried (cooldown expired).
    pub fn should_retry(&self) -> bool {
        if self.healthy {
            return true;
        }
        if let (Some(since), Some(kind)) = (self.unhealthy_since, self.last_error) {
            since.elapsed() >= kind.cooldown()
        } else {
            true // No error info — try anyway
        }
    }

    /// Uptime percentage (success / total).
    pub fn uptime_pct(&self) -> f64 {
        let total = self.total_success + self.total_failures;
        if total == 0 {
            100.0
        } else {
            (self.total_success as f64 / total as f64) * 100.0
        }
    }
}

/// Fleet-wide provider health registry.
#[derive(Debug, Clone)]
pub struct HealthRegistry {
    providers: Arc<RwLock<HashMap<String, ProviderMetrics>>>,
}

impl HealthRegistry {
    pub fn new() -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a provider (or get existing metrics).
    pub async fn register(&self, model_string: &str) {
        let mut providers = self.providers.write().await;
        providers
            .entry(model_string.to_string())
            .or_insert_with(|| ProviderMetrics::new(model_string.to_string()));
    }

    /// Record a successful request.
    pub async fn record_success(&self, model_string: &str, latency: Duration) {
        let mut providers = self.providers.write().await;
        if let Some(metrics) = providers.get_mut(model_string) {
            metrics.record_success(latency);
        }
    }

    /// Record a failed request.
    pub async fn record_failure(&self, model_string: &str, error: &str) {
        let mut providers = self.providers.write().await;
        if let Some(metrics) = providers.get_mut(model_string) {
            metrics.record_failure(error);
        }
    }

    /// Check if a provider is available (healthy or cooldown expired).
    pub async fn is_available(&self, model_string: &str) -> bool {
        let providers = self.providers.read().await;
        providers
            .get(model_string)
            .map(|m| m.should_retry())
            .unwrap_or(true) // Unknown provider = try it
    }

    /// Get the best available provider from a list (healthy + lowest latency).
    /// Returns the model string of the best provider, or None if all unavailable.
    pub async fn best_provider(&self, candidates: &[String]) -> Option<String> {
        let providers = self.providers.read().await;
        candidates
            .iter()
            .filter(|c| {
                providers
                    .get(c.as_str())
                    .map(|m| m.should_retry())
                    .unwrap_or(true)
            })
            .min_by(|a, b| {
                let la = providers.get(a.as_str()).map(|m| m.avg_latency_ms).unwrap_or(f64::MAX);
                let lb = providers.get(b.as_str()).map(|m| m.avg_latency_ms).unwrap_or(f64::MAX);
                la.partial_cmp(&lb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned()
    }

    /// Get a health dashboard snapshot.
    pub async fn dashboard(&self) -> Vec<ProviderMetrics> {
        let providers = self.providers.read().await;
        providers.values().cloned().collect()
    }
}

impl Default for HealthRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_classification() {
        assert_eq!(ProviderErrorKind::classify("429 Too Many Requests"), ProviderErrorKind::RateLimit);
        assert_eq!(ProviderErrorKind::classify("401 Unauthorized"), ProviderErrorKind::AuthFailure);
        assert_eq!(ProviderErrorKind::classify("403 Forbidden"), ProviderErrorKind::AuthFailure);
        assert_eq!(ProviderErrorKind::classify("500 Internal Server Error"), ProviderErrorKind::ServerError);
        assert_eq!(ProviderErrorKind::classify("Request timed out"), ProviderErrorKind::Timeout);
        assert_eq!(ProviderErrorKind::classify("connection refused"), ProviderErrorKind::NetworkError);
        assert_eq!(ProviderErrorKind::classify("something weird"), ProviderErrorKind::Other);
    }

    #[test]
    fn test_provider_health_lifecycle() {
        let mut m = ProviderMetrics::new("test/model".to_string());
        assert!(m.healthy);
        assert!(m.should_retry());

        // 3 failures → unhealthy
        m.record_failure("500 error");
        m.record_failure("500 error");
        m.record_failure("500 error");
        assert!(!m.healthy);

        // Success resets
        m.record_success(Duration::from_millis(100));
        assert!(m.healthy);
        assert_eq!(m.consecutive_failures, 0);
    }

    #[test]
    fn test_latency_tracking() {
        let mut m = ProviderMetrics::new("test/model".to_string());
        m.record_success(Duration::from_millis(100));
        m.record_success(Duration::from_millis(200));
        m.record_success(Duration::from_millis(300));
        assert!((m.avg_latency_ms - 200.0).abs() < 0.1);
    }

    #[test]
    fn test_uptime_pct() {
        let mut m = ProviderMetrics::new("test/model".to_string());
        m.record_success(Duration::from_millis(100));
        m.record_success(Duration::from_millis(100));
        m.record_failure("error");
        // 2 success, 1 failure = 66.67%
        assert!((m.uptime_pct() - 66.67).abs() < 1.0);
    }

    #[tokio::test]
    async fn test_health_registry() {
        let reg = HealthRegistry::new();
        reg.register("anthropic/claude").await;
        reg.register("openai/gpt-4o").await;

        reg.record_success("anthropic/claude", Duration::from_millis(500)).await;
        reg.record_success("openai/gpt-4o", Duration::from_millis(200)).await;

        // gpt-4o has lower latency
        let candidates = vec![
            "anthropic/claude".to_string(),
            "openai/gpt-4o".to_string(),
        ];
        let best = reg.best_provider(&candidates).await;
        assert_eq!(best.as_deref(), Some("openai/gpt-4o"));
    }

    #[tokio::test]
    async fn test_unhealthy_provider_skipped() {
        let reg = HealthRegistry::new();
        reg.register("bad/model").await;
        reg.register("good/model").await;

        // Make bad/model unhealthy
        reg.record_failure("bad/model", "500 error").await;
        reg.record_failure("bad/model", "500 error").await;
        reg.record_failure("bad/model", "500 error").await;

        reg.record_success("good/model", Duration::from_millis(100)).await;

        assert!(!reg.is_available("bad/model").await); // cooldown not expired
        assert!(reg.is_available("good/model").await);

        let candidates = vec![
            "bad/model".to_string(),
            "good/model".to_string(),
        ];
        let best = reg.best_provider(&candidates).await;
        assert_eq!(best.as_deref(), Some("good/model"));
    }
}
