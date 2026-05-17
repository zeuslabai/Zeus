//! Rate Limiter — Token bucket rate limiting for API and tool execution
//!
//! Provides per-client and per-endpoint rate limiting to protect Zeus
//! from abuse and ensure fair resource sharing between agents.
//!
//! Strategies:
//! - **Token bucket**: smooth rate limiting with burst allowance
//! - **Per-key limits**: different limits for different API keys/agents
//! - **Endpoint-specific**: different limits for different endpoints
//! - **Global limit**: overall system-wide rate cap

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::debug;

// ============================================================================
// Configuration
// ============================================================================

/// Rate limiter configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Enable rate limiting
    pub enabled: bool,
    /// Default requests per minute per client
    pub default_rpm: u32,
    /// Default burst size (max tokens in bucket)
    pub default_burst: u32,
    /// Per-endpoint overrides (endpoint → rpm)
    pub endpoint_limits: HashMap<String, EndpointLimit>,
    /// Global system-wide requests per minute cap
    pub global_rpm: u32,
}

/// Per-endpoint rate limit configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointLimit {
    pub rpm: u32,
    pub burst: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_rpm: 60,
            default_burst: 10,
            endpoint_limits: HashMap::new(),
            global_rpm: 1000,
        }
    }
}

// ============================================================================
// Token Bucket
// ============================================================================

/// A single token bucket for rate limiting
#[derive(Debug, Clone)]
struct TokenBucket {
    /// Current number of tokens available
    tokens: f64,
    /// Maximum tokens (burst capacity)
    max_tokens: f64,
    /// Token refill rate per second
    refill_rate: f64,
    /// Last time tokens were refilled
    last_refill: DateTime<Utc>,
    /// Total requests allowed
    total_allowed: u64,
    /// Total requests denied
    total_denied: u64,
}

impl TokenBucket {
    fn new(max_tokens: u32, rpm: u32) -> Self {
        Self {
            tokens: max_tokens as f64,
            max_tokens: max_tokens as f64,
            refill_rate: rpm as f64 / 60.0,
            last_refill: Utc::now(),
            total_allowed: 0,
            total_denied: 0,
        }
    }

    /// Try to consume one token, returning true if allowed
    fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            self.total_allowed += 1;
            true
        } else {
            self.total_denied += 1;
            false
        }
    }

    /// Refill tokens based on elapsed time
    fn refill(&mut self) {
        let now = Utc::now();
        let elapsed_secs = (now - self.last_refill).num_milliseconds() as f64 / 1000.0;
        if elapsed_secs > 0.0 {
            self.tokens = (self.tokens + elapsed_secs * self.refill_rate).min(self.max_tokens);
            self.last_refill = now;
        }
    }

    /// Time until next token is available (seconds)
    fn wait_time_secs(&self) -> f64 {
        if self.tokens >= 1.0 {
            return 0.0;
        }
        let needed = 1.0 - self.tokens;
        if self.refill_rate > 0.0 {
            needed / self.refill_rate
        } else {
            f64::INFINITY
        }
    }

    /// Reset the bucket to full
    fn reset(&mut self) {
        self.tokens = self.max_tokens;
        self.last_refill = Utc::now();
    }
}

// ============================================================================
// Rate Limit Result
// ============================================================================

/// Result of a rate limit check
#[derive(Debug, Clone, PartialEq)]
pub enum RateLimitResult {
    /// Request is allowed
    Allowed { remaining: u32 },
    /// Request is denied (rate limited)
    Limited { retry_after_secs: f64 },
}

impl RateLimitResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    pub fn is_limited(&self) -> bool {
        matches!(self, Self::Limited { .. })
    }
}

// ============================================================================
// Rate Limiter
// ============================================================================

/// Rate limiter managing per-client and per-endpoint buckets
pub struct RateLimiter {
    config: RateLimitConfig,
    /// Per-client buckets (client_id → bucket)
    client_buckets: HashMap<String, TokenBucket>,
    /// Per-client-endpoint buckets (client_id:endpoint → bucket)
    endpoint_buckets: HashMap<String, TokenBucket>,
    /// Global bucket
    global_bucket: TokenBucket,
    /// Stats
    stats: RateLimitStats,
}

/// Rate limiter statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RateLimitStats {
    pub total_checks: u64,
    pub total_allowed: u64,
    pub total_limited: u64,
    pub total_global_limited: u64,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        let global_bucket = TokenBucket::new(
            config.global_rpm / 6, // ~10s burst at global rate
            config.global_rpm,
        );
        Self {
            config,
            client_buckets: HashMap::new(),
            endpoint_buckets: HashMap::new(),
            global_bucket,
            stats: RateLimitStats::default(),
        }
    }

    /// Check if a request should be allowed
    pub fn check(&mut self, client_id: &str, endpoint: Option<&str>) -> RateLimitResult {
        self.stats.total_checks += 1;

        if !self.config.enabled {
            self.stats.total_allowed += 1;
            return RateLimitResult::Allowed {
                remaining: u32::MAX,
            };
        }

        // 1. Check global limit
        if !self.global_bucket.try_consume() {
            self.stats.total_limited += 1;
            self.stats.total_global_limited += 1;
            let wait = self.global_bucket.wait_time_secs();
            debug!(client = %client_id, "Global rate limit hit");
            return RateLimitResult::Limited {
                retry_after_secs: wait,
            };
        }

        // 2. Check endpoint-specific limit (if configured)
        if let Some(ep) = endpoint
            && let Some(ep_limit) = self.config.endpoint_limits.get(ep)
        {
            let key = format!("{}:{}", client_id, ep);
            let bucket = self
                .endpoint_buckets
                .entry(key)
                .or_insert_with(|| TokenBucket::new(ep_limit.burst, ep_limit.rpm));

            if !bucket.try_consume() {
                self.stats.total_limited += 1;
                let wait = bucket.wait_time_secs();
                debug!(client = %client_id, endpoint = %ep, "Endpoint rate limit hit");
                return RateLimitResult::Limited {
                    retry_after_secs: wait,
                };
            }
        }

        // 3. Check per-client limit
        let default_rpm = self.config.default_rpm;
        let default_burst = self.config.default_burst;
        let bucket = self
            .client_buckets
            .entry(client_id.to_string())
            .or_insert_with(|| TokenBucket::new(default_burst, default_rpm));

        if !bucket.try_consume() {
            self.stats.total_limited += 1;
            let wait = bucket.wait_time_secs();
            debug!(client = %client_id, "Client rate limit hit");
            return RateLimitResult::Limited {
                retry_after_secs: wait,
            };
        }

        self.stats.total_allowed += 1;
        let remaining = bucket.tokens as u32;
        RateLimitResult::Allowed { remaining }
    }

    /// Reset limits for a specific client
    pub fn reset_client(&mut self, client_id: &str) {
        if let Some(bucket) = self.client_buckets.get_mut(client_id) {
            bucket.reset();
        }
        // Also reset endpoint-specific buckets for this client
        for (key, bucket) in self.endpoint_buckets.iter_mut() {
            if key.starts_with(&format!("{}:", client_id)) {
                bucket.reset();
            }
        }
    }

    /// Reset all limits
    pub fn reset_all(&mut self) {
        for bucket in self.client_buckets.values_mut() {
            bucket.reset();
        }
        for bucket in self.endpoint_buckets.values_mut() {
            bucket.reset();
        }
        self.global_bucket.reset();
    }

    /// Get per-client usage report
    pub fn client_report(&self) -> Vec<ClientUsage> {
        self.client_buckets
            .iter()
            .map(|(id, bucket)| ClientUsage {
                client_id: id.clone(),
                total_allowed: bucket.total_allowed,
                total_denied: bucket.total_denied,
                tokens_remaining: bucket.tokens as u32,
                max_tokens: bucket.max_tokens as u32,
            })
            .collect()
    }

    /// Number of tracked clients
    pub fn client_count(&self) -> usize {
        self.client_buckets.len()
    }

    /// Get statistics
    pub fn stats(&self) -> &RateLimitStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = RateLimitStats::default();
    }

    /// Get config
    pub fn config(&self) -> &RateLimitConfig {
        &self.config
    }

    /// Update config (recreates global bucket)
    pub fn set_config(&mut self, config: RateLimitConfig) {
        self.global_bucket = TokenBucket::new(config.global_rpm / 6, config.global_rpm);
        self.config = config;
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(RateLimitConfig::default())
    }
}

// ============================================================================
// Client Usage Report
// ============================================================================

/// Per-client usage stats
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientUsage {
    pub client_id: String,
    pub total_allowed: u64,
    pub total_denied: u64,
    pub tokens_remaining: u32,
    pub max_tokens: u32,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn test_limiter() -> RateLimiter {
        RateLimiter::new(RateLimitConfig {
            enabled: true,
            default_rpm: 60,
            default_burst: 5,
            global_rpm: 600,
            endpoint_limits: HashMap::new(),
        })
    }

    #[test]
    fn test_allows_within_burst() {
        let mut limiter = test_limiter();
        for _ in 0..5 {
            let result = limiter.check("client-1", None);
            assert!(result.is_allowed());
        }
    }

    #[test]
    fn test_limits_after_burst() {
        let mut limiter = test_limiter();
        // Exhaust burst of 5
        for _ in 0..5 {
            limiter.check("client-1", None);
        }
        // 6th should be limited
        let result = limiter.check("client-1", None);
        assert!(result.is_limited());
    }

    #[test]
    fn test_refill_allows_more() {
        let mut limiter = test_limiter();
        // Exhaust burst
        for _ in 0..5 {
            limiter.check("client-1", None);
        }
        assert!(limiter.check("client-1", None).is_limited());

        // Manually advance time by backdating last_refill
        if let Some(bucket) = limiter.client_buckets.get_mut("client-1") {
            bucket.last_refill = Utc::now() - Duration::seconds(5);
            bucket.refill();
        }
        // Should have refilled ~5 tokens (60rpm = 1/sec * 5sec)
        let result = limiter.check("client-1", None);
        assert!(result.is_allowed());
    }

    #[test]
    fn test_independent_clients() {
        let mut limiter = test_limiter();
        // Exhaust client-1
        for _ in 0..5 {
            limiter.check("client-1", None);
        }
        assert!(limiter.check("client-1", None).is_limited());

        // client-2 should be fine
        assert!(limiter.check("client-2", None).is_allowed());
    }

    #[test]
    fn test_endpoint_specific_limits() {
        let mut ep_limits = HashMap::new();
        ep_limits.insert("/v1/chat".to_string(), EndpointLimit { rpm: 10, burst: 2 });

        let mut limiter = RateLimiter::new(RateLimitConfig {
            enabled: true,
            default_rpm: 60,
            default_burst: 10,
            global_rpm: 600,
            endpoint_limits: ep_limits,
        });

        // 2 requests to /v1/chat allowed (burst=2)
        assert!(limiter.check("client-1", Some("/v1/chat")).is_allowed());
        assert!(limiter.check("client-1", Some("/v1/chat")).is_allowed());

        // 3rd to /v1/chat limited
        assert!(limiter.check("client-1", Some("/v1/chat")).is_limited());

        // But general requests still allowed (default burst=10)
        assert!(limiter.check("client-1", None).is_allowed());
    }

    #[test]
    fn test_global_limit() {
        let mut limiter = RateLimiter::new(RateLimitConfig {
            enabled: true,
            default_rpm: 600,
            default_burst: 200,
            global_rpm: 6, // Very low: 1 per 10 seconds, burst=1
            endpoint_limits: HashMap::new(),
        });

        // First request uses the global token
        let result = limiter.check("client-1", None);
        assert!(result.is_allowed());

        // Global bucket burst is global_rpm/6 = 1, so second should be limited
        let result = limiter.check("client-2", None);
        assert!(result.is_limited());
    }

    #[test]
    fn test_disabled_limiter() {
        let mut limiter = RateLimiter::new(RateLimitConfig {
            enabled: false,
            ..Default::default()
        });
        // Should always allow when disabled
        for _ in 0..100 {
            assert!(limiter.check("client-1", None).is_allowed());
        }
    }

    #[test]
    fn test_reset_client() {
        let mut limiter = test_limiter();
        for _ in 0..5 {
            limiter.check("client-1", None);
        }
        assert!(limiter.check("client-1", None).is_limited());

        limiter.reset_client("client-1");
        assert!(limiter.check("client-1", None).is_allowed());
    }

    #[test]
    fn test_reset_all() {
        let mut limiter = test_limiter();
        for _ in 0..5 {
            limiter.check("client-1", None);
            limiter.check("client-2", None);
        }
        limiter.reset_all();
        assert!(limiter.check("client-1", None).is_allowed());
        assert!(limiter.check("client-2", None).is_allowed());
    }

    #[test]
    fn test_client_report() {
        let mut limiter = test_limiter();
        limiter.check("client-1", None);
        limiter.check("client-1", None);
        limiter.check("client-2", None);

        let report = limiter.client_report();
        assert_eq!(report.len(), 2);

        let c1 = report.iter().find(|r| r.client_id == "client-1").unwrap();
        assert_eq!(c1.total_allowed, 2);
    }

    #[test]
    fn test_client_count() {
        let mut limiter = test_limiter();
        assert_eq!(limiter.client_count(), 0);
        limiter.check("client-1", None);
        assert_eq!(limiter.client_count(), 1);
        limiter.check("client-2", None);
        assert_eq!(limiter.client_count(), 2);
    }

    #[test]
    fn test_stats_tracking() {
        let mut limiter = test_limiter();
        limiter.check("client-1", None);
        limiter.check("client-1", None);
        assert_eq!(limiter.stats().total_checks, 2);
        assert_eq!(limiter.stats().total_allowed, 2);
    }

    #[test]
    fn test_stats_limited_count() {
        let mut limiter = test_limiter();
        for _ in 0..6 {
            limiter.check("client-1", None);
        }
        assert_eq!(limiter.stats().total_limited, 1);
    }

    #[test]
    fn test_reset_stats() {
        let mut limiter = test_limiter();
        limiter.check("client-1", None);
        limiter.reset_stats();
        assert_eq!(limiter.stats().total_checks, 0);
    }

    #[test]
    fn test_result_helpers() {
        assert!(RateLimitResult::Allowed { remaining: 5 }.is_allowed());
        assert!(!RateLimitResult::Allowed { remaining: 5 }.is_limited());
        assert!(
            RateLimitResult::Limited {
                retry_after_secs: 1.0
            }
            .is_limited()
        );
        assert!(
            !RateLimitResult::Limited {
                retry_after_secs: 1.0
            }
            .is_allowed()
        );
    }

    #[test]
    fn test_wait_time() {
        let mut bucket = TokenBucket::new(1, 60);
        bucket.try_consume(); // use the one token
        let wait = bucket.wait_time_secs();
        assert!(wait > 0.0);
        assert!(wait <= 1.0); // 60rpm = 1/sec, so max 1s wait
    }

    #[test]
    fn test_config_update() {
        let mut limiter = test_limiter();
        limiter.set_config(RateLimitConfig {
            default_rpm: 120,
            ..Default::default()
        });
        assert_eq!(limiter.config().default_rpm, 120);
    }

    #[test]
    fn test_default_limiter() {
        let limiter = RateLimiter::default();
        assert!(limiter.config().enabled);
        assert_eq!(limiter.config().default_rpm, 60);
        assert_eq!(limiter.config().global_rpm, 1000);
    }

    #[test]
    fn test_remaining_decreases() {
        let mut limiter = test_limiter();
        if let RateLimitResult::Allowed { remaining: r1 } = limiter.check("client-1", None) {
            if let RateLimitResult::Allowed { remaining: r2 } = limiter.check("client-1", None) {
                assert!(r2 < r1, "Remaining should decrease: {} vs {}", r2, r1);
            }
        }
    }
}
