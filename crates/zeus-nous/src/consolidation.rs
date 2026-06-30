//! Consolidation Engine - Background memory pattern extraction
//!
//! Runs periodically to:
//! 1. Decay importance of episodic memories over time
//! 2. Extract patterns from frequently occurring topics/entities
//! 3. Promote high-confidence lessons to semantic memory
//! 4. Clean up low-importance old episodic memories

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

// ============================================================================
// Types
// ============================================================================

/// Result of a consolidation cycle
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsolidationResult {
    /// Number of patterns found in episodic memory
    pub patterns_found: usize,
    /// Number of memories promoted from episodic to semantic
    pub memories_promoted: usize,
    /// Number of memories that had their importance decayed
    pub memories_decayed: usize,
    /// Number of importance scores updated
    pub importance_updates: usize,
    /// Number of memories eligible for cleanup
    pub cleanup_eligible: usize,
    /// When this consolidation ran
    pub timestamp: DateTime<Utc>,
}

/// A pattern extracted from episodic memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedPattern {
    /// Topic or entity name
    pub topic: String,
    /// Number of occurrences in episodic memory
    pub frequency: usize,
    /// Suggested semantic summary
    pub summary: String,
}

// ============================================================================
// ConsolidationEngine
// ============================================================================

/// Background consolidation engine for memory hierarchy management
pub struct ConsolidationEngine {
    /// How often consolidation runs (in seconds)
    interval_secs: u64,
    /// Importance decay rate per cycle (fraction subtracted each cycle)
    decay_rate: f32,
    /// Minimum importance before an episodic memory is eligible for cleanup
    cleanup_threshold: f32,
    /// Minimum days before an old memory is cleaned up
    cleanup_age_days: i64,
    /// Minimum frequency for a keyword to be considered a pattern
    pattern_min_frequency: usize,
}

impl ConsolidationEngine {
    /// Create a new consolidation engine
    pub fn new(interval_secs: u64) -> Self {
        Self {
            interval_secs,
            decay_rate: 0.05,
            cleanup_threshold: 0.1,
            cleanup_age_days: 30,
            pattern_min_frequency: 3,
        }
    }

    /// Get the configured interval in seconds
    pub fn interval_secs(&self) -> u64 {
        self.interval_secs
    }

    /// Extract keyword patterns from a set of memory contents.
    /// Returns patterns that appear at least `pattern_min_frequency` times.
    pub fn extract_patterns(&self, contents: &[String]) -> Vec<ExtractedPattern> {
        let mut word_counts: HashMap<String, usize> = HashMap::new();

        for content in contents {
            // Extract meaningful words (>3 chars, lowercase, deduplicate per message)
            let words: std::collections::HashSet<String> = content
                .split_whitespace()
                .map(|w| {
                    w.trim_matches(|c: char| !c.is_alphanumeric())
                        .to_lowercase()
                })
                .filter(|w| w.len() > 3)
                .collect();

            for word in words {
                *word_counts.entry(word).or_insert(0) += 1;
            }
        }

        // Filter to frequent patterns and sort by frequency
        let mut patterns: Vec<ExtractedPattern> = word_counts
            .into_iter()
            .filter(|(_, count)| *count >= self.pattern_min_frequency)
            .map(|(topic, frequency)| ExtractedPattern {
                summary: format!("Recurring topic: '{}' ({} mentions)", topic, frequency),
                topic,
                frequency,
            })
            .collect();

        patterns.sort_by(|a, b| b.frequency.cmp(&a.frequency));
        patterns
    }

    /// Compute decayed importance values for a set of memories.
    /// Returns a list of (index, new_importance) pairs for memories whose
    /// importance decreased.
    pub fn decay_importance(&self, importances: &[f32]) -> Vec<(usize, f32)> {
        importances
            .iter()
            .enumerate()
            .filter_map(|(i, &imp)| {
                let new_imp = (imp - self.decay_rate).max(0.0);
                if (new_imp - imp).abs() > f32::EPSILON {
                    Some((i, new_imp))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Determine which memories should be promoted to semantic based on
    /// high lesson confidence. Returns indices of lessons that should be promoted.
    pub fn select_for_promotion(&self, lesson_confidences: &[f32]) -> Vec<usize> {
        lesson_confidences
            .iter()
            .enumerate()
            .filter(|(_, conf)| **conf >= 0.8)
            .map(|(i, _)| i)
            .collect()
    }

    /// Determine which memories should be cleaned up based on low importance
    /// and age. Returns indices eligible for cleanup.
    pub fn select_for_cleanup(&self, importances: &[f32], ages_days: &[i64]) -> Vec<usize> {
        importances
            .iter()
            .zip(ages_days.iter())
            .enumerate()
            .filter(|(_, (imp, age))| {
                **imp < self.cleanup_threshold && **age > self.cleanup_age_days
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Run a full consolidation cycle against provided data.
    /// This is a pure function that doesn't touch the database directly —
    /// the caller is responsible for applying the results.
    pub fn consolidate(
        &self,
        episodic_contents: &[String],
        importances: &[f32],
        ages_days: &[i64],
        lesson_confidences: &[f32],
    ) -> ConsolidationResult {
        let patterns = self.extract_patterns(episodic_contents);
        let decayed = self.decay_importance(importances);
        let promoted = self.select_for_promotion(lesson_confidences);
        let cleanup = self.select_for_cleanup(importances, ages_days);

        let result = ConsolidationResult {
            patterns_found: patterns.len(),
            memories_promoted: promoted.len(),
            memories_decayed: decayed.len(),
            importance_updates: decayed.len(),
            cleanup_eligible: cleanup.len(),
            timestamp: Utc::now(),
        };

        info!(
            patterns = result.patterns_found,
            promoted = result.memories_promoted,
            decayed = result.memories_decayed,
            cleanup = result.cleanup_eligible,
            "Consolidation cycle complete"
        );

        debug!(
            "Top patterns: {:?}",
            patterns
                .iter()
                .take(5)
                .map(|p| &p.topic)
                .collect::<Vec<_>>()
        );

        result
    }
}

impl Default for ConsolidationEngine {
    fn default() -> Self {
        Self::new(900) // 15 minutes
    }
}

// ============================================================================
// Background Runner
// ============================================================================

/// Trait for providing memory data to the consolidation engine.
/// This abstracts the database access so the engine remains testable.
pub trait ConsolidationDataProvider: Send + Sync {
    /// Get episodic memory contents for pattern extraction
    fn episodic_contents(&self) -> Vec<String>;
    /// Get importance values for all episodic memories
    fn episodic_importances(&self) -> Vec<f32>;
    /// Get ages in days for all episodic memories
    fn episodic_ages_days(&self) -> Vec<i64>;
    /// Get lesson confidence values
    fn lesson_confidences(&self) -> Vec<f32>;
    /// Apply decay to episodic memories
    fn apply_decay(&self, decay_rate: f32) -> usize;
    /// Delete memories at the given indices (cleanup)
    fn cleanup_memories(&self, indices: &[usize]) -> usize;
}

impl ConsolidationEngine {
    /// Run the consolidation engine as a background task.
    ///
    /// Periodically calls `consolidate()` using data from the provider,
    /// then applies decay and cleanup. Stops when the shutdown signal fires.
    pub async fn run_background(
        self: Arc<Self>,
        provider: Arc<dyn ConsolidationDataProvider>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        info!(
            interval_secs = self.interval_secs,
            "Consolidation background task started"
        );

        loop {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(self.interval_secs)) => {
                    // Gather data from provider
                    let contents = provider.episodic_contents();
                    let importances = provider.episodic_importances();
                    let ages = provider.episodic_ages_days();
                    let confidences = provider.lesson_confidences();

                    // Run consolidation
                    let result = self.consolidate(&contents, &importances, &ages, &confidences);

                    // Apply decay
                    let decayed = provider.apply_decay(self.decay_rate);
                    debug!(decayed, "Applied importance decay");

                    // Cleanup eligible memories
                    if result.cleanup_eligible > 0 {
                        let cleanup_indices = self.select_for_cleanup(&importances, &ages);
                        let cleaned = provider.cleanup_memories(&cleanup_indices);
                        debug!(cleaned, eligible = result.cleanup_eligible, "Cleaned up old memories");
                    }
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("Consolidation background task shutting down");
                        break;
                    }
                }
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_consolidation_engine_creation() {
        let engine = ConsolidationEngine::new(600);
        assert_eq!(engine.interval_secs(), 600);
    }

    #[test]
    fn test_extract_patterns() {
        let engine = ConsolidationEngine::new(900);
        let contents = vec![
            "Working on the Rust project today".to_string(),
            "Rust programming language features".to_string(),
            "Building with Rust and cargo".to_string(),
            "Python data science work".to_string(),
            "Rust compile times improving".to_string(),
        ];

        let patterns = engine.extract_patterns(&contents);
        // "rust" should appear 4 times (above min_frequency of 3)
        assert!(patterns.iter().any(|p| p.topic == "rust"));
        let rust_pat = patterns
            .iter()
            .find(|p| p.topic == "rust")
            .expect("find should succeed");
        assert_eq!(rust_pat.frequency, 4);
    }

    #[test]
    fn test_extract_patterns_empty() {
        let engine = ConsolidationEngine::new(900);
        let patterns = engine.extract_patterns(&[]);
        assert!(patterns.is_empty());
    }

    #[test]
    fn test_extract_patterns_below_threshold() {
        let engine = ConsolidationEngine::new(900);
        let contents = vec![
            "One unique sentence".to_string(),
            "Another different topic".to_string(),
        ];
        let patterns = engine.extract_patterns(&contents);
        // No word appears 3+ times
        assert!(patterns.is_empty());
    }

    #[test]
    fn test_decay_importance() {
        let engine = ConsolidationEngine::new(900);
        let importances = vec![1.0, 0.5, 0.1, 0.03, 0.0];

        let decayed = engine.decay_importance(&importances);
        // All except 0.0 should decay
        assert!(decayed.len() >= 4);

        // Check values decreased
        for (idx, new_val) in &decayed {
            assert!(*new_val < importances[*idx]);
            assert!(*new_val >= 0.0);
        }
    }

    #[test]
    fn test_decay_importance_floor() {
        let engine = ConsolidationEngine::new(900);
        let importances = vec![0.02]; // Less than decay_rate (0.05)

        let decayed = engine.decay_importance(&importances);
        assert_eq!(decayed.len(), 1);
        assert_eq!(decayed[0].1, 0.0); // Should floor at 0.0
    }

    #[test]
    fn test_select_for_promotion() {
        let engine = ConsolidationEngine::new(900);
        let confidences = vec![0.3, 0.5, 0.8, 0.95, 0.6];

        let promoted = engine.select_for_promotion(&confidences);
        assert_eq!(promoted, vec![2, 3]); // indices with conf >= 0.8
    }

    #[test]
    fn test_select_for_cleanup() {
        let engine = ConsolidationEngine::new(900);
        let importances = vec![0.5, 0.05, 0.02, 0.8, 0.01];
        let ages = vec![10, 40, 60, 5, 35];

        let cleanup = engine.select_for_cleanup(&importances, &ages);
        // Only indices where importance < 0.1 AND age > 30
        assert!(cleanup.contains(&1)); // imp=0.05, age=40
        assert!(cleanup.contains(&2)); // imp=0.02, age=60
        assert!(cleanup.contains(&4)); // imp=0.01, age=35
        assert!(!cleanup.contains(&0)); // imp too high
        assert!(!cleanup.contains(&3)); // imp too high and age too low
    }

    #[test]
    fn test_full_consolidation() {
        let engine = ConsolidationEngine::new(900);
        let contents = vec![
            "Working on Rust".to_string(),
            "Rust programming".to_string(),
            "More Rust work".to_string(),
        ];
        let importances = vec![0.8, 0.5, 0.2];
        let ages = vec![10, 20, 5];
        let confidences = vec![0.9, 0.3, 0.85];

        let result = engine.consolidate(&contents, &importances, &ages, &confidences);
        assert!(result.patterns_found >= 1); // "rust" appears 3 times
        assert_eq!(result.memories_promoted, 2); // confidences 0.9 and 0.85
        assert!(result.memories_decayed > 0);
        assert_eq!(result.cleanup_eligible, 0); // No old low-importance memories
    }

    #[test]
    fn test_consolidation_with_cleanup() {
        let engine = ConsolidationEngine::new(900);
        let contents = vec![
            "Old forgotten thing".to_string(),
            "Another old memory".to_string(),
            "Recent important work".to_string(),
        ];
        let importances = vec![0.05, 0.03, 0.8]; // first two below cleanup threshold
        let ages = vec![60, 45, 5]; // first two older than 30 days
        let confidences = vec![0.1, 0.2, 0.3];

        let result = engine.consolidate(&contents, &importances, &ages, &confidences);
        assert_eq!(result.cleanup_eligible, 2); // Two old low-importance memories
        assert_eq!(result.memories_promoted, 0); // No high-confidence lessons
    }

    #[test]
    fn test_extract_patterns_single_word() {
        let engine = ConsolidationEngine::new(900);
        // Single-word contents; each word appears in its own message
        // Need 3+ appearances to qualify as a pattern
        let contents = vec![
            "deploy the application".to_string(),
            "deploy to production".to_string(),
            "deploy the service now".to_string(),
        ];

        let patterns = engine.extract_patterns(&contents);
        assert!(patterns.iter().any(|p| p.topic == "deploy"));
        let deploy_pat = patterns
            .iter()
            .find(|p| p.topic == "deploy")
            .expect("find should succeed");
        assert_eq!(deploy_pat.frequency, 3);
    }

    #[test]
    fn test_extract_patterns_stop_words_filtered() {
        let engine = ConsolidationEngine::new(900);
        // Common short words (<=3 chars) are filtered out by the len > 3 check
        let contents = vec![
            "the cat sat on the mat".to_string(),
            "the dog ran to the park".to_string(),
            "the fox jumped over the fence".to_string(),
            "the bird flew in the sky".to_string(),
        ];

        let patterns = engine.extract_patterns(&contents);
        // "the" is only 3 chars, so it should be filtered out
        assert!(!patterns.iter().any(|p| p.topic == "the"));
    }

    #[test]
    fn test_decay_importance_multiple_cycles() {
        let engine = ConsolidationEngine::new(900);
        let mut importances = vec![1.0];

        for _ in 0..5 {
            let decayed = engine.decay_importance(&importances);
            if let Some((_, new_val)) = decayed.first() {
                importances[0] = *new_val;
            }
        }

        // After 5 decays of 0.05 each: 1.0 - 5*0.05 = 0.75
        assert!((importances[0] - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn test_select_for_promotion_none_qualify() {
        let engine = ConsolidationEngine::new(900);
        let confidences = vec![0.1, 0.3, 0.5, 0.7, 0.79];

        let promoted = engine.select_for_promotion(&confidences);
        // None are >= 0.8
        assert!(promoted.is_empty());
    }

    #[test]
    fn test_select_for_cleanup_young_items() {
        let engine = ConsolidationEngine::new(900);
        // All items have low importance but are young (< 30 days)
        let importances = vec![0.01, 0.02, 0.05];
        let ages = vec![5, 10, 29];

        let cleanup = engine.select_for_cleanup(&importances, &ages);
        // None should be cleaned up because all ages <= 30
        assert!(cleanup.is_empty());
    }

    #[test]
    fn test_consolidation_engine_config() {
        let engine = ConsolidationEngine::default();
        assert_eq!(engine.interval_secs(), 900); // 15 minutes default

        let custom = ConsolidationEngine::new(60);
        assert_eq!(custom.interval_secs(), 60);

        // Verify internal defaults via behavior
        // decay_rate = 0.05: decaying 0.5 should yield 0.45
        let decayed = custom.decay_importance(&[0.5]);
        assert_eq!(decayed.len(), 1);
        assert!((decayed[0].1 - 0.45).abs() < f32::EPSILON);

        // pattern_min_frequency = 3: two occurrences should not produce patterns
        let contents = vec!["rust code".to_string(), "rust project".to_string()];
        let patterns = custom.extract_patterns(&contents);
        assert!(patterns.is_empty());
    }

    #[tokio::test]
    async fn test_run_background_shutdown() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct MockProvider {
            cycle_count: AtomicUsize,
        }

        impl ConsolidationDataProvider for MockProvider {
            fn episodic_contents(&self) -> Vec<String> {
                self.cycle_count.fetch_add(1, Ordering::SeqCst);
                vec!["test content".to_string()]
            }
            fn episodic_importances(&self) -> Vec<f32> {
                vec![0.5]
            }
            fn episodic_ages_days(&self) -> Vec<i64> {
                vec![10]
            }
            fn lesson_confidences(&self) -> Vec<f32> {
                vec![0.3]
            }
            fn apply_decay(&self, _rate: f32) -> usize {
                1
            }
            fn cleanup_memories(&self, _indices: &[usize]) -> usize {
                0
            }
        }

        let engine = Arc::new(ConsolidationEngine::new(1)); // 1-second interval
        let provider = Arc::new(MockProvider {
            cycle_count: AtomicUsize::new(0),
        });
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let engine_clone = engine.clone();
        let provider_clone = provider.clone();
        let handle = tokio::spawn(async move {
            engine_clone
                .run_background(provider_clone, shutdown_rx)
                .await;
        });

        // Let it run for at least one cycle
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        // Signal shutdown
        shutdown_tx.send(true).expect("channel send should succeed");
        handle.await.expect("async operation should succeed");

        // Should have run at least once
        assert!(
            provider.cycle_count.load(Ordering::SeqCst) >= 1,
            "Expected at least 1 consolidation cycle"
        );
    }
}
