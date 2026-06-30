//! Concurrency controls for Zeus agent operations.
//!
//! Provides semaphore-based concurrency limiting to control how many
//! agent executions can run simultaneously. Supports sequential (one at a time)
//! and parallel (up to N concurrent) modes.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Configuration for concurrency limiting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcurrencyConfig {
    /// Maximum concurrent agent executions.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    /// Queue mode.
    #[serde(default)]
    pub queue_mode: QueueMode,
}

fn default_max_concurrent() -> usize {
    1
}

impl Default for ConcurrencyConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_concurrent(),
            queue_mode: QueueMode::default(),
        }
    }
}

/// Controls how concurrent requests are handled.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueMode {
    /// Process one at a time (default for safety).
    #[default]
    Sequential,
    /// Allow up to N concurrent executions.
    Parallel,
}

/// Limits concurrent access to the agent using a semaphore.
///
/// In `Sequential` mode, only one execution is allowed at a time regardless
/// of the `max_concurrent` setting. In `Parallel` mode, up to `max_concurrent`
/// executions can proceed simultaneously.
pub struct ConcurrencyLimiter {
    semaphore: Arc<Semaphore>,
    max_concurrent: usize,
}

impl ConcurrencyLimiter {
    /// Create a new limiter from the given configuration.
    pub fn new(config: &ConcurrencyConfig) -> Self {
        let max = match config.queue_mode {
            QueueMode::Sequential => 1,
            QueueMode::Parallel => config.max_concurrent.max(1),
        };
        Self {
            semaphore: Arc::new(Semaphore::new(max)),
            max_concurrent: max,
        }
    }

    /// Acquire a permit, waiting until one is available.
    ///
    /// Returns an error if the semaphore has been closed.
    pub async fn acquire(&self) -> Result<tokio::sync::OwnedSemaphorePermit, String> {
        self.semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "concurrency semaphore closed".to_string())
    }

    /// Try to acquire a permit without blocking.
    /// Returns `None` if no permits are currently available.
    pub fn try_acquire(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.semaphore.clone().try_acquire_owned().ok()
    }

    /// Number of currently active (in-use) permits.
    pub fn active_count(&self) -> usize {
        self.max_concurrent - self.semaphore.available_permits()
    }

    /// Maximum allowed concurrent executions.
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// Number of permits currently available.
    pub fn available(&self) -> usize {
        self.semaphore.available_permits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sequential_mode_allows_one() {
        let config = ConcurrencyConfig {
            max_concurrent: 5, // ignored in sequential mode
            queue_mode: QueueMode::Sequential,
        };
        let limiter = ConcurrencyLimiter::new(&config);
        assert_eq!(limiter.max_concurrent(), 1);
        assert_eq!(limiter.available(), 1);

        // Acquire the single permit
        let _permit = limiter.acquire().await.unwrap();
        assert_eq!(limiter.available(), 0);

        // A second try_acquire should fail
        assert!(limiter.try_acquire().is_none());
    }

    #[tokio::test]
    async fn test_parallel_mode_allows_multiple() {
        let config = ConcurrencyConfig {
            max_concurrent: 3,
            queue_mode: QueueMode::Parallel,
        };
        let limiter = ConcurrencyLimiter::new(&config);
        assert_eq!(limiter.max_concurrent(), 3);
        assert_eq!(limiter.available(), 3);

        let _p1 = limiter.acquire().await.unwrap();
        let _p2 = limiter.acquire().await.unwrap();
        let _p3 = limiter.acquire().await.unwrap();
        assert_eq!(limiter.available(), 0);

        // Fourth should fail
        assert!(limiter.try_acquire().is_none());
    }

    #[tokio::test]
    async fn test_active_count_increases_with_acquisition() {
        let config = ConcurrencyConfig {
            max_concurrent: 3,
            queue_mode: QueueMode::Parallel,
        };
        let limiter = ConcurrencyLimiter::new(&config);
        assert_eq!(limiter.active_count(), 0);

        let _p1 = limiter.acquire().await.unwrap();
        assert_eq!(limiter.active_count(), 1);

        let _p2 = limiter.acquire().await.unwrap();
        assert_eq!(limiter.active_count(), 2);

        // Drop one permit, active count should decrease
        drop(_p1);
        assert_eq!(limiter.active_count(), 1);
    }

    #[tokio::test]
    async fn test_try_acquire_fails_when_full() {
        let config = ConcurrencyConfig {
            max_concurrent: 1,
            queue_mode: QueueMode::Parallel,
        };
        let limiter = ConcurrencyLimiter::new(&config);

        let _permit = limiter.try_acquire();
        assert!(_permit.is_some());

        // Now full
        assert!(limiter.try_acquire().is_none());
    }

    #[test]
    fn test_default_config_is_sequential() {
        let config = ConcurrencyConfig::default();
        assert_eq!(config.max_concurrent, 1);
        assert!(matches!(config.queue_mode, QueueMode::Sequential));

        let limiter = ConcurrencyLimiter::new(&config);
        assert_eq!(limiter.max_concurrent(), 1);
    }

    #[tokio::test]
    async fn test_permit_release_restores_availability() {
        let config = ConcurrencyConfig {
            max_concurrent: 2,
            queue_mode: QueueMode::Parallel,
        };
        let limiter = ConcurrencyLimiter::new(&config);

        let p1 = limiter.acquire().await.unwrap();
        let p2 = limiter.acquire().await.unwrap();
        assert_eq!(limiter.available(), 0);

        drop(p1);
        assert_eq!(limiter.available(), 1);

        drop(p2);
        assert_eq!(limiter.available(), 2);
    }

    #[test]
    fn test_parallel_mode_enforces_minimum_one() {
        let config = ConcurrencyConfig {
            max_concurrent: 0, // should be clamped to 1
            queue_mode: QueueMode::Parallel,
        };
        let limiter = ConcurrencyLimiter::new(&config);
        assert_eq!(limiter.max_concurrent(), 1);
    }

    #[test]
    fn test_config_serde_roundtrip() {
        let config = ConcurrencyConfig {
            max_concurrent: 4,
            queue_mode: QueueMode::Parallel,
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ConcurrencyConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.max_concurrent, 4);
        assert!(matches!(deserialized.queue_mode, QueueMode::Parallel));
    }

    // ================================================================
    // New tests
    // ================================================================

    #[test]
    fn test_config_parallel_with_max() {
        let config = ConcurrencyConfig {
            max_concurrent: 10,
            queue_mode: QueueMode::Parallel,
        };
        let limiter = ConcurrencyLimiter::new(&config);
        assert_eq!(limiter.max_concurrent(), 10);
        assert_eq!(limiter.available(), 10);
        assert_eq!(limiter.active_count(), 0);
    }

    #[test]
    fn test_config_serde_parallel() {
        let config = ConcurrencyConfig {
            max_concurrent: 8,
            queue_mode: QueueMode::Parallel,
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"parallel\""));
        assert!(json.contains("8"));

        let deserialized: ConcurrencyConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.max_concurrent, 8);
        assert!(matches!(deserialized.queue_mode, QueueMode::Parallel));

        // Also test sequential serialization
        let seq_config = ConcurrencyConfig {
            max_concurrent: 1,
            queue_mode: QueueMode::Sequential,
        };
        let seq_json = serde_json::to_string(&seq_config).unwrap();
        assert!(seq_json.contains("\"sequential\""));

        let seq_deserialized: ConcurrencyConfig = serde_json::from_str(&seq_json).unwrap();
        assert!(matches!(seq_deserialized.queue_mode, QueueMode::Sequential));
    }

    #[tokio::test]
    async fn test_sequential_blocks_second() {
        let config = ConcurrencyConfig {
            max_concurrent: 1,
            queue_mode: QueueMode::Sequential,
        };
        let limiter = ConcurrencyLimiter::new(&config);

        // Acquire the single permit
        let permit = limiter.acquire().await.unwrap();
        assert_eq!(limiter.active_count(), 1);
        assert_eq!(limiter.available(), 0);

        // Second try_acquire should fail
        assert!(limiter.try_acquire().is_none());

        // Drop the permit, now should be available again
        drop(permit);
        assert_eq!(limiter.available(), 1);
        assert_eq!(limiter.active_count(), 0);

        // Now try_acquire should succeed
        let permit2 = limiter.try_acquire();
        assert!(permit2.is_some());
    }

    #[tokio::test]
    async fn test_parallel_allows_multiple() {
        let config = ConcurrencyConfig {
            max_concurrent: 5,
            queue_mode: QueueMode::Parallel,
        };
        let limiter = ConcurrencyLimiter::new(&config);

        // Acquire all 5 permits
        let mut permits = Vec::new();
        for i in 0..5 {
            let p = limiter.try_acquire();
            assert!(p.is_some(), "Should be able to acquire permit {}", i);
            permits.push(p.unwrap());
        }

        // All permits used
        assert_eq!(limiter.available(), 0);
        assert_eq!(limiter.active_count(), 5);

        // 6th should fail
        assert!(limiter.try_acquire().is_none());

        // Release one
        permits.pop();
        assert_eq!(limiter.available(), 1);
        assert_eq!(limiter.active_count(), 4);

        // Now one more should succeed
        let extra = limiter.try_acquire();
        assert!(extra.is_some());
    }
}
