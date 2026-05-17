//! Supersession Detection for Temporal Memory
//!
//! Detects when a newly stored memory supersedes (replaces/updates) an existing
//! memory. Uses embedding similarity to find candidates, then applies heuristic
//! or LLM-based judgment to confirm supersession.
//!
//! ## How it works
//!
//! 1. When a new Fact/Preference/Semantic memory is stored, we search for
//!    similar existing memories of the same type.
//! 2. Candidates with cosine similarity above the threshold are evaluated.
//! 3. A `SupersessionJudge` decides if the new memory supersedes each candidate.
//! 4. Superseded memories get `valid_to = now` and `superseded_by = new_id`.

use crate::{MemoryStore, MemoryType};
use tracing::{debug, info};
use zeus_core::Result;

/// Configuration for supersession detection
#[derive(Debug, Clone)]
pub struct SupersessionConfig {
    /// Minimum cosine similarity to consider a candidate (0.0–1.0)
    pub similarity_threshold: f64,
    /// Maximum number of candidates to evaluate per new memory
    pub max_candidates: usize,
    /// Memory types eligible for supersession (others are always appended)
    pub eligible_types: Vec<MemoryType>,
    /// Enable supersession detection
    pub enabled: bool,
}

impl Default for SupersessionConfig {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.80,
            max_candidates: 5,
            eligible_types: vec![
                MemoryType::Fact,
                MemoryType::Preference,
                MemoryType::Semantic,
            ],
            enabled: true,
        }
    }
}

/// Trait for judging whether a new memory supersedes an existing one.
///
/// Implementations can range from simple heuristics to full LLM calls.
pub trait SupersessionJudge: Send + Sync {
    /// Determine if `new_content` supersedes `old_content`.
    ///
    /// Returns `true` if the new memory replaces the old one.
    /// Both memories are of the same `MemoryType`.
    fn is_superseded(&self, old_content: &str, new_content: &str, memory_type: &MemoryType)
    -> bool;
}

/// Simple similarity-based judge that uses keyword overlap and content analysis.
///
/// This is the default judge — no LLM required. It uses:
/// - Jaccard similarity on significant words
/// - Pattern matching for contradiction signals
/// - Fact structure analysis (subject matching)
pub struct HeuristicJudge {
    /// Minimum word overlap ratio to consider supersession (0.0–1.0)
    pub overlap_threshold: f64,
}

impl Default for HeuristicJudge {
    fn default() -> Self {
        Self {
            overlap_threshold: 0.40,
        }
    }
}

impl SupersessionJudge for HeuristicJudge {
    fn is_superseded(
        &self,
        old_content: &str,
        new_content: &str,
        memory_type: &MemoryType,
    ) -> bool {
        // Skip if contents are identical (dedup handles this)
        if old_content.trim() == new_content.trim() {
            return false;
        }

        let old_words = significant_words(old_content);
        let new_words = significant_words(new_content);

        if old_words.is_empty() || new_words.is_empty() {
            return false;
        }

        // Jaccard similarity on significant words
        let intersection = old_words.intersection(&new_words).count();
        let union = old_words.union(&new_words).count();
        let jaccard = intersection as f64 / union as f64;

        // For facts and preferences, we need subject overlap + different values
        match memory_type {
            MemoryType::Fact | MemoryType::Preference => {
                // High subject overlap with different overall content = likely supersession
                // e.g., "User prefers dark mode" → "User prefers light mode"
                let subject_overlap = subject_similarity(old_content, new_content);
                subject_overlap >= self.overlap_threshold && jaccard < 0.95
            }
            MemoryType::Semantic => {
                // Semantic knowledge: high topic overlap = likely updated understanding
                jaccard >= self.overlap_threshold && jaccard < 0.95
            }
            _ => false,
        }
    }
}

/// Extract significant words (lowercased, stop words removed)
fn significant_words(text: &str) -> std::collections::HashSet<String> {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
        "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into", "through",
        "during", "before", "after", "above", "below", "between", "out", "off", "over", "under",
        "again", "further", "then", "once", "here", "there", "when", "where", "why", "how", "all",
        "each", "every", "both", "few", "more", "most", "other", "some", "such", "no", "nor",
        "not", "only", "own", "same", "so", "than", "too", "very", "just", "because", "but", "and",
        "or", "if", "while", "that", "this", "it", "its", "i", "me", "my", "we", "our", "you",
        "your", "he", "him", "his", "she", "her", "they", "them", "their", "what", "which", "who",
        "whom",
    ];

    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2 && !STOP_WORDS.contains(w))
        .map(|w| w.to_string())
        .collect()
}

/// Compute subject similarity: how much the "topic" words overlap.
/// Focuses on the first part of each text (usually contains the subject).
fn subject_similarity(old: &str, new: &str) -> f64 {
    // Take first ~50 chars as "subject" area
    let old_subj: Vec<String> = old
        .chars()
        .take(80)
        .collect::<String>()
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(|w| w.to_string())
        .collect();

    let new_subj: Vec<String> = new
        .chars()
        .take(80)
        .collect::<String>()
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(|w| w.to_string())
        .collect();

    if old_subj.is_empty() || new_subj.is_empty() {
        return 0.0;
    }

    let old_set: std::collections::HashSet<_> = old_subj.iter().collect();
    let new_set: std::collections::HashSet<_> = new_subj.iter().collect();
    let intersection = old_set.intersection(&new_set).count();
    let union = old_set.union(&new_set).count();
    intersection as f64 / union as f64
}

/// Detect and apply supersessions for a newly stored memory.
///
/// Called after a new memory is stored with an embedding. Searches for similar
/// existing memories and marks superseded ones.
///
/// Returns the number of memories that were superseded.
pub fn detect_supersessions(
    store: &MemoryStore,
    new_id: i64,
    new_content: &str,
    new_embedding: &[f32],
    memory_type: &MemoryType,
    config: &SupersessionConfig,
    judge: &dyn SupersessionJudge,
) -> Result<usize> {
    if !config.enabled {
        return Ok(0);
    }

    // Only check eligible memory types
    if !config.eligible_types.contains(memory_type) {
        return Ok(0);
    }

    // Find similar existing memories via embedding similarity
    let candidates = store.vector_search(new_embedding, config.max_candidates * 2)?;

    let mut superseded_count = 0;

    for candidate in &candidates {
        // Skip self
        if candidate.id == new_id {
            continue;
        }

        // Skip already-superseded memories
        if candidate.valid_to.is_some() {
            continue;
        }

        // Skip different memory types
        if candidate.memory_type != *memory_type {
            continue;
        }

        // Check similarity threshold
        if (candidate.score as f64) < config.similarity_threshold {
            continue;
        }

        // Check with judge
        if judge.is_superseded(&candidate.content, new_content, memory_type) {
            debug!(
                old_id = candidate.id,
                new_id = new_id,
                similarity = candidate.score,
                "Superseding memory"
            );
            store.supersede_message(candidate.id, new_id)?;
            superseded_count += 1;

            if superseded_count >= config.max_candidates {
                break;
            }
        }
    }

    if superseded_count > 0 {
        info!(
            count = superseded_count,
            new_id = new_id,
            memory_type = %memory_type,
            "Detected and applied supersessions"
        );
    }

    Ok(superseded_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_significant_words() {
        let words = significant_words("The user prefers dark mode for the editor");
        assert!(words.contains("user"));
        assert!(words.contains("prefers"));
        assert!(words.contains("dark"));
        assert!(words.contains("mode"));
        assert!(words.contains("editor"));
        assert!(!words.contains("the")); // stop word
        assert!(!words.contains("for")); // stop word
    }

    #[test]
    fn test_subject_similarity_same_topic() {
        let sim = subject_similarity(
            "User prefers dark mode in the editor",
            "User prefers light mode in the editor",
        );
        assert!(
            sim > 0.5,
            "Same-topic sentences should have high similarity: {}",
            sim
        );
    }

    #[test]
    fn test_subject_similarity_different_topics() {
        let sim = subject_similarity(
            "User prefers dark mode in the editor",
            "The weather forecast shows rain tomorrow",
        );
        assert!(
            sim < 0.2,
            "Different topics should have low similarity: {}",
            sim
        );
    }

    #[test]
    fn test_heuristic_judge_fact_supersession() {
        let judge = HeuristicJudge::default();

        // Same subject, different value → supersession
        assert!(judge.is_superseded(
            "User's favorite color is blue",
            "User's favorite color is green",
            &MemoryType::Fact,
        ));

        // Completely different facts → no supersession
        assert!(!judge.is_superseded(
            "User's favorite color is blue",
            "The database runs on PostgreSQL",
            &MemoryType::Fact,
        ));

        // Identical content → no supersession (dedup handles this)
        assert!(!judge.is_superseded(
            "User's favorite color is blue",
            "User's favorite color is blue",
            &MemoryType::Fact,
        ));
    }

    #[test]
    fn test_heuristic_judge_preference_supersession() {
        let judge = HeuristicJudge::default();

        assert!(judge.is_superseded(
            "User prefers vim keybindings",
            "User prefers emacs keybindings",
            &MemoryType::Preference,
        ));

        // Different preferences entirely
        assert!(!judge.is_superseded(
            "User prefers vim keybindings",
            "User likes running in the morning",
            &MemoryType::Preference,
        ));
    }

    #[test]
    fn test_heuristic_judge_working_memory_skipped() {
        let judge = HeuristicJudge::default();

        // Working memory is never eligible for supersession
        assert!(!judge.is_superseded(
            "Working on task A",
            "Working on task B",
            &MemoryType::Working,
        ));
    }

    #[test]
    fn test_config_defaults() {
        let config = SupersessionConfig::default();
        assert!(config.enabled);
        assert_eq!(config.similarity_threshold, 0.80);
        assert_eq!(config.max_candidates, 5);
        assert_eq!(config.eligible_types.len(), 3);
    }
}
