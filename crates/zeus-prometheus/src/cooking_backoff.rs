//! Backoff Strategy
//!
//! Exponential backoff with jitter for retry logic.

use rand::Rng;
use std::time::Duration;

/// Configuration for backoff behavior
#[derive(Debug, Clone)]
pub struct BackoffConfig {
    /// Initial delay in milliseconds
    pub initial_delay_ms: u64,
    /// Maximum delay in milliseconds
    pub max_delay_ms: u64,
    /// Multiplier for each retry
    pub multiplier: f64,
    /// Maximum number of retries
    pub max_retries: u32,
    /// Add random jitter (0.0 to 1.0)
    pub jitter: f64,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 1000, // 1 second
            max_delay_ms: 60_000,   // 1 minute max
            multiplier: 2.0,        // Double each time
            max_retries: 5,         // 5 attempts total
            jitter: 0.2,            // 20% jitter
        }
    }
}

/// Backoff strategy for calculating delays
pub struct BackoffStrategy {
    config: BackoffConfig,
    current_attempt: u32,
}

impl BackoffStrategy {
    /// Create a new backoff strategy
    pub fn new(config: BackoffConfig) -> Self {
        Self {
            config,
            current_attempt: 0,
        }
    }

    /// Create with default configuration
    pub fn default_strategy() -> Self {
        Self::new(BackoffConfig::default())
    }

    /// Calculate delay for the next retry
    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.current_attempt >= self.config.max_retries {
            return None;
        }

        let delay = self.calculate_delay(self.current_attempt);
        self.current_attempt += 1;

        Some(delay)
    }

    /// Peek at the next delay without incrementing
    pub fn peek_delay(&self) -> Option<Duration> {
        if self.current_attempt >= self.config.max_retries {
            return None;
        }

        Some(self.calculate_delay(self.current_attempt))
    }

    /// Calculate delay for a specific attempt number
    fn calculate_delay(&self, attempt: u32) -> Duration {
        let base = self.config.initial_delay_ms as f64;
        let multiplier = self.config.multiplier.powi(attempt as i32);
        let delay_ms = (base * multiplier) as u64;

        // Apply max cap
        let capped = delay_ms.min(self.config.max_delay_ms);

        // Apply jitter
        let jittered = if self.config.jitter > 0.0 {
            let mut rng = rand::thread_rng();
            let jitter_range = capped as f64 * self.config.jitter;
            let jitter = (rng.r#gen::<f64>() * 2.0 - 1.0) * jitter_range;
            ((capped as f64) + jitter) as u64
        } else {
            capped
        };

        Duration::from_millis(jittered.max(1))
    }

    /// Reset the backoff state
    pub fn reset(&mut self) {
        self.current_attempt = 0;
    }

    /// Get current attempt number
    pub fn current_attempt(&self) -> u32 {
        self.current_attempt
    }

    /// Check if more retries are allowed
    pub fn can_retry(&self) -> bool {
        self.current_attempt < self.config.max_retries
    }

    /// Get remaining retries
    pub fn remaining_retries(&self) -> u32 {
        self.config.max_retries.saturating_sub(self.current_attempt)
    }

    /// Get the configuration
    pub fn config(&self) -> &BackoffConfig {
        &self.config
    }
}

/// Calculate delay with override for rate limit retry-after
pub fn delay_with_override(backoff: &BackoffStrategy, override_secs: Option<u64>) -> Duration {
    if let Some(secs) = override_secs {
        Duration::from_secs(secs)
    } else {
        backoff.peek_delay().unwrap_or(Duration::from_secs(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = BackoffConfig::default();
        assert_eq!(config.initial_delay_ms, 1000);
        assert_eq!(config.max_retries, 5);
    }

    #[test]
    fn test_exponential_growth() {
        let config = BackoffConfig {
            initial_delay_ms: 100,
            max_delay_ms: 100_000,
            multiplier: 2.0,
            max_retries: 10,
            jitter: 0.0, // No jitter for predictable testing
        };

        let mut backoff = BackoffStrategy::new(config);

        // First delay should be ~100ms
        let d1 = backoff.next_delay().unwrap();
        assert!(d1.as_millis() >= 100 && d1.as_millis() <= 100);

        // Second delay should be ~200ms
        let d2 = backoff.next_delay().unwrap();
        assert!(d2.as_millis() >= 200 && d2.as_millis() <= 200);

        // Third delay should be ~400ms
        let d3 = backoff.next_delay().unwrap();
        assert!(d3.as_millis() >= 400 && d3.as_millis() <= 400);
    }

    #[test]
    fn test_max_delay_cap() {
        let config = BackoffConfig {
            initial_delay_ms: 10_000,
            max_delay_ms: 20_000,
            multiplier: 10.0,
            max_retries: 5,
            jitter: 0.0,
        };

        let mut backoff = BackoffStrategy::new(config);

        // First: 10000
        let d1 = backoff.next_delay().unwrap();
        assert_eq!(d1.as_millis(), 10_000);

        // Second: 100000 -> capped at 20000
        let d2 = backoff.next_delay().unwrap();
        assert_eq!(d2.as_millis(), 20_000);
    }

    #[test]
    fn test_max_retries() {
        let config = BackoffConfig {
            max_retries: 3,
            ..Default::default()
        };

        let mut backoff = BackoffStrategy::new(config);

        assert!(backoff.next_delay().is_some()); // 1
        assert!(backoff.next_delay().is_some()); // 2
        assert!(backoff.next_delay().is_some()); // 3
        assert!(backoff.next_delay().is_none()); // No more retries
    }

    #[test]
    fn test_reset() {
        let config = BackoffConfig {
            max_retries: 2,
            ..Default::default()
        };

        let mut backoff = BackoffStrategy::new(config);

        backoff.next_delay();
        backoff.next_delay();
        assert!(backoff.next_delay().is_none());

        backoff.reset();
        assert!(backoff.next_delay().is_some());
    }

    #[test]
    fn test_can_retry() {
        let config = BackoffConfig {
            max_retries: 2,
            ..Default::default()
        };

        let mut backoff = BackoffStrategy::new(config);

        assert!(backoff.can_retry());
        assert_eq!(backoff.remaining_retries(), 2);

        backoff.next_delay();
        assert!(backoff.can_retry());
        assert_eq!(backoff.remaining_retries(), 1);

        backoff.next_delay();
        assert!(!backoff.can_retry());
        assert_eq!(backoff.remaining_retries(), 0);
    }

    #[test]
    fn test_delay_with_override() {
        let config = BackoffConfig::default();
        let backoff = BackoffStrategy::new(config);

        // Without override, uses backoff
        let d1 = delay_with_override(&backoff, None);
        assert!(d1.as_millis() > 0);

        // With override, uses the override
        let d2 = delay_with_override(&backoff, Some(30));
        assert_eq!(d2.as_secs(), 30);
    }
}
