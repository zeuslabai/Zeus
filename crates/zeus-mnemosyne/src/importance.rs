//! Memory Importance Scoring — Rank memories for context injection
//!
//! Assigns importance scores (0.0–1.0) to memories based on multiple
//! signals, enabling the Prometheus memory injector to select the most
//! relevant context within token budgets.
//!
//! Scoring signals:
//! - **Recency**: newer memories score higher (exponential decay)
//! - **Frequency**: memories accessed more often are more important
//! - **Relevance**: keyword overlap with current query
//! - **Type boost**: certain memory types (facts, decisions) score higher
//! - **Explicit pin**: manually pinned memories always rank highest

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tracing::debug;

// ============================================================================
// Configuration
// ============================================================================

/// Importance scoring configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportanceConfig {
    /// Weight for recency signal (0.0–1.0)
    pub recency_weight: f64,
    /// Weight for frequency signal (0.0–1.0)
    pub frequency_weight: f64,
    /// Weight for relevance signal (0.0–1.0)
    pub relevance_weight: f64,
    /// Weight for type boost signal (0.0–1.0)
    pub type_weight: f64,
    /// Recency half-life in hours (score halves every N hours)
    pub recency_half_life_hours: f64,
    /// Maximum access count for frequency normalization
    pub max_access_count: u32,
    /// Type boost values
    pub type_boosts: HashMap<String, f64>,
    /// Score threshold below which memories are excluded
    pub min_score: f64,
}

impl Default for ImportanceConfig {
    fn default() -> Self {
        let mut type_boosts = HashMap::new();
        type_boosts.insert("fact".into(), 0.9);
        type_boosts.insert("decision".into(), 0.95);
        type_boosts.insert("preference".into(), 0.85);
        type_boosts.insert("context".into(), 0.6);
        type_boosts.insert("message".into(), 0.4);
        type_boosts.insert("observation".into(), 0.5);

        Self {
            recency_weight: 0.3,
            frequency_weight: 0.2,
            relevance_weight: 0.35,
            type_weight: 0.15,
            recency_half_life_hours: 24.0,
            max_access_count: 50,
            min_score: 0.1,
            type_boosts,
        }
    }
}

// ============================================================================
// Memory Entry (for scoring)
// ============================================================================

/// A memory entry with metadata for scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Unique ID
    pub id: String,
    /// Content text
    pub content: String,
    /// Memory type (fact, decision, preference, context, message)
    pub memory_type: String,
    /// When this memory was created
    pub created_at: DateTime<Utc>,
    /// When this memory was last accessed
    pub last_accessed: DateTime<Utc>,
    /// Number of times accessed
    pub access_count: u32,
    /// Whether this memory is pinned (always highest priority)
    pub pinned: bool,
}

// ============================================================================
// Scored Memory
// ============================================================================

/// A memory with its computed importance score
#[derive(Debug, Clone)]
pub struct ScoredMemory {
    pub entry: MemoryEntry,
    pub score: f64,
    /// Breakdown of score components
    pub breakdown: ScoreBreakdown,
}

/// Individual signal contributions to the final score
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    pub recency: f64,
    pub frequency: f64,
    pub relevance: f64,
    pub type_boost: f64,
    pub pinned: bool,
}

// ============================================================================
// Importance Scorer
// ============================================================================

/// Scores and ranks memories by importance
pub struct ImportanceScorer {
    config: ImportanceConfig,
    stats: ScorerStats,
}

/// Scorer statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScorerStats {
    pub total_scored: u64,
    pub total_filtered: u64,
    pub total_pinned_found: u64,
}

impl ImportanceScorer {
    pub fn new(config: ImportanceConfig) -> Self {
        Self {
            config,
            stats: ScorerStats::default(),
        }
    }

    /// Score a single memory entry, optionally with a relevance query
    pub fn score(&mut self, entry: &MemoryEntry, query: Option<&str>) -> ScoredMemory {
        self.stats.total_scored += 1;

        if entry.pinned {
            self.stats.total_pinned_found += 1;
            return ScoredMemory {
                entry: entry.clone(),
                score: 1.0,
                breakdown: ScoreBreakdown {
                    pinned: true,
                    ..Default::default()
                },
            };
        }

        let recency = self.compute_recency(entry);
        let frequency = self.compute_frequency(entry);
        let relevance = query.map_or(0.5, |q| compute_relevance(&entry.content, q));
        let type_boost = self.compute_type_boost(&entry.memory_type);

        let score = self.config.recency_weight * recency
            + self.config.frequency_weight * frequency
            + self.config.relevance_weight * relevance
            + self.config.type_weight * type_boost;

        // Clamp to 0.0–1.0
        let score = score.clamp(0.0, 1.0);

        ScoredMemory {
            entry: entry.clone(),
            score,
            breakdown: ScoreBreakdown {
                recency,
                frequency,
                relevance,
                type_boost,
                pinned: false,
            },
        }
    }

    /// Score and rank a batch of memories, filtering by min_score
    pub fn rank(
        &mut self,
        entries: &[MemoryEntry],
        query: Option<&str>,
        limit: usize,
    ) -> Vec<ScoredMemory> {
        // Score all entries first (avoids borrow conflicts in chained closures)
        let all_scored: Vec<ScoredMemory> = entries.iter().map(|e| self.score(e, query)).collect();

        let min_score = self.config.min_score;
        let mut scored: Vec<ScoredMemory> = all_scored
            .into_iter()
            .filter(|s| {
                if s.score < min_score && !s.breakdown.pinned {
                    self.stats.total_filtered += 1;
                    false
                } else {
                    true
                }
            })
            .collect();

        // Sort by score descending (pinned first, then by score)
        scored.sort_by(|a, b| {
            b.breakdown.pinned.cmp(&a.breakdown.pinned).then(
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });

        scored.truncate(limit);

        debug!(
            total = entries.len(),
            returned = scored.len(),
            "Ranked memories by importance"
        );

        scored
    }

    /// Compute recency score using exponential decay
    fn compute_recency(&self, entry: &MemoryEntry) -> f64 {
        let now = Utc::now();
        let age_hours = (now - entry.last_accessed).num_minutes() as f64 / 60.0;
        if age_hours <= 0.0 {
            return 1.0;
        }
        let half_life = self.config.recency_half_life_hours;
        // Exponential decay: score = 0.5^(age/half_life)
        (0.5_f64).powf(age_hours / half_life)
    }

    /// Compute frequency score (normalized access count)
    fn compute_frequency(&self, entry: &MemoryEntry) -> f64 {
        let max = self.config.max_access_count as f64;
        (entry.access_count as f64 / max).min(1.0)
    }

    /// Compute type boost
    fn compute_type_boost(&self, memory_type: &str) -> f64 {
        self.config
            .type_boosts
            .get(memory_type)
            .copied()
            .unwrap_or(0.5)
    }

    /// Get statistics
    pub fn stats(&self) -> &ScorerStats {
        &self.stats
    }

    /// Reset stats
    pub fn reset_stats(&mut self) {
        self.stats = ScorerStats::default();
    }

    /// Get config
    pub fn config(&self) -> &ImportanceConfig {
        &self.config
    }

    /// Update config
    pub fn set_config(&mut self, config: ImportanceConfig) {
        self.config = config;
    }
}

impl Default for ImportanceScorer {
    fn default() -> Self {
        Self::new(ImportanceConfig::default())
    }
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Compute keyword relevance between content and query (Jaccard on tokens)
fn compute_relevance(content: &str, query: &str) -> f64 {
    let content_tokens = tokenize(content);
    let query_tokens = tokenize(query);

    if query_tokens.is_empty() {
        return 0.5;
    }

    let intersection = content_tokens.intersection(&query_tokens).count();
    let union = content_tokens.union(&query_tokens).count();

    if union == 0 {
        return 0.0;
    }

    intersection as f64 / union as f64
}

/// Tokenize text into lowercase word tokens (min 2 chars)
fn tokenize(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_lowercase())
        .collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_entry(
        id: &str,
        content: &str,
        memory_type: &str,
        hours_ago: i64,
        access_count: u32,
    ) -> MemoryEntry {
        let now = Utc::now();
        MemoryEntry {
            id: id.to_string(),
            content: content.to_string(),
            memory_type: memory_type.to_string(),
            created_at: now - Duration::hours(hours_ago),
            last_accessed: now - Duration::hours(hours_ago),
            access_count,
            pinned: false,
        }
    }

    fn make_pinned(id: &str, content: &str) -> MemoryEntry {
        MemoryEntry {
            id: id.to_string(),
            content: content.to_string(),
            memory_type: "fact".to_string(),
            created_at: Utc::now(),
            last_accessed: Utc::now(),
            access_count: 0,
            pinned: true,
        }
    }

    #[test]
    fn test_pinned_always_max_score() {
        let mut scorer = ImportanceScorer::default();
        let entry = make_pinned("p1", "Always important");
        let scored = scorer.score(&entry, None);
        assert!((scored.score - 1.0).abs() < f64::EPSILON);
        assert!(scored.breakdown.pinned);
    }

    #[test]
    fn test_recent_scores_higher_than_old() {
        let mut scorer = ImportanceScorer::default();
        let recent = make_entry("r", "Recent memory content here", "fact", 1, 5);
        let old = make_entry("o", "Old memory content here today", "fact", 72, 5);

        let score_recent = scorer.score(&recent, None).score;
        let score_old = scorer.score(&old, None).score;
        assert!(
            score_recent > score_old,
            "Recent ({}) should score higher than old ({})",
            score_recent,
            score_old
        );
    }

    #[test]
    fn test_frequent_scores_higher() {
        let mut scorer = ImportanceScorer::default();
        let frequent = make_entry("f", "Frequently accessed memory", "context", 12, 40);
        let rare = make_entry("r", "Rarely accessed memory here", "context", 12, 1);

        let score_freq = scorer.score(&frequent, None).score;
        let score_rare = scorer.score(&rare, None).score;
        assert!(score_freq > score_rare);
    }

    #[test]
    fn test_relevance_boosts_score() {
        let mut scorer = ImportanceScorer::default();
        let entry = make_entry(
            "e1",
            "Rust programming language compiler optimizations",
            "fact",
            12,
            5,
        );

        // Query overlaps 4 of 5 tokens → Jaccard = 4/6 = 0.67 > default 0.5
        let with_query = scorer
            .score(&entry, Some("Rust programming language compiler"))
            .score;
        let without_query = scorer.score(&entry, None).score;
        assert!(
            with_query > without_query,
            "Relevant query ({}) should boost score vs no query ({})",
            with_query,
            without_query
        );
    }

    #[test]
    fn test_type_boost_decision_higher_than_message() {
        let mut scorer = ImportanceScorer::default();
        let decision = make_entry("d", "We decided to use PostgreSQL", "decision", 12, 5);
        let message = make_entry("m", "We decided to use PostgreSQL", "message", 12, 5);

        let score_decision = scorer.score(&decision, None).score;
        let score_message = scorer.score(&message, None).score;
        assert!(score_decision > score_message);
    }

    #[test]
    fn test_rank_returns_sorted() {
        let mut scorer = ImportanceScorer::default();
        let entries = vec![
            make_entry(
                "old",
                "Old memory about databases and storage",
                "context",
                48,
                1,
            ),
            make_entry(
                "new",
                "New memory about databases and storage",
                "fact",
                1,
                10,
            ),
            make_pinned("pin", "Pinned critical information always"),
        ];

        let ranked = scorer.rank(&entries, None, 10);
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].entry.id, "pin"); // pinned first
        assert!(ranked[0].score >= ranked[1].score);
        assert!(ranked[1].score >= ranked[2].score);
    }

    #[test]
    fn test_rank_respects_limit() {
        let mut scorer = ImportanceScorer::default();
        let entries: Vec<MemoryEntry> = (0..10)
            .map(|i| {
                make_entry(
                    &format!("e{}", i),
                    &format!("Memory number {} about various topics", i),
                    "fact",
                    i as i64,
                    5,
                )
            })
            .collect();

        let ranked = scorer.rank(&entries, None, 3);
        assert_eq!(ranked.len(), 3);
    }

    #[test]
    fn test_rank_filters_low_scores() {
        let mut scorer = ImportanceScorer::new(ImportanceConfig {
            min_score: 0.5,
            ..Default::default()
        });
        let entries = vec![
            make_entry("good", "Important recent fact about systems", "fact", 1, 20),
            make_entry(
                "bad",
                "Trivial old observation about nothing",
                "observation",
                200,
                0,
            ),
        ];

        let ranked = scorer.rank(&entries, None, 10);
        // The old low-access observation may be filtered
        assert!(ranked.len() >= 1);
        assert!(ranked[0].score >= 0.5);
    }

    #[test]
    fn test_compute_relevance_identical() {
        let score = compute_relevance("hello world", "hello world");
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_relevance_disjoint() {
        let score = compute_relevance("alpha beta gamma", "delta epsilon zeta");
        assert!(score.abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_relevance_partial() {
        let score = compute_relevance("alpha beta gamma", "beta gamma delta");
        // intersection=2, union=4, score=0.5
        assert!((score - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_relevance_empty_query() {
        let score = compute_relevance("some content here", "");
        assert!((score - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_recency_decay() {
        let mut scorer = ImportanceScorer::new(ImportanceConfig {
            recency_half_life_hours: 24.0,
            ..Default::default()
        });
        // Entry exactly 24 hours old should have ~0.5 recency score
        let entry = make_entry("e", "Some content here today", "fact", 24, 0);
        let scored = scorer.score(&entry, None);
        assert!(
            (scored.breakdown.recency - 0.5).abs() < 0.05,
            "24h old entry recency should be ~0.5, got {}",
            scored.breakdown.recency
        );
    }

    #[test]
    fn test_frequency_normalization() {
        let mut scorer = ImportanceScorer::new(ImportanceConfig {
            max_access_count: 100,
            ..Default::default()
        });
        let entry = make_entry("e", "Content here for test", "fact", 0, 50);
        let scored = scorer.score(&entry, None);
        assert!((scored.breakdown.frequency - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_frequency_capped_at_one() {
        let mut scorer = ImportanceScorer::new(ImportanceConfig {
            max_access_count: 10,
            ..Default::default()
        });
        let entry = make_entry("e", "Content here for test", "fact", 0, 100);
        let scored = scorer.score(&entry, None);
        assert!((scored.breakdown.frequency - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_unknown_type_gets_default_boost() {
        let mut scorer = ImportanceScorer::default();
        let entry = make_entry(
            "e",
            "Content with unknown type category",
            "unknown_type",
            0,
            5,
        );
        let scored = scorer.score(&entry, None);
        assert!((scored.breakdown.type_boost - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_stats_tracking() {
        let mut scorer = ImportanceScorer::default();
        let entry = make_entry("e", "Content for stats tracking test", "fact", 0, 5);
        scorer.score(&entry, None);
        scorer.score(&entry, None);
        assert_eq!(scorer.stats().total_scored, 2);
    }

    #[test]
    fn test_reset_stats() {
        let mut scorer = ImportanceScorer::default();
        let entry = make_entry("e", "Content for reset test here", "fact", 0, 5);
        scorer.score(&entry, None);
        scorer.reset_stats();
        assert_eq!(scorer.stats().total_scored, 0);
    }

    #[test]
    fn test_config_update() {
        let mut scorer = ImportanceScorer::default();
        scorer.set_config(ImportanceConfig {
            recency_weight: 0.5,
            ..Default::default()
        });
        assert!((scorer.config().recency_weight - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_default_scorer() {
        let scorer = ImportanceScorer::default();
        let w = scorer.config();
        let total = w.recency_weight + w.frequency_weight + w.relevance_weight + w.type_weight;
        assert!(
            (total - 1.0).abs() < f64::EPSILON,
            "Weights should sum to 1.0, got {}",
            total
        );
    }
}
