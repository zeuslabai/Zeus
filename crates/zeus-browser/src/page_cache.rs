//! Page Cache for browser automation.
//!
//! Caches fetched page content (HTML, text, screenshots) to avoid redundant
//! navigation and reduce latency for repeated access patterns:
//!
//! - **PageCache** — LRU cache with TTL for page content
//! - **CacheEntry** — cached page with metadata (URL, title, content type, size)
//! - **CachePolicy** — per-URL caching rules (TTL override, force-refresh patterns)
//! - **CacheStats** — hit/miss tracking and storage metrics

use std::collections::HashMap;

use chrono::{DateTime, Utc};

// ============================================================================
// Content types
// ============================================================================

/// Type of cached content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    Html,
    Text,
    Screenshot,
    Json,
}

impl ContentType {
    /// Display name.
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentType::Html => "html",
            ContentType::Text => "text",
            ContentType::Screenshot => "screenshot",
            ContentType::Json => "json",
        }
    }
}

// ============================================================================
// Cache entry
// ============================================================================

/// A cached page or content item.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// The URL this content was fetched from.
    pub url: String,
    /// Page title (if available).
    pub title: Option<String>,
    /// The cached content.
    pub content: String,
    /// Type of content.
    pub content_type: ContentType,
    /// Size in bytes.
    pub size_bytes: usize,
    /// When the content was cached.
    pub cached_at: DateTime<Utc>,
    /// When the entry expires.
    pub expires_at: DateTime<Utc>,
    /// Number of times this entry has been accessed.
    pub hit_count: u64,
    /// Last time this entry was accessed.
    pub last_accessed: DateTime<Utc>,
}

impl CacheEntry {
    /// Check if this entry has expired.
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Check if this entry is still valid (not expired).
    pub fn is_valid(&self) -> bool {
        !self.is_expired()
    }

    /// Age of this entry in seconds.
    pub fn age_secs(&self) -> i64 {
        Utc::now()
            .signed_duration_since(self.cached_at)
            .num_seconds()
    }
}

// ============================================================================
// Cache policy
// ============================================================================

/// Per-pattern caching rules.
#[derive(Debug, Clone)]
pub struct CacheRule {
    /// URL pattern to match (prefix match).
    pub url_pattern: String,
    /// TTL override in seconds (None = use default).
    pub ttl_secs: Option<u64>,
    /// Never cache URLs matching this pattern.
    pub no_cache: bool,
}

/// Cache policy configuration.
#[derive(Debug, Clone)]
pub struct CachePolicy {
    /// Default TTL in seconds.
    pub default_ttl_secs: u64,
    /// Maximum cache size in bytes.
    pub max_size_bytes: usize,
    /// Maximum number of entries.
    pub max_entries: usize,
    /// Per-URL rules (checked in order, first match wins).
    pub rules: Vec<CacheRule>,
    /// URL patterns that should never be cached.
    pub no_cache_patterns: Vec<String>,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            default_ttl_secs: 300,            // 5 minutes
            max_size_bytes: 50 * 1024 * 1024, // 50 MB
            max_entries: 100,
            rules: Vec::new(),
            no_cache_patterns: Vec::new(),
        }
    }
}

impl CachePolicy {
    /// Get the TTL for a given URL.
    pub fn ttl_for(&self, url: &str) -> Option<u64> {
        // Check no-cache patterns first
        for pattern in &self.no_cache_patterns {
            if url.contains(pattern) {
                return None;
            }
        }

        // Check rules
        for rule in &self.rules {
            if url.starts_with(&rule.url_pattern) || url.contains(&rule.url_pattern) {
                if rule.no_cache {
                    return None;
                }
                if let Some(ttl) = rule.ttl_secs {
                    return Some(ttl);
                }
            }
        }

        Some(self.default_ttl_secs)
    }
}

// ============================================================================
// Cache statistics
// ============================================================================

/// Cache hit/miss and storage statistics.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub expirations: u64,
    pub total_entries: usize,
    pub total_size_bytes: usize,
}

impl CacheStats {
    /// Hit rate as a fraction (0.0–1.0).
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            return 0.0;
        }
        self.hits as f64 / total as f64
    }

    /// Total lookups.
    pub fn total_lookups(&self) -> u64 {
        self.hits + self.misses
    }
}

// ============================================================================
// Page Cache
// ============================================================================

/// LRU page cache with TTL-based expiration.
pub struct PageCache {
    entries: HashMap<String, CacheEntry>,
    policy: CachePolicy,
    stats: CacheStats,
}

impl PageCache {
    /// Create a new page cache with the given policy.
    pub fn new(policy: CachePolicy) -> Self {
        Self {
            entries: HashMap::new(),
            policy,
            stats: CacheStats::default(),
        }
    }

    /// Create with default policy.
    pub fn with_defaults() -> Self {
        Self::new(CachePolicy::default())
    }

    /// Look up a cached page by URL. Returns None if not found or expired.
    pub fn get(&mut self, url: &str) -> Option<&CacheEntry> {
        // Check if entry exists and is valid
        let is_valid = self.entries.get(url).map(|e| e.is_valid()).unwrap_or(false);

        if !is_valid {
            if self.entries.contains_key(url) {
                // Entry exists but expired
                self.entries.remove(url);
                self.stats.expirations += 1;
                self.stats.total_entries = self.entries.len();
            }
            self.stats.misses += 1;
            return None;
        }

        // Update access stats
        if let Some(entry) = self.entries.get_mut(url) {
            entry.hit_count += 1;
            entry.last_accessed = Utc::now();
        }

        self.stats.hits += 1;
        self.entries.get(url)
    }

    /// Store page content in the cache.
    pub fn put(
        &mut self,
        url: &str,
        content: String,
        content_type: ContentType,
        title: Option<String>,
    ) -> bool {
        // Check if URL should be cached
        let ttl = match self.policy.ttl_for(url) {
            Some(ttl) => ttl,
            None => return false, // no-cache
        };

        let size = content.len();

        // Enforce max entry limit — evict LRU if needed
        while self.entries.len() >= self.policy.max_entries {
            self.evict_lru();
        }

        // Enforce max size — evict until we have room
        let current_size: usize = self.entries.values().map(|e| e.size_bytes).sum();
        let mut remaining_budget = self.policy.max_size_bytes.saturating_sub(current_size);
        while size > remaining_budget && !self.entries.is_empty() {
            self.evict_lru();
            let current: usize = self.entries.values().map(|e| e.size_bytes).sum();
            remaining_budget = self.policy.max_size_bytes.saturating_sub(current);
        }

        let now = Utc::now();
        let expires_at = now + chrono::Duration::seconds(ttl as i64);

        self.entries.insert(
            url.to_string(),
            CacheEntry {
                url: url.to_string(),
                title,
                content,
                content_type,
                size_bytes: size,
                cached_at: now,
                expires_at,
                hit_count: 0,
                last_accessed: now,
            },
        );

        self.stats.total_entries = self.entries.len();
        self.stats.total_size_bytes = self.entries.values().map(|e| e.size_bytes).sum();
        true
    }

    /// Invalidate (remove) a cached entry by URL.
    pub fn invalidate(&mut self, url: &str) -> bool {
        let removed = self.entries.remove(url).is_some();
        if removed {
            self.stats.total_entries = self.entries.len();
            self.stats.total_size_bytes = self.entries.values().map(|e| e.size_bytes).sum();
        }
        removed
    }

    /// Invalidate all entries matching a URL prefix.
    pub fn invalidate_prefix(&mut self, prefix: &str) -> usize {
        let to_remove: Vec<String> = self
            .entries
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();
        let count = to_remove.len();
        for key in to_remove {
            self.entries.remove(&key);
        }
        self.stats.total_entries = self.entries.len();
        self.stats.total_size_bytes = self.entries.values().map(|e| e.size_bytes).sum();
        count
    }

    /// Remove all expired entries.
    pub fn cleanup_expired(&mut self) -> usize {
        let expired: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| e.is_expired())
            .map(|(k, _)| k.clone())
            .collect();
        let count = expired.len();
        for key in &expired {
            self.entries.remove(key);
        }
        self.stats.expirations += count as u64;
        self.stats.total_entries = self.entries.len();
        self.stats.total_size_bytes = self.entries.values().map(|e| e.size_bytes).sum();
        count
    }

    /// Clear the entire cache.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.stats.total_entries = 0;
        self.stats.total_size_bytes = 0;
    }

    /// Get cache statistics.
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total cached size in bytes.
    pub fn total_size(&self) -> usize {
        self.entries.values().map(|e| e.size_bytes).sum()
    }

    /// List all cached URLs.
    pub fn cached_urls(&self) -> Vec<&str> {
        self.entries.keys().map(|k| k.as_str()).collect()
    }

    /// Evict the least recently used entry.
    fn evict_lru(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let lru_key = self
            .entries
            .iter()
            .min_by_key(|(_, e)| e.last_accessed)
            .map(|(k, _)| k.clone());

        if let Some(key) = lru_key {
            self.entries.remove(&key);
            self.stats.evictions += 1;
        }
    }
}

impl Default for PageCache {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- ContentType --------------------------------------------------------

    #[test]
    fn test_content_type_as_str() {
        assert_eq!(ContentType::Html.as_str(), "html");
        assert_eq!(ContentType::Text.as_str(), "text");
        assert_eq!(ContentType::Screenshot.as_str(), "screenshot");
        assert_eq!(ContentType::Json.as_str(), "json");
    }

    // -- CachePolicy --------------------------------------------------------

    #[test]
    fn test_policy_default() {
        let policy = CachePolicy::default();
        assert_eq!(policy.default_ttl_secs, 300);
        assert_eq!(policy.max_entries, 100);
    }

    #[test]
    fn test_policy_ttl_default() {
        let policy = CachePolicy::default();
        assert_eq!(policy.ttl_for("https://example.com"), Some(300));
    }

    #[test]
    fn test_policy_no_cache_pattern() {
        let policy = CachePolicy {
            no_cache_patterns: vec!["localhost".to_string()],
            ..Default::default()
        };
        assert_eq!(policy.ttl_for("http://localhost:3000"), None);
        assert_eq!(policy.ttl_for("https://example.com"), Some(300));
    }

    #[test]
    fn test_policy_rule_ttl_override() {
        let policy = CachePolicy {
            rules: vec![CacheRule {
                url_pattern: "https://api.".to_string(),
                ttl_secs: Some(60),
                no_cache: false,
            }],
            ..Default::default()
        };
        assert_eq!(policy.ttl_for("https://api.example.com/data"), Some(60));
        assert_eq!(policy.ttl_for("https://www.example.com"), Some(300));
    }

    #[test]
    fn test_policy_rule_no_cache() {
        let policy = CachePolicy {
            rules: vec![CacheRule {
                url_pattern: "/auth/".to_string(),
                ttl_secs: None,
                no_cache: true,
            }],
            ..Default::default()
        };
        assert_eq!(policy.ttl_for("https://example.com/auth/login"), None);
    }

    // -- CacheStats ---------------------------------------------------------

    #[test]
    fn test_stats_default() {
        let stats = CacheStats::default();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
        assert!((stats.hit_rate() - 0.0).abs() < f64::EPSILON);
        assert_eq!(stats.total_lookups(), 0);
    }

    #[test]
    fn test_stats_hit_rate() {
        let stats = CacheStats {
            hits: 3,
            misses: 1,
            ..Default::default()
        };
        assert!((stats.hit_rate() - 0.75).abs() < f64::EPSILON);
        assert_eq!(stats.total_lookups(), 4);
    }

    // -- PageCache basic ops ------------------------------------------------

    #[test]
    fn test_cache_new_empty() {
        let cache = PageCache::with_defaults();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.total_size(), 0);
    }

    #[test]
    fn test_cache_put_and_get() {
        let mut cache = PageCache::with_defaults();
        cache.put(
            "https://example.com",
            "<html>hello</html>".to_string(),
            ContentType::Html,
            Some("Example".to_string()),
        );
        assert_eq!(cache.len(), 1);

        let entry = cache.get("https://example.com").unwrap();
        assert_eq!(entry.url, "https://example.com");
        assert_eq!(entry.content, "<html>hello</html>");
        assert_eq!(entry.content_type, ContentType::Html);
        assert_eq!(entry.title.as_deref(), Some("Example"));
        assert_eq!(entry.hit_count, 1);
    }

    #[test]
    fn test_cache_miss() {
        let mut cache = PageCache::with_defaults();
        assert!(cache.get("https://nonexistent.com").is_none());
        assert_eq!(cache.stats().misses, 1);
    }

    #[test]
    fn test_cache_hit_increments_count() {
        let mut cache = PageCache::with_defaults();
        cache.put(
            "https://example.com",
            "content".to_string(),
            ContentType::Text,
            None,
        );
        cache.get("https://example.com");
        cache.get("https://example.com");
        let entry = cache.get("https://example.com").unwrap();
        assert_eq!(entry.hit_count, 3);
        assert_eq!(cache.stats().hits, 3);
    }

    #[test]
    fn test_cache_invalidate() {
        let mut cache = PageCache::with_defaults();
        cache.put("https://a.com", "a".to_string(), ContentType::Html, None);
        cache.put("https://b.com", "b".to_string(), ContentType::Html, None);
        assert!(cache.invalidate("https://a.com"));
        assert_eq!(cache.len(), 1);
        assert!(!cache.invalidate("https://nonexistent.com"));
    }

    #[test]
    fn test_cache_invalidate_prefix() {
        let mut cache = PageCache::with_defaults();
        cache.put(
            "https://api.example.com/v1/users",
            "u".to_string(),
            ContentType::Json,
            None,
        );
        cache.put(
            "https://api.example.com/v1/posts",
            "p".to_string(),
            ContentType::Json,
            None,
        );
        cache.put(
            "https://www.example.com",
            "w".to_string(),
            ContentType::Html,
            None,
        );

        let removed = cache.invalidate_prefix("https://api.example.com");
        assert_eq!(removed, 2);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_cache_clear() {
        let mut cache = PageCache::with_defaults();
        cache.put("https://a.com", "a".to_string(), ContentType::Html, None);
        cache.put("https://b.com", "b".to_string(), ContentType::Html, None);
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.stats().total_entries, 0);
    }

    #[test]
    fn test_cache_overwrite() {
        let mut cache = PageCache::with_defaults();
        cache.put(
            "https://example.com",
            "old".to_string(),
            ContentType::Html,
            None,
        );
        cache.put(
            "https://example.com",
            "new".to_string(),
            ContentType::Html,
            None,
        );
        assert_eq!(cache.len(), 1);
        let entry = cache.get("https://example.com").unwrap();
        assert_eq!(entry.content, "new");
    }

    // -- TTL expiration -----------------------------------------------------

    #[test]
    fn test_cache_expired_entry_removed_on_get() {
        let policy = CachePolicy {
            default_ttl_secs: 300,
            ..Default::default()
        };
        let mut cache = PageCache::new(policy);
        cache.put(
            "https://example.com",
            "content".to_string(),
            ContentType::Html,
            None,
        );

        // Manually backdate the entry to make it expired
        if let Some(entry) = cache.entries.get_mut("https://example.com") {
            entry.expires_at = Utc::now() - chrono::Duration::seconds(10);
        }

        assert!(cache.get("https://example.com").is_none());
        assert_eq!(cache.stats().expirations, 1);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_cleanup_expired() {
        let mut cache = PageCache::with_defaults();
        cache.put("https://a.com", "a".to_string(), ContentType::Html, None);
        cache.put("https://b.com", "b".to_string(), ContentType::Html, None);
        cache.put("https://c.com", "c".to_string(), ContentType::Html, None);

        // Backdate two entries
        for url in &["https://a.com", "https://b.com"] {
            if let Some(entry) = cache.entries.get_mut(*url) {
                entry.expires_at = Utc::now() - chrono::Duration::seconds(10);
            }
        }

        let cleaned = cache.cleanup_expired();
        assert_eq!(cleaned, 2);
        assert_eq!(cache.len(), 1);
    }

    // -- LRU eviction -------------------------------------------------------

    #[test]
    fn test_lru_eviction_on_max_entries() {
        let policy = CachePolicy {
            max_entries: 3,
            ..Default::default()
        };
        let mut cache = PageCache::new(policy);

        cache.put("https://a.com", "a".to_string(), ContentType::Html, None);
        // Backdate "a" to be the oldest accessed
        if let Some(entry) = cache.entries.get_mut("https://a.com") {
            entry.last_accessed = Utc::now() - chrono::Duration::seconds(100);
        }

        cache.put("https://b.com", "b".to_string(), ContentType::Html, None);
        cache.put("https://c.com", "c".to_string(), ContentType::Html, None);

        // This should evict "a" (least recently used)
        cache.put("https://d.com", "d".to_string(), ContentType::Html, None);
        assert_eq!(cache.len(), 3);
        assert!(cache.entries.get("https://a.com").is_none());
        assert!(cache.entries.get("https://d.com").is_some());
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn test_lru_eviction_on_max_size() {
        let policy = CachePolicy {
            max_size_bytes: 100,
            max_entries: 1000,
            ..Default::default()
        };
        let mut cache = PageCache::new(policy);

        // Each entry ~40 bytes of content
        cache.put("https://a.com", "a".repeat(40), ContentType::Html, None);
        if let Some(entry) = cache.entries.get_mut("https://a.com") {
            entry.last_accessed = Utc::now() - chrono::Duration::seconds(100);
        }
        cache.put("https://b.com", "b".repeat(40), ContentType::Html, None);

        // This 40-byte entry should trigger eviction of "a" (total would be 120 > 100)
        cache.put("https://c.com", "c".repeat(40), ContentType::Html, None);
        assert!(cache.total_size() <= 100);
        assert!(cache.entries.get("https://a.com").is_none());
    }

    // -- No-cache -----------------------------------------------------------

    #[test]
    fn test_no_cache_pattern_prevents_storage() {
        let policy = CachePolicy {
            no_cache_patterns: vec!["localhost".to_string()],
            ..Default::default()
        };
        let mut cache = PageCache::new(policy);

        let stored = cache.put(
            "http://localhost:3000/api",
            "data".to_string(),
            ContentType::Json,
            None,
        );
        assert!(!stored);
        assert!(cache.is_empty());
    }

    // -- CacheEntry ---------------------------------------------------------

    #[test]
    fn test_entry_is_valid() {
        let entry = CacheEntry {
            url: "https://example.com".to_string(),
            title: None,
            content: "test".to_string(),
            content_type: ContentType::Html,
            size_bytes: 4,
            cached_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::seconds(300),
            hit_count: 0,
            last_accessed: Utc::now(),
        };
        assert!(entry.is_valid());
        assert!(!entry.is_expired());
    }

    #[test]
    fn test_entry_is_expired() {
        let entry = CacheEntry {
            url: "https://example.com".to_string(),
            title: None,
            content: "test".to_string(),
            content_type: ContentType::Html,
            size_bytes: 4,
            cached_at: Utc::now() - chrono::Duration::seconds(400),
            expires_at: Utc::now() - chrono::Duration::seconds(100),
            hit_count: 0,
            last_accessed: Utc::now() - chrono::Duration::seconds(400),
        };
        assert!(entry.is_expired());
        assert!(!entry.is_valid());
    }

    #[test]
    fn test_entry_age() {
        let entry = CacheEntry {
            url: "https://example.com".to_string(),
            title: None,
            content: "test".to_string(),
            content_type: ContentType::Html,
            size_bytes: 4,
            cached_at: Utc::now() - chrono::Duration::seconds(60),
            expires_at: Utc::now() + chrono::Duration::seconds(240),
            hit_count: 0,
            last_accessed: Utc::now(),
        };
        let age = entry.age_secs();
        assert!(age >= 59 && age <= 61);
    }

    // -- cached_urls --------------------------------------------------------

    #[test]
    fn test_cached_urls() {
        let mut cache = PageCache::with_defaults();
        cache.put("https://a.com", "a".to_string(), ContentType::Html, None);
        cache.put("https://b.com", "b".to_string(), ContentType::Html, None);
        let urls = cache.cached_urls();
        assert_eq!(urls.len(), 2);
        assert!(urls.contains(&"https://a.com"));
        assert!(urls.contains(&"https://b.com"));
    }

    // -- Stats integration --------------------------------------------------

    #[test]
    fn test_stats_tracking() {
        let mut cache = PageCache::with_defaults();
        cache.put(
            "https://example.com",
            "content".to_string(),
            ContentType::Html,
            None,
        );

        // Hit
        cache.get("https://example.com");
        // Miss
        cache.get("https://other.com");

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert!((stats.hit_rate() - 0.5).abs() < f64::EPSILON);
        assert_eq!(stats.total_entries, 1);
        assert!(stats.total_size_bytes > 0);
    }

    #[test]
    fn test_stats_after_clear() {
        let mut cache = PageCache::with_defaults();
        cache.put("https://a.com", "a".to_string(), ContentType::Html, None);
        cache.clear();
        assert_eq!(cache.stats().total_entries, 0);
        assert_eq!(cache.stats().total_size_bytes, 0);
    }

    // -- Multiple content types ---------------------------------------------

    #[test]
    fn test_different_content_types() {
        let mut cache = PageCache::with_defaults();
        cache.put(
            "https://a.com/page",
            "<html>".to_string(),
            ContentType::Html,
            None,
        );
        cache.put(
            "https://a.com/text",
            "plain text".to_string(),
            ContentType::Text,
            None,
        );
        cache.put(
            "https://a.com/data",
            r#"{"key":"val"}"#.to_string(),
            ContentType::Json,
            None,
        );
        cache.put(
            "https://a.com/img",
            "base64data".to_string(),
            ContentType::Screenshot,
            None,
        );

        assert_eq!(cache.len(), 4);
        assert_eq!(
            cache.get("https://a.com/page").unwrap().content_type,
            ContentType::Html
        );
        assert_eq!(
            cache.get("https://a.com/data").unwrap().content_type,
            ContentType::Json
        );
    }
}
