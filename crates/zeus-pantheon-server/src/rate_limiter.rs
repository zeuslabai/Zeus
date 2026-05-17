//! Per-connection token-bucket rate limiter.
//!
//! Burst: 10 messages, sustained: 2 msg/sec.
//! Uses a simple token-bucket algorithm — no external crates required.

use std::time::Instant;

/// Default burst capacity (tokens).
const DEFAULT_BURST: u32 = 10;
/// Default refill rate: tokens per second.
const DEFAULT_RATE_PER_SEC: f64 = 2.0;

#[derive(Debug, Clone)]
pub struct RateLimiter {
    /// Max tokens (burst ceiling).
    capacity: u32,
    /// Tokens added per second.
    rate_per_sec: f64,
    /// Current token count (fractional for smooth refill).
    tokens: f64,
    /// Last time tokens were refilled.
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            capacity: DEFAULT_BURST,
            rate_per_sec: DEFAULT_RATE_PER_SEC,
            tokens: DEFAULT_BURST as f64,
            last_refill: Instant::now(),
        }
    }

    pub fn with_rate(burst: u32, rate_per_sec: f64) -> Self {
        Self {
            capacity: burst,
            rate_per_sec,
            tokens: burst as f64,
            last_refill: Instant::now(),
        }
    }

    /// Try to consume one token. Returns `true` if allowed, `false` if rate-limited.
    pub fn check(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate_per_sec).min(self.capacity as f64);
        self.last_refill = now;
    }

    /// Seconds until next token is available (0.0 if already available).
    pub fn retry_after_secs(&mut self) -> f64 {
        self.refill();
        if self.tokens >= 1.0 {
            0.0
        } else {
            if self.rate_per_sec > 0.0 { (1.0 - self.tokens) / self.rate_per_sec } else { 1.0 }
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn burst_allows_initial_messages() {
        let mut rl = RateLimiter::new();
        for _ in 0..10 {
            assert!(rl.check(), "Should allow up to burst");
        }
        assert!(!rl.check(), "Should deny after burst exhausted");
    }

    #[test]
    fn refills_over_time() {
        let mut rl = RateLimiter::with_rate(10, 10.0); // fast refill for test
        for _ in 0..10 { rl.check(); } // drain
        // Simulate 0.5s passing
        rl.last_refill = Instant::now() - Duration::from_millis(500);
        assert!(rl.check(), "Should allow after refill");
    }
}
