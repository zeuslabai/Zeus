//! HTTP rate limiting middleware for the REST API.
//!
//! Per-IP token bucket rate limiter with two tiers:
//! - **Global**: generous limit for all endpoints (default 600 req/min)
//! - **LLM**: stricter limit for expensive LLM-invoking endpoints (default 300 req/min)
//!
//! Health endpoints (`/` and `/health`) are exempt from rate limiting.

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Rate limit configuration.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum requests per minute per IP for all endpoints.
    pub global_rpm: u32,
    /// Maximum requests per minute per IP for LLM-invoking endpoints.
    pub llm_rpm: u32,
    /// Extra burst tokens above sustained rate.
    pub burst_size: u32,
    /// How often (in seconds) to prune stale IP entries.
    pub cleanup_interval_secs: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            global_rpm: 600,
            llm_rpm: 300,
            burst_size: 100,
            cleanup_interval_secs: 300,
        }
    }
}

/// Per-IP token bucket state.
struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(capacity: f64) -> Self {
        Self {
            tokens: capacity,
            last_refill: Instant::now(),
        }
    }
}

/// Per-IP token bucket rate limiter for HTTP requests.
///
/// Maintains separate global and LLM buckets per IP address.
/// Thread-safe via `std::sync::Mutex` (non-async, held briefly).
pub struct HttpRateLimiter {
    config: RateLimitConfig,
    global_buckets: Mutex<HashMap<IpAddr, TokenBucket>>,
    llm_buckets: Mutex<HashMap<IpAddr, TokenBucket>>,
}

impl HttpRateLimiter {
    /// Create a new rate limiter wrapped in `Arc` for shared ownership.
    pub fn new(config: RateLimitConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            global_buckets: Mutex::new(HashMap::new()),
            llm_buckets: Mutex::new(HashMap::new()),
        })
    }

    /// Try to acquire a token for the given IP.
    ///
    /// Checks the global bucket and, if `is_llm_endpoint` is true, also the LLM bucket.
    /// Returns `Ok(())` on success or `Err(retry_after_secs)` if rate limited.
    pub fn try_acquire(&self, ip: IpAddr, is_llm_endpoint: bool) -> Result<(), u32> {
        // Check global bucket
        {
            let mut buckets = self
                .global_buckets
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let capacity =
                self.config.global_rpm as f64 / 60.0 * 60.0 + self.config.burst_size as f64;
            let refill_rate = self.config.global_rpm as f64 / 60.0; // tokens per second
            let bucket = buckets
                .entry(ip)
                .or_insert_with(|| TokenBucket::new(capacity));

            refill(bucket, refill_rate, capacity);

            if bucket.tokens < 1.0 {
                let wait = ((1.0 - bucket.tokens) / refill_rate).ceil() as u32;
                return Err(wait.max(1));
            }
            bucket.tokens -= 1.0;
        }

        // Check LLM bucket if applicable
        if is_llm_endpoint {
            let mut buckets = self
                .llm_buckets
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let capacity = self.config.llm_rpm as f64 / 60.0 * 60.0 + self.config.burst_size as f64;
            let refill_rate = self.config.llm_rpm as f64 / 60.0;
            let bucket = buckets
                .entry(ip)
                .or_insert_with(|| TokenBucket::new(capacity));

            refill(bucket, refill_rate, capacity);

            if bucket.tokens < 1.0 {
                // Refund the global token we already consumed
                let mut global = self
                    .global_buckets
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(gb) = global.get_mut(&ip) {
                    let global_cap =
                        self.config.global_rpm as f64 / 60.0 * 60.0 + self.config.burst_size as f64;
                    gb.tokens = (gb.tokens + 1.0).min(global_cap);
                }

                let wait = ((1.0 - bucket.tokens) / refill_rate).ceil() as u32;
                return Err(wait.max(1));
            }
            bucket.tokens -= 1.0;
        }

        Ok(())
    }

    /// Remove IP entries that haven't been seen in `cleanup_interval_secs`.
    pub fn cleanup(&self) {
        let cutoff = self.config.cleanup_interval_secs;
        let now = Instant::now();

        {
            let mut buckets = self
                .global_buckets
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            buckets.retain(|_, b| now.duration_since(b.last_refill).as_secs() < cutoff);
        }
        {
            let mut buckets = self
                .llm_buckets
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            buckets.retain(|_, b| now.duration_since(b.last_refill).as_secs() < cutoff);
        }
    }

    /// Spawn a background task that periodically cleans up stale entries.
    pub fn start_cleanup_task(self: &Arc<Self>) {
        let limiter = Arc::clone(self);
        let interval_secs = limiter.config.cleanup_interval_secs;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;
                limiter.cleanup();
            }
        });
    }
}

/// Refill tokens based on elapsed time since last refill.
fn refill(bucket: &mut TokenBucket, rate_per_sec: f64, capacity: f64) {
    let elapsed = bucket.last_refill.elapsed().as_secs_f64();
    if elapsed > 0.0 {
        let new_tokens = elapsed * rate_per_sec;
        bucket.tokens = (bucket.tokens + new_tokens).min(capacity);
        bucket.last_refill = Instant::now();
    }
}

/// Check if a request path targets an LLM-invoking endpoint.
fn is_llm_endpoint(path: &str) -> bool {
    path == "/v1/chat"
        || path == "/v1/chat/completions"
        || path.starts_with("/v1/tools/")
        || path == "/v1/sandbox/execute"
        || path == "/v1/tts/synthesize"
        || path == "/v1/tts/synthesize/stream"
        || path == "/v1/images/generate"
        || path == "/v1/prometheus/execute"
}

/// Check if a request path is exempt from rate limiting.
fn is_exempt(path: &str) -> bool {
    path == "/" || path == "/health"
}

/// Normalize an IP address for consistent rate-limit bucketing.
///
/// IPv4-mapped IPv6 addresses (`::ffff:a.b.c.d`) are converted to their
/// canonical IPv4 form so that a client alternating between IPv4 and
/// IPv4-in-IPv6 representations cannot obtain separate token buckets.
pub fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                IpAddr::V4(v4)
            } else {
                IpAddr::V6(v6)
            }
        }
        IpAddr::V4(_) => ip,
    }
}

/// Tower layer that injects the `HttpRateLimiter` into request extensions.
#[derive(Clone)]
pub struct RateLimitLayer {
    limiter: Arc<HttpRateLimiter>,
}

impl RateLimitLayer {
    pub fn new(limiter: Arc<HttpRateLimiter>) -> Self {
        Self { limiter }
    }
}

impl<S> tower::Layer<S> for RateLimitLayer {
    type Service = RateLimitService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitService {
            inner,
            limiter: Arc::clone(&self.limiter),
        }
    }
}

/// Tower service that injects the rate limiter and delegates to the inner service.
#[derive(Clone)]
pub struct RateLimitService<S> {
    inner: S,
    limiter: Arc<HttpRateLimiter>,
}

impl<S> tower::Service<Request<Body>> for RateLimitService<S>
where
    S: tower::Service<Request<Body>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let limiter = Arc::clone(&self.limiter);
        let mut inner = self.inner.clone();
        // Swap so we use the ready clone
        std::mem::swap(&mut self.inner, &mut inner);

        Box::pin(async move {
            let path = req.uri().path().to_string();

            // Exempt health endpoints
            if is_exempt(&path) {
                return inner.call(req).await;
            }

            // Extract IP from ConnectInfo extension if available, then
            // normalize so IPv4-mapped IPv6 and plain IPv4 share one bucket.
            let connect_ip = req
                .extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ci| normalize_ip(ci.0.ip()));
            // Local TUI/WebUI/CLI/agent hit their own gateway on loopback — don't self-throttle.
            if connect_ip.map(|ip| ip.is_loopback()).unwrap_or(false) {
                req.extensions_mut().insert(limiter);
                return inner.call(req).await;
            }
            let ip = connect_ip.unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));

            let is_llm = is_llm_endpoint(&path);

            match limiter.try_acquire(ip, is_llm) {
                Ok(()) => {
                    req.extensions_mut().insert(limiter);
                    inner.call(req).await
                }
                Err(retry_after) => {
                    let body = serde_json::json!({
                        "error": {
                            "message": "Rate limit exceeded",
                            "type": "rate_limit_error",
                            "retry_after": retry_after
                        }
                    });
                    Ok((
                        StatusCode::TOO_MANY_REQUESTS,
                        [
                            (axum::http::header::RETRY_AFTER, retry_after.to_string()),
                            (
                                axum::http::header::CONTENT_TYPE,
                                "application/json".to_string(),
                            ),
                        ],
                        body.to_string(),
                    )
                        .into_response())
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_rate_limiter_allows_under_limit() {
        let config = RateLimitConfig {
            global_rpm: 60,
            llm_rpm: 10,
            burst_size: 5,
            cleanup_interval_secs: 300,
        };
        let limiter = HttpRateLimiter::new(config);
        let ip: IpAddr = "10.0.0.1".parse().unwrap();

        // Should allow several requests (capacity = 60/60*60 + 5 = 65)
        for _ in 0..10 {
            assert!(limiter.try_acquire(ip, false).is_ok());
        }
    }

    #[test]
    fn test_rate_limiter_blocks_over_limit() {
        let config = RateLimitConfig {
            global_rpm: 60,
            llm_rpm: 10,
            burst_size: 0,
            cleanup_interval_secs: 300,
        };
        let limiter = HttpRateLimiter::new(config);
        let ip: IpAddr = "10.0.0.1".parse().unwrap();

        // Capacity = 60/60*60 + 0 = 60 tokens
        // Drain all tokens
        for _ in 0..60 {
            assert!(limiter.try_acquire(ip, false).is_ok());
        }

        // Should be blocked now
        let result = limiter.try_acquire(ip, false);
        assert!(result.is_err());
        assert!(result.unwrap_err() >= 1);
    }

    #[test]
    fn test_rate_limiter_refills_over_time() {
        let config = RateLimitConfig {
            global_rpm: 60,
            llm_rpm: 10,
            burst_size: 0,
            cleanup_interval_secs: 300,
        };
        let limiter = HttpRateLimiter::new(config);
        let ip: IpAddr = "10.0.0.1".parse().unwrap();

        // Drain all tokens
        for _ in 0..60 {
            limiter.try_acquire(ip, false).ok();
        }
        assert!(limiter.try_acquire(ip, false).is_err());

        // Manually backdate the last_refill to simulate time passing
        {
            let mut buckets = limiter
                .global_buckets
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(bucket) = buckets.get_mut(&ip) {
                bucket.last_refill = Instant::now() - Duration::from_secs(5);
            }
        }

        // Should succeed now (5 seconds * 1 token/sec = 5 tokens refilled)
        assert!(limiter.try_acquire(ip, false).is_ok());
    }

    #[test]
    fn test_llm_endpoint_stricter_limit() {
        let config = RateLimitConfig {
            global_rpm: 600,
            llm_rpm: 6,
            burst_size: 0,
            cleanup_interval_secs: 300,
        };
        let limiter = HttpRateLimiter::new(config);
        let ip: IpAddr = "10.0.0.1".parse().unwrap();

        // LLM capacity = 6/60*60 + 0 = 6 tokens
        for _ in 0..6 {
            assert!(limiter.try_acquire(ip, true).is_ok());
        }

        // LLM bucket exhausted, should be blocked
        assert!(limiter.try_acquire(ip, true).is_err());

        // Global endpoint should still work (600 - 6 = 594 tokens left)
        assert!(limiter.try_acquire(ip, false).is_ok());
    }

    #[test]
    fn test_different_ips_independent() {
        let config = RateLimitConfig {
            global_rpm: 60,
            llm_rpm: 10,
            burst_size: 0,
            cleanup_interval_secs: 300,
        };
        let limiter = HttpRateLimiter::new(config);
        let ip_a: IpAddr = "10.0.0.1".parse().unwrap();
        let ip_b: IpAddr = "10.0.0.2".parse().unwrap();

        // Exhaust IP A
        for _ in 0..60 {
            limiter.try_acquire(ip_a, false).ok();
        }
        assert!(limiter.try_acquire(ip_a, false).is_err());

        // IP B should still work
        assert!(limiter.try_acquire(ip_b, false).is_ok());
    }

    #[test]
    fn test_health_endpoints_exempt() {
        assert!(is_exempt("/"));
        assert!(is_exempt("/health"));
        assert!(!is_exempt("/v1/chat"));
        assert!(!is_exempt("/v1/tools/shell"));
    }

    #[test]
    fn test_llm_endpoint_detection() {
        assert!(is_llm_endpoint("/v1/chat"));
        assert!(is_llm_endpoint("/v1/chat/completions"));
        assert!(is_llm_endpoint("/v1/tools/shell"));
        assert!(is_llm_endpoint("/v1/tools/web_fetch"));
        assert!(is_llm_endpoint("/v1/sandbox/execute"));
        assert!(is_llm_endpoint("/v1/tts/synthesize"));
        assert!(is_llm_endpoint("/v1/tts/synthesize/stream"));
        assert!(is_llm_endpoint("/v1/images/generate"));
        assert!(is_llm_endpoint("/v1/prometheus/execute"));

        assert!(!is_llm_endpoint("/v1/sessions"));
        assert!(!is_llm_endpoint("/v1/memory"));
        assert!(!is_llm_endpoint("/v1/tools"));
        assert!(!is_llm_endpoint("/v1/config"));
        assert!(!is_llm_endpoint("/"));
        assert!(!is_llm_endpoint("/health"));
    }

    #[test]
    fn test_cleanup_removes_stale() {
        let config = RateLimitConfig {
            global_rpm: 60,
            llm_rpm: 10,
            burst_size: 5,
            cleanup_interval_secs: 10, // 10 seconds for test
        };
        let limiter = HttpRateLimiter::new(config);
        let ip: IpAddr = "10.0.0.1".parse().unwrap();

        // Create an entry
        limiter.try_acquire(ip, false).ok();

        // Backdate the entry to be stale
        {
            let mut buckets = limiter
                .global_buckets
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(bucket) = buckets.get_mut(&ip) {
                bucket.last_refill = Instant::now() - Duration::from_secs(15);
            }
        }

        // Cleanup should remove it
        limiter.cleanup();

        // Verify the entry was removed (next acquire creates fresh)
        let count = limiter
            .global_buckets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_default_config_values() {
        let config = RateLimitConfig::default();
        assert_eq!(config.global_rpm, 600);
        assert_eq!(config.llm_rpm, 300);
        assert_eq!(config.burst_size, 100);
        assert_eq!(config.cleanup_interval_secs, 300);
    }

    #[test]
    fn test_normalize_ip_v4_unchanged() {
        let ip: IpAddr = "1.2.3.4".parse().unwrap();
        assert_eq!(normalize_ip(ip), ip);
    }

    #[test]
    fn test_normalize_ip_v4_mapped_v6_becomes_v4() {
        // ::ffff:1.2.3.4 is the IPv4-mapped form of 1.2.3.4
        let v6: IpAddr = "::ffff:1.2.3.4".parse().unwrap();
        let expected: IpAddr = "1.2.3.4".parse().unwrap();
        assert_eq!(normalize_ip(v6), expected);
    }

    #[test]
    fn test_normalize_ip_pure_v6_unchanged() {
        let v6: IpAddr = "2001:db8::1".parse().unwrap();
        assert_eq!(normalize_ip(v6), v6);
    }

    #[test]
    fn test_normalize_ip_v4_mapped_shares_bucket_with_v4() {
        // Client sending from ::ffff:10.0.0.1 and 10.0.0.1 must hit the same bucket.
        let config = RateLimitConfig {
            global_rpm: 6,
            llm_rpm: 6,
            burst_size: 0,
            cleanup_interval_secs: 300,
        };
        let limiter = HttpRateLimiter::new(config);

        let v4: IpAddr = "10.0.0.1".parse().unwrap();
        let v4_in_v6: IpAddr = "::ffff:10.0.0.1".parse().unwrap();

        // Drain using plain IPv4
        for _ in 0..6 {
            limiter.try_acquire(normalize_ip(v4), false).ok();
        }

        // The IPv4-mapped IPv6 form must now also be blocked (same bucket after normalization)
        assert!(
            limiter.try_acquire(normalize_ip(v4_in_v6), false).is_err(),
            "IPv4-mapped IPv6 must share the rate-limit bucket with IPv4"
        );
    }

    #[test]
    fn test_retry_after_is_positive() {
        let config = RateLimitConfig {
            global_rpm: 6,
            llm_rpm: 6,
            burst_size: 0,
            cleanup_interval_secs: 300,
        };
        let limiter = HttpRateLimiter::new(config);
        let ip: IpAddr = "10.0.0.1".parse().unwrap();

        // Exhaust all tokens
        for _ in 0..6 {
            limiter.try_acquire(ip, false).ok();
        }

        match limiter.try_acquire(ip, false) {
            Err(retry_after) => assert!(retry_after >= 1),
            Ok(()) => panic!("Should have been rate limited"),
        }
    }

    #[test]
    fn test_llm_refunds_global_on_llm_deny() {
        let config = RateLimitConfig {
            global_rpm: 600,
            llm_rpm: 6,
            burst_size: 0,
            cleanup_interval_secs: 300,
        };
        let limiter = HttpRateLimiter::new(config);
        let ip: IpAddr = "10.0.0.1".parse().unwrap();

        // Exhaust LLM bucket
        for _ in 0..6 {
            limiter.try_acquire(ip, true).ok();
        }

        // Check global tokens before denied LLM request
        let global_before = {
            let buckets = limiter
                .global_buckets
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            buckets.get(&ip).map(|b| b.tokens).unwrap_or(0.0)
        };

        // This LLM request should be denied and global should be refunded
        assert!(limiter.try_acquire(ip, true).is_err());

        let global_after = {
            let buckets = limiter
                .global_buckets
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            buckets.get(&ip).map(|b| b.tokens).unwrap_or(0.0)
        };

        // Global tokens should be the same or higher (refunded)
        assert!(global_after >= global_before);
    }

    #[cfg(test)]
    mod integration_tests {
        use super::*;
        use axum::{Router, body::Body, http::Request};
        use tower::ServiceExt;

        async fn test_router() -> Router {
            Router::new()
                .route("/", axum::routing::get(|| async { "health" }))
                .route("/api/test", axum::routing::get(|| async { "test" }))
                .route("/v1/chat", axum::routing::post(|| async { "llm endpoint" }))
        }

        #[tokio::test]
        async fn test_rate_limit_allows_under_limit() {
            let config = RateLimitConfig {
                global_rpm: 60,
                llm_rpm: 10,
                burst_size: 5,
                cleanup_interval_secs: 300,
            };
            let limiter = HttpRateLimiter::new(config);
            let layer = RateLimitLayer::new(limiter);
            let app = test_router().await.layer(layer);

            // Should allow several requests under the limit
            for _ in 0..5 {
                let req = Request::builder()
                    .uri("/api/test")
                    .body(Body::empty())
                    .unwrap();
                let response = app.clone().oneshot(req).await.unwrap();
                assert_eq!(response.status(), 200);
            }
        }

        #[tokio::test]
        async fn test_rate_limit_blocks_over_limit() {
            let config = RateLimitConfig {
                global_rpm: 6, // Very low limit for testing
                llm_rpm: 3,
                burst_size: 0,
                cleanup_interval_secs: 300,
            };
            let limiter = HttpRateLimiter::new(config);
            let layer = RateLimitLayer::new(limiter);
            let app = test_router().await.layer(layer);

            // Exhaust the limit (6 tokens)
            for _ in 0..6 {
                let req = Request::builder()
                    .uri("/api/test")
                    .body(Body::empty())
                    .unwrap();
                let response = app.clone().oneshot(req).await.unwrap();
                assert_eq!(response.status(), 200);
            }

            // Next request should be rate limited
            let req = Request::builder()
                .uri("/api/test")
                .body(Body::empty())
                .unwrap();
            let response = app.clone().oneshot(req).await.unwrap();
            assert_eq!(response.status(), 429); // TOO_MANY_REQUESTS

            // Check Retry-After header is present
            let retry_after = response.headers().get("retry-after");
            assert!(retry_after.is_some());
        }

        #[tokio::test]
        async fn test_health_endpoints_exempt() {
            let config = RateLimitConfig {
                global_rpm: 0, // Zero limit to ensure everything is blocked except exempt
                llm_rpm: 0,
                burst_size: 0,
                cleanup_interval_secs: 300,
            };
            let limiter = HttpRateLimiter::new(config);
            let layer = RateLimitLayer::new(limiter);
            let app = test_router().await.layer(layer);

            // Health endpoint should work even with zero rate limit
            let req = Request::builder().uri("/").body(Body::empty()).unwrap();
            let response = app.clone().oneshot(req).await.unwrap();
            assert_eq!(response.status(), 200);
        }

        #[tokio::test]
        async fn test_llm_endpoint_stricter_limit() {
            let config = RateLimitConfig {
                global_rpm: 600, // Generous global limit
                llm_rpm: 3,      // Strict LLM limit
                burst_size: 0,
                cleanup_interval_secs: 300,
            };
            let limiter = HttpRateLimiter::new(config);
            let layer = RateLimitLayer::new(limiter);
            let app = test_router().await.layer(layer);

            // Should allow 3 LLM requests
            for _ in 0..3 {
                let req = Request::builder()
                    .uri("/v1/chat")
                    .method("POST")
                    .body(Body::empty())
                    .unwrap();
                let response = app.clone().oneshot(req).await.unwrap();
                assert_eq!(response.status(), 200);
            }

            // 4th LLM request should be blocked
            let req = Request::builder()
                .uri("/v1/chat")
                .method("POST")
                .body(Body::empty())
                .unwrap();
            let response = app.clone().oneshot(req).await.unwrap();
            assert_eq!(response.status(), 429);

            // But regular endpoints should still work
            let req = Request::builder()
                .uri("/api/test")
                .body(Body::empty())
                .unwrap();
            let response = app.clone().oneshot(req).await.unwrap();
            assert_eq!(response.status(), 200);
        }

        #[tokio::test]
        async fn test_loopback_connect_info_exempt_from_rate_limit() {
            let config = RateLimitConfig {
                global_rpm: 1,
                llm_rpm: 1,
                burst_size: 0,
                cleanup_interval_secs: 300,
            };
            let limiter = HttpRateLimiter::new(config);
            let layer = RateLimitLayer::new(limiter);
            let app = test_router().await.layer(layer);

            // Real loopback ConnectInfo represents local TUI/WebUI/CLI traffic and
            // should not self-throttle even beyond the configured limit.
            for _ in 0..5 {
                let mut req = Request::builder()
                    .uri("/api/test")
                    .body(Body::empty())
                    .unwrap();
                req.extensions_mut()
                    .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4242))));
                let response = app.clone().oneshot(req).await.unwrap();
                assert_eq!(response.status(), 200);
            }
        }

        #[tokio::test]
        async fn test_rate_limit_error_response_format() {
            let config = RateLimitConfig {
                global_rpm: 6,
                llm_rpm: 3,
                burst_size: 0,
                cleanup_interval_secs: 300,
            };
            let limiter = HttpRateLimiter::new(config);
            let layer = RateLimitLayer::new(limiter);
            let app = test_router().await.layer(layer);

            // Exhaust limit
            for _ in 0..6 {
                let req = Request::builder()
                    .uri("/api/test")
                    .body(Body::empty())
                    .unwrap();
                app.clone().oneshot(req).await.unwrap();
            }

            // Get rate limited response
            let req = Request::builder()
                .uri("/api/test")
                .body(Body::empty())
                .unwrap();
            let response = app.clone().oneshot(req).await.unwrap();
            assert_eq!(response.status(), 429);

            // Check headers
            assert!(response.headers().get("retry-after").is_some());
            assert_eq!(
                response.headers().get("content-type").unwrap(),
                "application/json"
            );

            // Check JSON body structure
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert!(json["error"]["message"].is_string());
            assert_eq!(json["error"]["type"], "rate_limit_error");
            assert!(json["error"]["retry_after"].is_number());
        }
    }
}
