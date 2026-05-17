//! Context Journal — Track reasoning decisions and context across sessions
//!
//! Records the cognitive context behind agent decisions, enabling:
//! - Post-hoc analysis of why decisions were made
//! - Cross-session pattern recognition
//! - Confidence calibration (was the agent right?)
//! - Context replay for debugging agent behavior
//!
//! Each journal entry captures the decision, alternatives considered,
//! confidence level, and outcome when available.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::debug;

// ============================================================================
// Configuration
// ============================================================================

/// Context journal configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalConfig {
    /// Maximum entries to keep in memory
    pub max_entries: usize,
    /// Auto-prune entries older than this (hours, 0 = no pruning)
    pub retention_hours: u64,
    /// Enable outcome tracking (requires explicit resolution)
    pub track_outcomes: bool,
    /// Enable pattern detection across entries
    pub detect_patterns: bool,
    /// Minimum entries before pattern detection kicks in
    pub pattern_min_entries: usize,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            max_entries: 1000,
            retention_hours: 168, // 1 week
            track_outcomes: true,
            detect_patterns: true,
            pattern_min_entries: 10,
        }
    }
}

// ============================================================================
// Journal Entry
// ============================================================================

/// A single decision record in the context journal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Unique ID
    pub id: String,
    /// Session ID this decision belongs to
    pub session_id: String,
    /// The decision made
    pub decision: String,
    /// Category (tool_choice, model_selection, strategy, routing, etc.)
    pub category: String,
    /// Alternatives that were considered
    pub alternatives: Vec<String>,
    /// Why this decision was chosen over alternatives
    pub reasoning: String,
    /// Confidence in this decision (0.0–1.0)
    pub confidence: f64,
    /// Context that influenced the decision
    pub context_factors: Vec<String>,
    /// When the decision was made
    pub timestamp: DateTime<Utc>,
    /// Outcome (populated later via resolve)
    pub outcome: Option<DecisionOutcome>,
}

/// Outcome of a past decision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionOutcome {
    /// Whether the decision was successful
    pub success: bool,
    /// Description of what happened
    pub description: String,
    /// When the outcome was recorded
    pub resolved_at: DateTime<Utc>,
}

// ============================================================================
// Decision Pattern
// ============================================================================

/// A detected pattern in decision-making
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionPattern {
    /// Category this pattern applies to
    pub category: String,
    /// Most common decision in this category
    pub dominant_decision: String,
    /// How often the dominant decision is made (0.0–1.0)
    pub frequency: f64,
    /// Success rate when this decision is made
    pub success_rate: f64,
    /// Number of entries this pattern is based on
    pub sample_size: usize,
}

// ============================================================================
// Context Journal
// ============================================================================

/// The context journal — records and analyzes agent decisions
pub struct ContextJournal {
    config: JournalConfig,
    entries: Vec<JournalEntry>,
    next_id: u64,
    stats: JournalStats,
}

/// Journal statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JournalStats {
    pub total_entries: u64,
    pub total_resolved: u64,
    pub total_successful: u64,
    pub total_failed: u64,
    pub avg_confidence: f64,
    pub categories_seen: u64,
}

impl ContextJournal {
    pub fn new(config: JournalConfig) -> Self {
        Self {
            config,
            entries: Vec::new(),
            next_id: 1,
            stats: JournalStats::default(),
        }
    }

    /// Record a new decision
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        session_id: &str,
        decision: &str,
        category: &str,
        alternatives: Vec<String>,
        reasoning: &str,
        confidence: f64,
        context_factors: Vec<String>,
    ) -> String {
        let id = format!("j-{}", self.next_id);
        self.next_id += 1;

        let entry = JournalEntry {
            id: id.clone(),
            session_id: session_id.to_string(),
            decision: decision.to_string(),
            category: category.to_string(),
            alternatives,
            reasoning: reasoning.to_string(),
            confidence: confidence.clamp(0.0, 1.0),
            context_factors,
            timestamp: Utc::now(),
            outcome: None,
        };

        self.entries.push(entry);
        self.stats.total_entries += 1;

        // Update running average confidence
        let n = self.stats.total_entries as f64;
        self.stats.avg_confidence = self.stats.avg_confidence * ((n - 1.0) / n) + confidence / n;

        // Prune if over max
        while self.entries.len() > self.config.max_entries {
            self.entries.remove(0);
        }

        debug!(id = %id, category = %category, confidence = confidence, "Decision recorded");
        id
    }

    /// Resolve a decision's outcome
    pub fn resolve(&mut self, entry_id: &str, success: bool, description: &str) -> bool {
        if !self.config.track_outcomes {
            return false;
        }

        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == entry_id) {
            if entry.outcome.is_some() {
                return false; // Already resolved
            }
            entry.outcome = Some(DecisionOutcome {
                success,
                description: description.to_string(),
                resolved_at: Utc::now(),
            });
            self.stats.total_resolved += 1;
            if success {
                self.stats.total_successful += 1;
            } else {
                self.stats.total_failed += 1;
            }
            debug!(id = %entry_id, success = success, "Decision resolved");
            return true;
        }
        false
    }

    /// Get entries for a specific session
    pub fn session_entries(&self, session_id: &str) -> Vec<&JournalEntry> {
        self.entries
            .iter()
            .filter(|e| e.session_id == session_id)
            .collect()
    }

    /// Get entries by category
    pub fn category_entries(&self, category: &str) -> Vec<&JournalEntry> {
        self.entries
            .iter()
            .filter(|e| e.category == category)
            .collect()
    }

    /// Get unresolved entries (decisions without outcomes)
    pub fn unresolved(&self) -> Vec<&JournalEntry> {
        self.entries
            .iter()
            .filter(|e| e.outcome.is_none())
            .collect()
    }

    /// Detect patterns across decisions
    pub fn detect_patterns(&self) -> Vec<DecisionPattern> {
        if !self.config.detect_patterns {
            return Vec::new();
        }

        // Group by category
        let mut category_groups: HashMap<String, Vec<&JournalEntry>> = HashMap::new();
        for entry in &self.entries {
            category_groups
                .entry(entry.category.clone())
                .or_default()
                .push(entry);
        }

        let mut patterns = Vec::new();

        for (category, entries) in &category_groups {
            if entries.len() < self.config.pattern_min_entries {
                continue;
            }

            // Find dominant decision
            let mut decision_counts: HashMap<&str, usize> = HashMap::new();
            for entry in entries {
                *decision_counts.entry(&entry.decision).or_insert(0) += 1;
            }

            if let Some((dominant, count)) = decision_counts.iter().max_by_key(|(_, c)| *c) {
                let frequency = *count as f64 / entries.len() as f64;

                // Calculate success rate for dominant decision
                let resolved_dominant: Vec<&&JournalEntry> = entries
                    .iter()
                    .filter(|e| e.decision == **dominant && e.outcome.is_some())
                    .collect();

                let success_rate = if resolved_dominant.is_empty() {
                    0.0
                } else {
                    let successes = resolved_dominant
                        .iter()
                        .filter(|e| e.outcome.as_ref().is_some_and(|o| o.success))
                        .count();
                    successes as f64 / resolved_dominant.len() as f64
                };

                patterns.push(DecisionPattern {
                    category: category.clone(),
                    dominant_decision: dominant.to_string(),
                    frequency,
                    success_rate,
                    sample_size: entries.len(),
                });
            }
        }

        patterns
    }

    /// Get confidence calibration (are high-confidence decisions more successful?)
    pub fn calibration_report(&self) -> CalibrationReport {
        let resolved: Vec<&JournalEntry> = self
            .entries
            .iter()
            .filter(|e| e.outcome.is_some())
            .collect();

        if resolved.is_empty() {
            return CalibrationReport::default();
        }

        // Bucket by confidence ranges
        let mut high_conf = (0u32, 0u32); // (total, successful)
        let mut med_conf = (0u32, 0u32);
        let mut low_conf = (0u32, 0u32);

        for entry in &resolved {
            let success = entry.outcome.as_ref().is_some_and(|o| o.success);
            if entry.confidence >= 0.8 {
                high_conf.0 += 1;
                if success {
                    high_conf.1 += 1;
                }
            } else if entry.confidence >= 0.5 {
                med_conf.0 += 1;
                if success {
                    med_conf.1 += 1;
                }
            } else {
                low_conf.0 += 1;
                if success {
                    low_conf.1 += 1;
                }
            }
        }

        CalibrationReport {
            high_confidence_accuracy: if high_conf.0 > 0 {
                high_conf.1 as f64 / high_conf.0 as f64
            } else {
                0.0
            },
            medium_confidence_accuracy: if med_conf.0 > 0 {
                med_conf.1 as f64 / med_conf.0 as f64
            } else {
                0.0
            },
            low_confidence_accuracy: if low_conf.0 > 0 {
                low_conf.1 as f64 / low_conf.0 as f64
            } else {
                0.0
            },
            total_resolved: resolved.len() as u32,
            well_calibrated: high_conf.0 > 0
                && med_conf.0 > 0
                && (high_conf.1 as f64 / high_conf.0 as f64)
                    > (med_conf.1 as f64 / med_conf.0 as f64),
        }
    }

    /// Prune old entries beyond retention period
    pub fn prune(&mut self) -> usize {
        if self.config.retention_hours == 0 {
            return 0;
        }
        let cutoff = Utc::now() - Duration::hours(self.config.retention_hours as i64);
        let before = self.entries.len();
        self.entries.retain(|e| e.timestamp >= cutoff);
        before - self.entries.len()
    }

    /// Get a specific entry by ID
    pub fn get(&self, entry_id: &str) -> Option<&JournalEntry> {
        self.entries.iter().find(|e| e.id == entry_id)
    }

    /// Total entries currently stored
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the journal is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all entries
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Get statistics
    pub fn stats(&self) -> &JournalStats {
        &self.stats
    }

    /// Get config
    pub fn config(&self) -> &JournalConfig {
        &self.config
    }

    /// Update config
    pub fn set_config(&mut self, config: JournalConfig) {
        self.config = config;
    }
}

impl Default for ContextJournal {
    fn default() -> Self {
        Self::new(JournalConfig::default())
    }
}

// ============================================================================
// Calibration Report
// ============================================================================

/// How well confidence scores predict success
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CalibrationReport {
    /// Accuracy of decisions with confidence >= 0.8
    pub high_confidence_accuracy: f64,
    /// Accuracy of decisions with confidence 0.5–0.8
    pub medium_confidence_accuracy: f64,
    /// Accuracy of decisions with confidence < 0.5
    pub low_confidence_accuracy: f64,
    /// Total resolved decisions
    pub total_resolved: u32,
    /// Whether high confidence → higher accuracy than medium
    pub well_calibrated: bool,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_journal() -> ContextJournal {
        ContextJournal::new(JournalConfig {
            max_entries: 100,
            retention_hours: 24,
            track_outcomes: true,
            detect_patterns: true,
            pattern_min_entries: 3,
        })
    }

    fn record_simple(journal: &mut ContextJournal, category: &str, decision: &str) -> String {
        journal.record(
            "session-1",
            decision,
            category,
            vec!["alt-a".into(), "alt-b".into()],
            "Best option available",
            0.8,
            vec!["context-1".into()],
        )
    }

    #[test]
    fn test_record_entry() {
        let mut journal = test_journal();
        let id = record_simple(&mut journal, "tool_choice", "shell");
        assert_eq!(journal.len(), 1);
        assert!(id.starts_with("j-"));
    }

    #[test]
    fn test_get_entry() {
        let mut journal = test_journal();
        let id = record_simple(&mut journal, "tool_choice", "shell");
        let entry = journal.get(&id).unwrap();
        assert_eq!(entry.decision, "shell");
        assert_eq!(entry.category, "tool_choice");
        assert_eq!(entry.alternatives.len(), 2);
    }

    #[test]
    fn test_resolve_success() {
        let mut journal = test_journal();
        let id = record_simple(&mut journal, "tool_choice", "shell");
        assert!(journal.resolve(&id, true, "Command executed successfully"));
        let entry = journal.get(&id).unwrap();
        assert!(entry.outcome.is_some());
        assert!(entry.outcome.as_ref().unwrap().success);
    }

    #[test]
    fn test_resolve_failure() {
        let mut journal = test_journal();
        let id = record_simple(&mut journal, "tool_choice", "shell");
        assert!(journal.resolve(&id, false, "Permission denied"));
        assert_eq!(journal.stats().total_failed, 1);
    }

    #[test]
    fn test_double_resolve_rejected() {
        let mut journal = test_journal();
        let id = record_simple(&mut journal, "tool_choice", "shell");
        assert!(journal.resolve(&id, true, "OK"));
        assert!(!journal.resolve(&id, false, "Changed mind")); // rejected
    }

    #[test]
    fn test_resolve_nonexistent() {
        let mut journal = test_journal();
        assert!(!journal.resolve("nonexistent", true, "OK"));
    }

    #[test]
    fn test_session_entries() {
        let mut journal = test_journal();
        journal.record("s1", "d1", "cat", vec![], "r", 0.8, vec![]);
        journal.record("s2", "d2", "cat", vec![], "r", 0.8, vec![]);
        journal.record("s1", "d3", "cat", vec![], "r", 0.8, vec![]);

        assert_eq!(journal.session_entries("s1").len(), 2);
        assert_eq!(journal.session_entries("s2").len(), 1);
    }

    #[test]
    fn test_category_entries() {
        let mut journal = test_journal();
        record_simple(&mut journal, "tool_choice", "shell");
        record_simple(&mut journal, "model_selection", "claude");
        record_simple(&mut journal, "tool_choice", "read_file");

        assert_eq!(journal.category_entries("tool_choice").len(), 2);
        assert_eq!(journal.category_entries("model_selection").len(), 1);
    }

    #[test]
    fn test_unresolved() {
        let mut journal = test_journal();
        let id1 = record_simple(&mut journal, "tool", "shell");
        record_simple(&mut journal, "tool", "read");

        journal.resolve(&id1, true, "OK");
        assert_eq!(journal.unresolved().len(), 1);
    }

    #[test]
    fn test_detect_patterns() {
        let mut journal = test_journal();
        // Record many "shell" decisions in tool_choice
        for _ in 0..5 {
            let id = record_simple(&mut journal, "tool_choice", "shell");
            journal.resolve(&id, true, "OK");
        }
        record_simple(&mut journal, "tool_choice", "read_file");

        let patterns = journal.detect_patterns();
        assert!(!patterns.is_empty());
        let tool_pattern = patterns
            .iter()
            .find(|p| p.category == "tool_choice")
            .unwrap();
        assert_eq!(tool_pattern.dominant_decision, "shell");
        assert!(tool_pattern.frequency > 0.5);
        assert!(tool_pattern.success_rate > 0.0);
    }

    #[test]
    fn test_detect_patterns_below_minimum() {
        let mut journal = test_journal();
        record_simple(&mut journal, "rare", "decision");
        record_simple(&mut journal, "rare", "decision");
        // Only 2 entries, min is 3
        let patterns = journal.detect_patterns();
        assert!(patterns.iter().find(|p| p.category == "rare").is_none());
    }

    #[test]
    fn test_calibration_report() {
        let mut journal = test_journal();

        // High confidence decisions — all succeed
        for _ in 0..3 {
            let id = journal.record("s1", "good", "cat", vec![], "r", 0.9, vec![]);
            journal.resolve(&id, true, "OK");
        }
        // Low confidence decisions — all fail
        for _ in 0..3 {
            let id = journal.record("s1", "risky", "cat", vec![], "r", 0.3, vec![]);
            journal.resolve(&id, false, "Failed");
        }

        let report = journal.calibration_report();
        assert_eq!(report.total_resolved, 6);
        assert!((report.high_confidence_accuracy - 1.0).abs() < f64::EPSILON);
        assert!((report.low_confidence_accuracy - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_max_entries_prune() {
        let mut journal = ContextJournal::new(JournalConfig {
            max_entries: 5,
            ..Default::default()
        });
        for i in 0..10 {
            journal.record("s1", &format!("d{}", i), "cat", vec![], "r", 0.8, vec![]);
        }
        assert_eq!(journal.len(), 5);
    }

    #[test]
    fn test_retention_prune() {
        let mut journal = ContextJournal::new(JournalConfig {
            retention_hours: 1,
            ..Default::default()
        });
        journal.record("s1", "recent", "cat", vec![], "r", 0.8, vec![]);

        // Backdate one entry
        if let Some(entry) = journal.entries.first_mut() {
            entry.timestamp = Utc::now() - Duration::hours(2);
        }

        let pruned = journal.prune();
        assert_eq!(pruned, 1);
        assert_eq!(journal.len(), 0);
    }

    #[test]
    fn test_clear() {
        let mut journal = test_journal();
        record_simple(&mut journal, "cat", "dec");
        journal.clear();
        assert!(journal.is_empty());
    }

    #[test]
    fn test_confidence_clamped() {
        let mut journal = test_journal();
        journal.record("s1", "d", "c", vec![], "r", 1.5, vec![]);
        let entry = journal.entries.last().unwrap();
        assert!((entry.confidence - 1.0).abs() < f64::EPSILON);

        journal.record("s1", "d", "c", vec![], "r", -0.5, vec![]);
        let entry = journal.entries.last().unwrap();
        assert!((entry.confidence - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_avg_confidence_tracking() {
        let mut journal = test_journal();
        journal.record("s1", "d1", "c", vec![], "r", 0.8, vec![]);
        journal.record("s1", "d2", "c", vec![], "r", 0.6, vec![]);
        // avg = (0.8 + 0.6) / 2 = 0.7
        assert!((journal.stats().avg_confidence - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_stats_tracking() {
        let mut journal = test_journal();
        let id = record_simple(&mut journal, "cat", "dec");
        journal.resolve(&id, true, "OK");
        assert_eq!(journal.stats().total_entries, 1);
        assert_eq!(journal.stats().total_resolved, 1);
        assert_eq!(journal.stats().total_successful, 1);
    }

    #[test]
    fn test_config_update() {
        let mut journal = test_journal();
        journal.set_config(JournalConfig {
            max_entries: 500,
            ..Default::default()
        });
        assert_eq!(journal.config().max_entries, 500);
    }

    #[test]
    fn test_default_journal() {
        let journal = ContextJournal::default();
        assert_eq!(journal.config().max_entries, 1000);
        assert!(journal.is_empty());
    }

    #[test]
    fn test_patterns_disabled() {
        let mut journal = ContextJournal::new(JournalConfig {
            detect_patterns: false,
            ..Default::default()
        });
        for _ in 0..10 {
            record_simple(&mut journal, "cat", "dec");
        }
        assert!(journal.detect_patterns().is_empty());
    }
}
