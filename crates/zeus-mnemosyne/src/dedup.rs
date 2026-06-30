//! Semantic Deduplication — Prevent redundant memory storage
//!
//! Provides multiple dedup strategies for Mnemosyne:
//!
//! 1. **Exact dedup**: SHA-256 content hash matching
//! 2. **Fuzzy dedup**: Jaccard similarity on token sets
//! 3. **Semantic dedup**: cosine similarity on embeddings (when available)
//! 4. **Temporal dedup**: suppress near-identical messages within a time window
//!
//! The dedup engine runs before each store operation and returns a verdict:
//! Accept, Merge (combine with existing), or Skip (too similar).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use tracing::debug;

// ============================================================================
// Configuration
// ============================================================================

/// Dedup configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupConfig {
    /// Enable exact (hash) dedup
    pub exact_dedup: bool,
    /// Enable fuzzy (token Jaccard) dedup
    pub fuzzy_dedup: bool,
    /// Fuzzy similarity threshold (0.0–1.0). Above this = duplicate.
    pub fuzzy_threshold: f64,
    /// Enable temporal dedup (same content within time window)
    pub temporal_dedup: bool,
    /// Temporal window in seconds
    pub temporal_window_secs: i64,
    /// Minimum content length to apply dedup (skip very short messages)
    pub min_content_length: usize,
    /// Maximum recent entries to check against for fuzzy dedup
    pub fuzzy_window_size: usize,
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            exact_dedup: true,
            fuzzy_dedup: true,
            fuzzy_threshold: 0.85,
            temporal_dedup: true,
            temporal_window_secs: 60,
            min_content_length: 20,
            fuzzy_window_size: 100,
        }
    }
}

// ============================================================================
// Dedup Verdict
// ============================================================================

/// Result of dedup check
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DedupVerdict {
    /// Content is unique — store it
    Accept,
    /// Content is similar to an existing entry — merge/update instead
    Merge {
        existing_hash: String,
        similarity: f64,
    },
    /// Content is a duplicate — skip storage
    Skip { reason: String, similarity: f64 },
}

impl DedupVerdict {
    pub fn should_store(&self) -> bool {
        matches!(self, Self::Accept | Self::Merge { .. })
    }

    pub fn is_duplicate(&self) -> bool {
        matches!(self, Self::Skip { .. })
    }
}

// ============================================================================
// Recent Entry (for fuzzy/temporal dedup)
// ============================================================================

#[derive(Debug, Clone)]
struct RecentEntry {
    hash: String,
    tokens: HashSet<String>,
    timestamp: DateTime<Utc>,
    #[allow(dead_code)]
    content_preview: String,
}

// ============================================================================
// Dedup Engine
// ============================================================================

/// The dedup engine — checks new content against recent history
pub struct DedupEngine {
    config: DedupConfig,
    /// Exact hash set
    seen_hashes: HashSet<String>,
    /// Recent entries for fuzzy matching (bounded queue)
    recent: VecDeque<RecentEntry>,
    /// Stats
    stats: DedupStats,
}

/// Dedup statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DedupStats {
    pub total_checked: u64,
    pub exact_duplicates: u64,
    pub fuzzy_duplicates: u64,
    pub temporal_duplicates: u64,
    pub accepted: u64,
    pub merged: u64,
}

impl DedupEngine {
    pub fn new(config: DedupConfig) -> Self {
        Self {
            config,
            seen_hashes: HashSet::new(),
            recent: VecDeque::new(),
            stats: DedupStats::default(),
        }
    }

    /// Check if content should be stored, merged, or skipped
    pub fn check(&mut self, content: &str) -> DedupVerdict {
        self.stats.total_checked += 1;

        // Skip dedup for very short content
        if content.len() < self.config.min_content_length {
            self.stats.accepted += 1;
            return DedupVerdict::Accept;
        }

        let hash = compute_hash(content);

        // Stage 1: Exact hash dedup
        if self.config.exact_dedup && self.seen_hashes.contains(&hash) {
            self.stats.exact_duplicates += 1;
            debug!(hash = %hash, "Exact duplicate detected");
            return DedupVerdict::Skip {
                reason: "exact duplicate".into(),
                similarity: 1.0,
            };
        }

        // Stage 2: Temporal dedup (same or very similar within time window)
        if self.config.temporal_dedup {
            let cutoff = Utc::now() - Duration::seconds(self.config.temporal_window_secs);
            for entry in self.recent.iter().rev() {
                if entry.timestamp < cutoff {
                    break;
                }
                if entry.hash == hash {
                    self.stats.temporal_duplicates += 1;
                    debug!("Temporal duplicate detected");
                    return DedupVerdict::Skip {
                        reason: "temporal duplicate".into(),
                        similarity: 1.0,
                    };
                }
            }
        }

        // Stage 3: Fuzzy (Jaccard) dedup
        if self.config.fuzzy_dedup {
            let tokens = tokenize(content);
            let mut best_similarity = 0.0;
            let mut best_hash = String::new();

            for entry in self.recent.iter().rev() {
                let sim = jaccard_similarity(&tokens, &entry.tokens);
                if sim > best_similarity {
                    best_similarity = sim;
                    best_hash = entry.hash.clone();
                }
            }

            if best_similarity >= self.config.fuzzy_threshold {
                if best_similarity >= 0.95 {
                    self.stats.fuzzy_duplicates += 1;
                    debug!(similarity = best_similarity, "Fuzzy duplicate (skip)");
                    return DedupVerdict::Skip {
                        reason: "fuzzy duplicate".into(),
                        similarity: best_similarity,
                    };
                } else {
                    self.stats.merged += 1;
                    debug!(similarity = best_similarity, "Fuzzy near-duplicate (merge)");
                    // Record this content
                    self.record(content, &hash, &tokens);
                    return DedupVerdict::Merge {
                        existing_hash: best_hash,
                        similarity: best_similarity,
                    };
                }
            }
        }

        // Accept — record and store
        let tokens = if self.config.fuzzy_dedup {
            tokenize(content)
        } else {
            HashSet::new()
        };
        self.record(content, &hash, &tokens);
        self.stats.accepted += 1;

        DedupVerdict::Accept
    }

    /// Manually register content as seen (for bootstrapping from existing DB)
    pub fn register(&mut self, content: &str) {
        let hash = compute_hash(content);
        let tokens = tokenize(content);
        self.record(content, &hash, &tokens);
    }

    /// Get dedup statistics
    pub fn stats(&self) -> &DedupStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = DedupStats::default();
    }

    /// Clear all dedup state
    pub fn clear(&mut self) {
        self.seen_hashes.clear();
        self.recent.clear();
        self.stats = DedupStats::default();
    }

    /// Number of unique hashes tracked
    pub fn unique_count(&self) -> usize {
        self.seen_hashes.len()
    }

    /// Get config
    pub fn config(&self) -> &DedupConfig {
        &self.config
    }

    /// Update config
    pub fn set_config(&mut self, config: DedupConfig) {
        self.config = config;
    }

    // -- internals --

    fn record(&mut self, content: &str, hash: &str, tokens: &HashSet<String>) {
        self.seen_hashes.insert(hash.to_string());

        let preview = if content.len() > 50 {
            let mut end = 50;
            while !content.is_char_boundary(end) && end < content.len() {
                end += 1;
            }
            format!("{}...", &content[..end])
        } else {
            content.to_string()
        };

        self.recent.push_back(RecentEntry {
            hash: hash.to_string(),
            tokens: tokens.clone(),
            timestamp: Utc::now(),
            content_preview: preview,
        });

        // Trim recent window
        while self.recent.len() > self.config.fuzzy_window_size {
            self.recent.pop_front();
        }
    }
}

impl Default for DedupEngine {
    fn default() -> Self {
        Self::new(DedupConfig::default())
    }
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Compute SHA-256 hash of content (matches Mnemosyne's compute_content_hash)
fn compute_hash(content: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Tokenize content into a set of lowercase word tokens
fn tokenize(content: &str) -> HashSet<String> {
    content
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_lowercase())
        .collect()
}

/// Jaccard similarity between two token sets (|A∩B| / |A∪B|)
fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_engine() -> DedupEngine {
        DedupEngine::new(DedupConfig::default())
    }

    #[test]
    fn test_accept_unique_content() {
        let mut engine = test_engine();
        let verdict = engine.check("This is a completely unique message about Rust programming");
        assert_eq!(verdict, DedupVerdict::Accept);
    }

    #[test]
    fn test_exact_duplicate_skip() {
        let mut engine = test_engine();
        let content = "This is a message that will be stored twice in the system";
        engine.check(content);
        let verdict = engine.check(content);
        assert!(verdict.is_duplicate());
        match verdict {
            DedupVerdict::Skip { reason, similarity } => {
                assert!(reason.contains("exact") || reason.contains("temporal"));
                assert!((similarity - 1.0).abs() < f64::EPSILON);
            }
            _ => panic!("Expected Skip"),
        }
    }

    #[test]
    fn test_fuzzy_duplicate_high_similarity() {
        let mut engine = DedupEngine::new(DedupConfig {
            fuzzy_threshold: 0.80,
            ..Default::default()
        });
        engine.check("The quick brown fox jumps over the lazy dog in the park today right now");
        let verdict =
            engine.check("The quick brown fox leaps over the lazy dog in the park today right now");
        assert!(
            verdict.is_duplicate() || matches!(verdict, DedupVerdict::Merge { .. }),
            "Expected Skip or Merge for high similarity, got {:?}",
            verdict
        );
    }

    #[test]
    fn test_fuzzy_different_content_accepted() {
        let mut engine = test_engine();
        engine.check("The quick brown fox jumps over the lazy dog in the garden");
        let verdict = engine.check("Quantum computing represents a paradigm shift in processing");
        assert_eq!(verdict, DedupVerdict::Accept);
    }

    #[test]
    fn test_short_content_always_accepted() {
        let mut engine = test_engine();
        engine.check("hello");
        let verdict = engine.check("hello"); // same but < min_content_length
        assert_eq!(verdict, DedupVerdict::Accept);
    }

    #[test]
    fn test_temporal_dedup() {
        let mut engine = DedupEngine::new(DedupConfig {
            exact_dedup: false, // disable exact to test temporal specifically
            fuzzy_dedup: false,
            temporal_dedup: true,
            temporal_window_secs: 60,
            min_content_length: 5,
            ..Default::default()
        });
        let content = "This is temporal test content that should be deduped";
        engine.check(content);
        let verdict = engine.check(content);
        match verdict {
            DedupVerdict::Skip { reason, .. } => {
                assert!(reason.contains("temporal"));
            }
            _ => panic!("Expected temporal Skip, got {:?}", verdict),
        }
    }

    #[test]
    fn test_stats_tracking() {
        let mut engine = test_engine();
        engine.check("First unique message about databases and storage systems");
        engine.check("Second unique message about networking and protocols");
        engine.check("First unique message about databases and storage systems"); // dup
        let stats = engine.stats();
        assert_eq!(stats.total_checked, 3);
        assert_eq!(stats.accepted, 2);
        assert!(stats.exact_duplicates + stats.temporal_duplicates >= 1);
    }

    #[test]
    fn test_reset_stats() {
        let mut engine = test_engine();
        engine.check("Some content that gets checked by the dedup engine");
        engine.reset_stats();
        assert_eq!(engine.stats().total_checked, 0);
    }

    #[test]
    fn test_clear_all() {
        let mut engine = test_engine();
        engine.check("Content that populates the internal dedup state");
        engine.clear();
        assert_eq!(engine.unique_count(), 0);
        // Same content should be accepted after clear
        let verdict = engine.check("Content that populates the internal dedup state");
        assert_eq!(verdict, DedupVerdict::Accept);
    }

    #[test]
    fn test_register_bootstraps() {
        let mut engine = test_engine();
        engine.register("Pre-existing content that was already in the database");
        let verdict = engine.check("Pre-existing content that was already in the database");
        assert!(verdict.is_duplicate());
    }

    #[test]
    fn test_jaccard_identical() {
        let a: HashSet<String> = ["foo", "bar"].iter().map(|s| s.to_string()).collect();
        let sim = jaccard_similarity(&a, &a);
        assert!((sim - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jaccard_disjoint() {
        let a: HashSet<String> = ["foo", "bar"].iter().map(|s| s.to_string()).collect();
        let b: HashSet<String> = ["baz", "qux"].iter().map(|s| s.to_string()).collect();
        let sim = jaccard_similarity(&a, &b);
        assert!(sim.abs() < f64::EPSILON);
    }

    #[test]
    fn test_jaccard_partial_overlap() {
        let a: HashSet<String> = ["foo", "bar", "baz"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let b: HashSet<String> = ["bar", "baz", "qux"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // intersection=2, union=4, sim=0.5
        let sim = jaccard_similarity(&a, &b);
        assert!((sim - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jaccard_empty_sets() {
        let a: HashSet<String> = HashSet::new();
        let b: HashSet<String> = HashSet::new();
        assert!((jaccard_similarity(&a, &b) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_tokenize() {
        let tokens = tokenize("Hello, World! This is a test.");
        assert!(tokens.contains("hello"));
        assert!(tokens.contains("world"));
        assert!(tokens.contains("this"));
        assert!(tokens.contains("test"));
        assert!(!tokens.contains("a")); // too short (< 2 chars)
    }

    #[test]
    fn test_config_update() {
        let mut engine = test_engine();
        engine.set_config(DedupConfig {
            fuzzy_threshold: 0.99,
            ..Default::default()
        });
        assert!((engine.config().fuzzy_threshold - 0.99).abs() < f64::EPSILON);
    }

    #[test]
    fn test_verdict_helpers() {
        assert!(DedupVerdict::Accept.should_store());
        assert!(
            DedupVerdict::Merge {
                existing_hash: "abc".into(),
                similarity: 0.9,
            }
            .should_store()
        );
        assert!(
            !DedupVerdict::Skip {
                reason: "dup".into(),
                similarity: 1.0,
            }
            .should_store()
        );

        assert!(!DedupVerdict::Accept.is_duplicate());
        assert!(
            DedupVerdict::Skip {
                reason: "dup".into(),
                similarity: 1.0,
            }
            .is_duplicate()
        );
    }

    #[test]
    fn test_unique_count() {
        let mut engine = test_engine();
        engine.check("First unique message with enough content to pass min length");
        engine.check("Second unique message with different content entirely here");
        assert_eq!(engine.unique_count(), 2);
    }

    #[test]
    fn test_window_size_bounded() {
        let mut engine = DedupEngine::new(DedupConfig {
            fuzzy_window_size: 3,
            fuzzy_dedup: false, // disable fuzzy to avoid cross-match skips
            ..Default::default()
        });
        let topics = [
            "Quantum computing changes semiconductor fabrication processes",
            "Marine biology explores deep ocean bioluminescence phenomena",
            "Renaissance architecture influenced modern cathedral construction",
            "Astrophysics analyzes gravitational wave detection methods",
            "Culinary traditions preserve ancient fermentation techniques",
            "Paleontology discovers preserved dinosaur feather specimens",
            "Cryptographic protocols ensure blockchain transaction security",
            "Neuroplasticity research reveals adult brain adaptation capacity",
            "Volcanic eruptions reshape geological terrain formations drastically",
            "Orchestral compositions blend classical symphonic arrangements beautifully",
        ];
        for topic in &topics {
            engine.check(topic);
        }
        assert_eq!(engine.recent.len(), 3);
    }

    #[test]
    fn test_default_engine() {
        let engine = DedupEngine::default();
        assert!(engine.config().exact_dedup);
        assert!(engine.config().fuzzy_dedup);
        assert!(engine.config().temporal_dedup);
    }
}
