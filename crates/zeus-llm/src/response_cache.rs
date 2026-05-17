//! LLM Response Cache — reduce API costs and latency.
//!
//! Caches LLM responses keyed by a hash of the request (model + messages + params).
//! Supports:
//! - **Exact match caching**: Same request → cached response
//! - **TTL expiration**: Entries expire after configurable time
//! - **Size limits**: LRU eviction when cache exceeds max entries
//! - **Hit/miss tracking**: Cache effectiveness metrics
//! - **Selective caching**: Only cache responses meeting quality criteria
//! - **Cache invalidation**: Per-model, per-session, or full flush

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the response cache.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of cached entries.
    pub max_entries: usize,
    /// Time-to-live in seconds for cache entries.
    pub ttl_secs: u64,
    /// Whether caching is enabled.
    pub enabled: bool,
    /// Minimum response length to cache (skip very short responses).
    pub min_response_length: usize,
    /// Maximum response length to cache (skip very large responses).
    pub max_response_length: usize,
    /// Whether to cache error responses.
    pub cache_errors: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 1000,
            ttl_secs: 3600, // 1 hour
            enabled: true,
            min_response_length: 1,
            max_response_length: 100_000,
            cache_errors: false,
        }
    }
}

// ============================================================================
// Types
// ============================================================================

/// A cached response entry.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// Hash key of the request.
    pub key: String,
    /// Model used.
    pub model: String,
    /// Cached response text.
    pub response: String,
    /// Input tokens reported.
    pub input_tokens: u64,
    /// Output tokens reported.
    pub output_tokens: u64,
    /// Whether this was an error response.
    pub is_error: bool,
    /// When the entry was created (unix secs).
    pub created_at: u64,
    /// Last access time (unix secs).
    pub last_accessed: u64,
    /// Number of times this entry was hit.
    pub hit_count: u64,
}

/// Result of a cache lookup.
#[derive(Debug, Clone)]
pub enum CacheLookup {
    /// Cache hit with the stored response.
    Hit(CacheEntry),
    /// Cache miss — request not found.
    Miss,
    /// Cache miss — entry expired.
    Expired,
    /// Cache disabled.
    Disabled,
}

impl CacheLookup {
    /// Whether this is a hit.
    pub fn is_hit(&self) -> bool {
        matches!(self, CacheLookup::Hit(_))
    }
}

/// Cache effectiveness statistics.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Total lookups performed.
    pub lookups: u64,
    /// Cache hits.
    pub hits: u64,
    /// Cache misses.
    pub misses: u64,
    /// Expired entry encounters.
    pub expirations: u64,
    /// Entries evicted due to size limit.
    pub evictions: u64,
    /// Total entries stored.
    pub entries_stored: u64,
    /// Current entry count.
    pub current_entries: usize,
    /// Estimated tokens saved by cache hits.
    pub tokens_saved: u64,
}

impl CacheStats {
    /// Hit rate as a ratio (0.0–1.0).
    pub fn hit_rate(&self) -> f64 {
        if self.lookups == 0 {
            0.0
        } else {
            self.hits as f64 / self.lookups as f64
        }
    }
}

// ============================================================================
// Cache Key Generation
// ============================================================================

/// Generate a cache key from request parameters.
///
/// Uses a simple hash of model + messages + temperature to create a deterministic key.
pub fn cache_key(model: &str, messages: &[(&str, &str)], temperature: Option<f64>) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    model.hash(&mut hasher);
    for (role, content) in messages {
        role.hash(&mut hasher);
        content.hash(&mut hasher);
    }
    if let Some(t) = temperature {
        // Hash temperature as bits to avoid float comparison issues
        t.to_bits().hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

// ============================================================================
// Response Cache
// ============================================================================

/// The LLM response cache.
pub struct ResponseCache {
    config: CacheConfig,
    entries: HashMap<String, CacheEntry>,
    stats: CacheStats,
}

impl ResponseCache {
    /// Create with default configuration.
    pub fn new() -> Self {
        Self {
            config: CacheConfig::default(),
            entries: HashMap::new(),
            stats: CacheStats::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: CacheConfig) -> Self {
        Self {
            config,
            entries: HashMap::new(),
            stats: CacheStats::default(),
        }
    }

    /// Update configuration.
    pub fn set_config(&mut self, config: CacheConfig) {
        self.config = config;
    }

    /// Get current statistics.
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Get current entry count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look up a cached response.
    pub fn get(&mut self, key: &str) -> CacheLookup {
        if !self.config.enabled {
            return CacheLookup::Disabled;
        }

        self.stats.lookups += 1;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if let Some(entry) = self.entries.get_mut(key) {
            // Check TTL
            if now > entry.created_at + self.config.ttl_secs {
                let _expired = entry.clone();
                self.entries.remove(key);
                self.stats.expirations += 1;
                self.stats.misses += 1;
                self.stats.current_entries = self.entries.len();
                return CacheLookup::Expired;
            }

            entry.last_accessed = now;
            entry.hit_count += 1;
            self.stats.hits += 1;
            self.stats.tokens_saved += entry.input_tokens + entry.output_tokens;
            CacheLookup::Hit(entry.clone())
        } else {
            self.stats.misses += 1;
            CacheLookup::Miss
        }
    }

    /// Store a response in the cache.
    pub fn put(
        &mut self,
        key: &str,
        model: &str,
        response: &str,
        input_tokens: u64,
        output_tokens: u64,
        is_error: bool,
    ) -> bool {
        if !self.config.enabled {
            return false;
        }

        // Skip errors unless configured to cache them
        if is_error && !self.config.cache_errors {
            return false;
        }

        // Check response length bounds
        let len = response.len();
        if len < self.config.min_response_length || len > self.config.max_response_length {
            return false;
        }

        // Evict if at capacity (LRU: remove least recently accessed)
        if self.entries.len() >= self.config.max_entries && !self.entries.contains_key(key) {
            self.evict_lru();
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.entries.insert(
            key.to_string(),
            CacheEntry {
                key: key.to_string(),
                model: model.to_string(),
                response: response.to_string(),
                input_tokens,
                output_tokens,
                is_error,
                created_at: now,
                last_accessed: now,
                hit_count: 0,
            },
        );

        self.stats.entries_stored += 1;
        self.stats.current_entries = self.entries.len();
        true
    }

    /// Invalidate a specific entry.
    pub fn invalidate(&mut self, key: &str) -> bool {
        let removed = self.entries.remove(key).is_some();
        self.stats.current_entries = self.entries.len();
        removed
    }

    /// Invalidate all entries for a specific model.
    pub fn invalidate_model(&mut self, model: &str) -> usize {
        let before = self.entries.len();
        self.entries.retain(|_, e| e.model != model);
        let removed = before - self.entries.len();
        self.stats.current_entries = self.entries.len();
        removed
    }

    /// Flush the entire cache.
    pub fn flush(&mut self) -> usize {
        let count = self.entries.len();
        self.entries.clear();
        self.stats.current_entries = 0;
        count
    }

    /// Remove expired entries.
    pub fn cleanup_expired(&mut self) -> usize {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let ttl = self.config.ttl_secs;

        let before = self.entries.len();
        self.entries.retain(|_, e| now <= e.created_at + ttl);
        let removed = before - self.entries.len();
        self.stats.expirations += removed as u64;
        self.stats.current_entries = self.entries.len();
        removed
    }

    /// Get the top N most-hit cache entries.
    pub fn top_hits(&self, limit: usize) -> Vec<&CacheEntry> {
        let mut entries: Vec<&CacheEntry> = self.entries.values().collect();
        entries.sort_by(|a, b| b.hit_count.cmp(&a.hit_count));
        entries.truncate(limit);
        entries
    }

    /// Evict the least recently used entry.
    fn evict_lru(&mut self) {
        if let Some(lru_key) = self
            .entries
            .iter()
            .min_by_key(|(_, e)| e.last_accessed)
            .map(|(k, _)| k.clone())
        {
            self.entries.remove(&lru_key);
            self.stats.evictions += 1;
            self.stats.current_entries = self.entries.len();
        }
    }
}

impl Default for ResponseCache {
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
    fn test_default_config() {
        let config = CacheConfig::default();
        assert_eq!(config.max_entries, 1000);
        assert_eq!(config.ttl_secs, 3600);
        assert!(config.enabled);
        assert!(!config.cache_errors);
    }

    #[test]
    fn test_new_cache() {
        let c = ResponseCache::new();
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
        assert_eq!(c.stats().lookups, 0);
    }

    #[test]
    fn test_cache_key_deterministic() {
        let k1 = cache_key("model", &[("user", "hello")], Some(0.7));
        let k2 = cache_key("model", &[("user", "hello")], Some(0.7));
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_cache_key_different_model() {
        let k1 = cache_key("model-a", &[("user", "hello")], None);
        let k2 = cache_key("model-b", &[("user", "hello")], None);
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_cache_key_different_messages() {
        let k1 = cache_key("model", &[("user", "hello")], None);
        let k2 = cache_key("model", &[("user", "world")], None);
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_cache_key_different_temperature() {
        let k1 = cache_key("model", &[("user", "hi")], Some(0.5));
        let k2 = cache_key("model", &[("user", "hi")], Some(0.7));
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_put_and_get() {
        let mut c = ResponseCache::new();
        let key = "test-key";
        c.put(key, "ollama/llama3", "Hello world!", 100, 50, false);
        let result = c.get(key);
        assert!(result.is_hit());
        if let CacheLookup::Hit(entry) = result {
            assert_eq!(entry.response, "Hello world!");
            assert_eq!(entry.model, "ollama/llama3");
            assert_eq!(entry.input_tokens, 100);
            assert_eq!(entry.hit_count, 1);
        }
    }

    #[test]
    fn test_miss() {
        let mut c = ResponseCache::new();
        let result = c.get("nonexistent");
        assert!(!result.is_hit());
        assert!(matches!(result, CacheLookup::Miss));
    }

    #[test]
    fn test_disabled_cache() {
        let mut c = ResponseCache::with_config(CacheConfig {
            enabled: false,
            ..CacheConfig::default()
        });
        assert!(!c.put("key", "model", "response", 0, 0, false));
        assert!(matches!(c.get("key"), CacheLookup::Disabled));
    }

    #[test]
    fn test_ttl_expiration() {
        let mut c = ResponseCache::with_config(CacheConfig {
            ttl_secs: 10,
            ..CacheConfig::default()
        });
        c.put("key", "model", "response", 0, 0, false);
        // Manually backdate the entry to simulate expiration
        if let Some(entry) = c.entries.get_mut("key") {
            entry.created_at = entry.created_at.saturating_sub(20);
        }
        let result = c.get("key");
        assert!(matches!(result, CacheLookup::Expired));
        assert_eq!(c.stats().expirations, 1);
    }

    #[test]
    fn test_error_not_cached_by_default() {
        let mut c = ResponseCache::new();
        let stored = c.put("key", "model", "error message", 0, 0, true);
        assert!(!stored);
    }

    #[test]
    fn test_error_cached_when_enabled() {
        let mut c = ResponseCache::with_config(CacheConfig {
            cache_errors: true,
            ..CacheConfig::default()
        });
        assert!(c.put("key", "model", "error message", 0, 0, true));
        assert!(c.get("key").is_hit());
    }

    #[test]
    fn test_min_response_length() {
        let mut c = ResponseCache::with_config(CacheConfig {
            min_response_length: 10,
            ..CacheConfig::default()
        });
        assert!(!c.put("key", "model", "short", 0, 0, false));
        assert!(c.put("key2", "model", "long enough response", 0, 0, false));
    }

    #[test]
    fn test_max_response_length() {
        let mut c = ResponseCache::with_config(CacheConfig {
            max_response_length: 10,
            ..CacheConfig::default()
        });
        assert!(!c.put(
            "key",
            "model",
            "this is way too long for the cache",
            0,
            0,
            false
        ));
        assert!(c.put("key2", "model", "short", 0, 0, false));
    }

    #[test]
    fn test_lru_eviction() {
        let mut c = ResponseCache::with_config(CacheConfig {
            max_entries: 2,
            ..CacheConfig::default()
        });
        c.put("k1", "m", "resp1", 0, 0, false);
        c.put("k2", "m", "resp2", 0, 0, false);
        // Stagger last_accessed so k2 is clearly older
        if let Some(e) = c.entries.get_mut("k2") {
            e.last_accessed = e.last_accessed.saturating_sub(100);
        }
        // Adding k3 should evict k2 (least recently accessed)
        c.put("k3", "m", "resp3", 0, 0, false);
        assert_eq!(c.len(), 2);
        assert!(c.get("k1").is_hit());
        assert!(!c.get("k2").is_hit());
        assert!(c.get("k3").is_hit());
        assert!(c.stats().evictions >= 1);
    }

    #[test]
    fn test_invalidate() {
        let mut c = ResponseCache::new();
        c.put("k1", "m", "resp", 0, 0, false);
        assert!(c.invalidate("k1"));
        assert!(!c.invalidate("k1")); // Already removed
        assert!(c.is_empty());
    }

    #[test]
    fn test_invalidate_model() {
        let mut c = ResponseCache::new();
        c.put("k1", "model-a", "resp1", 0, 0, false);
        c.put("k2", "model-a", "resp2", 0, 0, false);
        c.put("k3", "model-b", "resp3", 0, 0, false);
        let removed = c.invalidate_model("model-a");
        assert_eq!(removed, 2);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn test_flush() {
        let mut c = ResponseCache::new();
        c.put("k1", "m", "r", 0, 0, false);
        c.put("k2", "m", "r", 0, 0, false);
        let flushed = c.flush();
        assert_eq!(flushed, 2);
        assert!(c.is_empty());
    }

    #[test]
    fn test_cleanup_expired() {
        let mut c = ResponseCache::with_config(CacheConfig {
            ttl_secs: 10,
            ..CacheConfig::default()
        });
        c.put("k1", "m", "r", 0, 0, false);
        c.put("k2", "m", "r", 0, 0, false);
        // Backdate both entries
        for entry in c.entries.values_mut() {
            entry.created_at = entry.created_at.saturating_sub(20);
        }
        let cleaned = c.cleanup_expired();
        assert_eq!(cleaned, 2);
        assert!(c.is_empty());
    }

    #[test]
    fn test_stats_tracking() {
        let mut c = ResponseCache::new();
        c.put("k1", "m", "response text", 100, 50, false);
        c.get("k1"); // Hit
        c.get("k2"); // Miss
        assert_eq!(c.stats().lookups, 2);
        assert_eq!(c.stats().hits, 1);
        assert_eq!(c.stats().misses, 1);
        assert_eq!(c.stats().entries_stored, 1);
        assert_eq!(c.stats().tokens_saved, 150); // 100 + 50
    }

    #[test]
    fn test_hit_rate() {
        let mut c = ResponseCache::new();
        c.put("k1", "m", "resp", 0, 0, false);
        c.get("k1"); // Hit
        c.get("k1"); // Hit
        c.get("k2"); // Miss
        assert!((c.stats().hit_rate() - 2.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_hit_rate_empty() {
        let stats = CacheStats::default();
        assert_eq!(stats.hit_rate(), 0.0);
    }

    #[test]
    fn test_top_hits() {
        let mut c = ResponseCache::new();
        c.put("k1", "m", "resp1", 0, 0, false);
        c.put("k2", "m", "resp2", 0, 0, false);
        c.get("k1"); // 1 hit
        c.get("k2"); // 1 hit
        c.get("k2"); // 2 hits
        c.get("k2"); // 3 hits

        let top = c.top_hits(1);
        assert_eq!(top[0].key, "k2");
        assert_eq!(top[0].hit_count, 3);
    }

    #[test]
    fn test_overwrite_existing_key() {
        let mut c = ResponseCache::new();
        c.put("k1", "m", "old response", 0, 0, false);
        c.put("k1", "m", "new response", 0, 0, false);
        assert_eq!(c.len(), 1);
        if let CacheLookup::Hit(entry) = c.get("k1") {
            assert_eq!(entry.response, "new response");
        }
    }

    #[test]
    fn test_set_config() {
        let mut c = ResponseCache::new();
        assert_eq!(c.config.max_entries, 1000);
        c.set_config(CacheConfig {
            max_entries: 50,
            ..CacheConfig::default()
        });
        assert_eq!(c.config.max_entries, 50);
    }

    #[test]
    fn test_multiple_hits_increment() {
        let mut c = ResponseCache::new();
        c.put("k", "m", "resp", 10, 5, false);
        c.get("k");
        c.get("k");
        c.get("k");
        if let CacheLookup::Hit(entry) = c.get("k") {
            assert_eq!(entry.hit_count, 4);
        }
        assert_eq!(c.stats().tokens_saved, 60); // 4 × 15
    }
}
