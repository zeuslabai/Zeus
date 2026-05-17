//! MetaLoop — AutoAgent-style iterative config optimization.
//!
//! Closes the loop between `ConfigExperiment` (atomic run-experiment primitive)
//! and an `ExperimentProposer` that picks which config mutations to try.
//! Parallel entry point to `auto_tune()` — reuses `run_experiment()` but with
//! a proposer-driven loop instead of hardcoded changes.

use crate::experiment::{default_experiment_changes, ConfigChange, ConfigExperiment, ExperimentOutcome};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use zeus_core::Result;

// ---------------------------------------------------------------------------
// Proposer trait
// ---------------------------------------------------------------------------

/// Proposes config mutations for the MetaLoop to test.
/// Implementations receive iteration history so they can learn from outcomes.
pub trait ExperimentProposer: Send + Sync {
    /// Propose a config change based on history of prior iterations.
    /// Return `None` to signal convergence (no more changes worth trying).
    fn propose(&mut self, history: &[IterationRecord]) -> Option<ConfigChange>;

    /// Human-readable name of this proposer (for logging/audit).
    fn name(&self) -> &str;
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Record of a single MetaLoop iteration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationRecord {
    /// Iteration number (0-based).
    pub iter: u32,
    /// The config change that was tested.
    pub change: ConfigChange,
    /// Outcome from `run_experiment`.
    pub outcome: ExperimentOutcome,
    /// When this iteration ran.
    pub timestamp: DateTime<Utc>,
}

/// Summary report from a full MetaLoop run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaLoopReport {
    /// All iterations in order.
    pub iterations: Vec<IterationRecord>,
    /// How many changes were kept.
    pub kept_count: u32,
    /// How many changes were reverted.
    pub reverted_count: u32,
    /// How many experiments failed to run.
    pub failed_count: u32,
    /// Name of the proposer used.
    pub proposer_name: String,
}

// ---------------------------------------------------------------------------
// MetaLoop
// ---------------------------------------------------------------------------

/// Iterative config optimizer: proposer picks mutations, experiment tests them.
pub struct MetaLoop {
    experiment: ConfigExperiment,
    proposer: Box<dyn ExperimentProposer>,
}

impl MetaLoop {
    /// Create a new MetaLoop with the given experiment engine and proposer.
    pub fn new(experiment: ConfigExperiment, proposer: Box<dyn ExperimentProposer>) -> Self {
        Self {
            experiment,
            proposer,
        }
    }

    /// Run the optimization loop for up to `max_iters` iterations.
    /// Stops early if the proposer returns `None` (convergence).
    pub async fn run(&mut self, max_iters: u32) -> Result<MetaLoopReport> {
        let proposer_name = self.proposer.name().to_string();
        info!(proposer = %proposer_name, max_iters, "MetaLoop starting");

        let mut history: Vec<IterationRecord> = Vec::new();
        let mut kept = 0u32;
        let mut reverted = 0u32;
        let mut failed = 0u32;

        for iter in 0..max_iters {
            // Ask proposer for next change
            let change = match self.proposer.propose(&history) {
                Some(c) => c,
                None => {
                    info!(iter, "Proposer signalled convergence — stopping early");
                    break;
                }
            };

            info!(
                iter,
                key = %change.key,
                value = %change.value,
                desc = %change.description,
                "MetaLoop iteration"
            );

            // Run the atomic experiment
            let outcome = match self.experiment.run_experiment(change.clone()).await {
                Ok(o) => o,
                Err(e) => {
                    warn!(iter, error = %e, "Experiment failed");
                    let outcome = ExperimentOutcome::Failed {
                        change: change.clone(),
                        error: e.to_string(),
                    };
                    failed += 1;
                    history.push(IterationRecord {
                        iter,
                        change,
                        outcome: outcome.clone(),
                        timestamp: Utc::now(),
                    });
                    continue;
                }
            };

            // Tally outcome
            match &outcome {
                ExperimentOutcome::Kept { .. } => kept += 1,
                ExperimentOutcome::Reverted { .. } => reverted += 1,
                ExperimentOutcome::Failed { .. } => failed += 1,
            }

            history.push(IterationRecord {
                iter,
                change,
                outcome,
                timestamp: Utc::now(),
            });
        }

        info!(
            kept,
            reverted,
            failed,
            total = history.len(),
            "MetaLoop complete"
        );

        Ok(MetaLoopReport {
            iterations: history,
            kept_count: kept,
            reverted_count: reverted,
            failed_count: failed,
            proposer_name,
        })
    }
}

// ---------------------------------------------------------------------------
// RandomProposer
// ---------------------------------------------------------------------------

/// Randomly samples from a whitelist of mutable config keys.
/// Phase 1 baseline — used to validate the loop works end-to-end
/// and to measure whether LlmProposer (Phase 2) adds signal over random.
pub struct RandomProposer {
    /// Whitelist of (key, candidate_values, description) tuples.
    mutable_keys: Vec<(String, Vec<String>, String)>,
    /// Current index (round-robin through keys).
    index: usize,
    /// Max proposals before signalling convergence.
    max_proposals: u32,
    /// Proposals made so far.
    proposed: u32,
}

impl RandomProposer {
    /// Create a proposer with the default mutable keys whitelist.
    pub fn new(max_proposals: u32) -> Self {
        Self {
            mutable_keys: default_mutable_keys(),
            index: 0,
            max_proposals,
            proposed: 0,
        }
    }
}

impl ExperimentProposer for RandomProposer {
    fn propose(&mut self, _history: &[IterationRecord]) -> Option<ConfigChange> {
        if self.proposed >= self.max_proposals || self.mutable_keys.is_empty() {
            return None;
        }

        let (key, values, desc) = &self.mutable_keys[self.index % self.mutable_keys.len()];

        // Pick a value — cycle through candidates based on proposal count
        let value = &values[self.proposed as usize % values.len()];

        let change = ConfigChange {
            key: key.clone(),
            value: value.clone(),
            description: desc.clone(),
        };

        self.index += 1;
        self.proposed += 1;
        Some(change)
    }

    fn name(&self) -> &str {
        "RandomProposer"
    }
}

// ---------------------------------------------------------------------------
// StaticProposer
// ---------------------------------------------------------------------------

/// Proposes a fixed, caller-supplied sequence of config changes, in order.
///
/// Backward-compat adapter that makes the hardcoded `auto_tune()` behavior
/// reachable through the `MetaLoop` interface. Also useful as a deterministic
/// baseline to A/B against `RandomProposer` or `LlmProposer` (Phase 2).
///
/// Returns `Some(change)` for each entry in the sequence, then `None` to
/// signal convergence once the sequence is exhausted.
pub struct StaticProposer {
    changes: Vec<ConfigChange>,
    idx: usize,
}

impl StaticProposer {
    /// Create a StaticProposer from an explicit sequence of changes.
    pub fn new(changes: Vec<ConfigChange>) -> Self {
        Self { changes, idx: 0 }
    }

    /// Create a StaticProposer seeded from the same default sequence that
    /// `auto_tune()` uses. Gives `auto_tune()` parity through `MetaLoop`.
    pub fn from_defaults() -> Self {
        Self::new(default_experiment_changes())
    }

    /// Number of changes remaining in the sequence.
    pub fn remaining(&self) -> usize {
        self.changes.len().saturating_sub(self.idx)
    }
}

impl ExperimentProposer for StaticProposer {
    fn propose(&mut self, _history: &[IterationRecord]) -> Option<ConfigChange> {
        if self.idx >= self.changes.len() {
            return None;
        }
        let change = self.changes[self.idx].clone();
        self.idx += 1;
        Some(change)
    }

    fn name(&self) -> &str {
        "StaticProposer"
    }
}

/// Default whitelist of config keys safe to mutate during experiments.
fn default_mutable_keys() -> Vec<(String, Vec<String>, String)> {
    vec![
        (
            "max_iterations".into(),
            vec!["10".into(), "15".into(), "20".into(), "25".into(), "30".into()],
            "Agent loop iteration limit".into(),
        ),
        (
            "max_subagent_iterations".into(),
            vec!["5".into(), "8".into(), "10".into(), "15".into()],
            "Subagent iteration limit".into(),
        ),
        (
            "prometheus.thinking_level".into(),
            vec!["low".into(), "medium".into(), "high".into()],
            "LLM thinking/reasoning depth".into(),
        ),
        (
            "gateway.timeout_secs".into(),
            vec!["600".into(), "900".into(), "1200".into(), "1800".into()],
            "Gateway cooking timeout".into(),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Stub proposer that returns a fixed sequence then stops.
    struct FixedProposer {
        changes: Vec<ConfigChange>,
        idx: usize,
    }

    impl FixedProposer {
        fn new(changes: Vec<ConfigChange>) -> Self {
            Self { changes, idx: 0 }
        }
    }

    impl ExperimentProposer for FixedProposer {
        fn propose(&mut self, _history: &[IterationRecord]) -> Option<ConfigChange> {
            if self.idx >= self.changes.len() {
                return None;
            }
            let c = self.changes[self.idx].clone();
            self.idx += 1;
            Some(c)
        }

        fn name(&self) -> &str {
            "FixedProposer"
        }
    }

    #[test]
    fn test_random_proposer_convergence() {
        let mut proposer = RandomProposer::new(3);
        let mut count = 0;
        while proposer.propose(&[]).is_some() {
            count += 1;
        }
        assert_eq!(count, 3, "RandomProposer should stop after max_proposals");
    }

    #[test]
    fn test_random_proposer_cycles_keys() {
        let mut proposer = RandomProposer::new(8);
        let mut keys = Vec::new();
        while let Some(change) = proposer.propose(&[]) {
            keys.push(change.key.clone());
        }
        assert_eq!(keys.len(), 8);
        // Should cycle through the 4 keys twice
        assert_eq!(keys[0], keys[4]);
        assert_eq!(keys[1], keys[5]);
    }

    #[test]
    fn test_fixed_proposer_stops() {
        let mut proposer = FixedProposer::new(vec![
            ConfigChange {
                key: "max_iterations".into(),
                value: "25".into(),
                description: "test".into(),
            },
        ]);
        assert!(proposer.propose(&[]).is_some());
        assert!(proposer.propose(&[]).is_none());
    }

    #[test]
    fn test_static_proposer_exhausts() {
        let changes = vec![
            ConfigChange {
                key: "max_iterations".into(),
                value: "25".into(),
                description: "first".into(),
            },
            ConfigChange {
                key: "max_iterations".into(),
                value: "30".into(),
                description: "second".into(),
            },
        ];
        let mut proposer = StaticProposer::new(changes);
        assert_eq!(proposer.remaining(), 2);
        assert_eq!(proposer.name(), "StaticProposer");

        let first = proposer.propose(&[]).expect("first");
        assert_eq!(first.value, "25");
        assert_eq!(proposer.remaining(), 1);

        let second = proposer.propose(&[]).expect("second");
        assert_eq!(second.value, "30");
        assert_eq!(proposer.remaining(), 0);

        assert!(proposer.propose(&[]).is_none(), "should signal convergence");
        assert!(proposer.propose(&[]).is_none(), "stays exhausted");
    }

    #[test]
    fn test_static_proposer_from_defaults() {
        let mut proposer = StaticProposer::from_defaults();
        assert_eq!(proposer.name(), "StaticProposer");
        assert!(proposer.remaining() > 0, "defaults should be non-empty");

        // Drain the proposer and verify it matches default_experiment_changes().
        let expected_len = proposer.remaining();
        let mut drained = 0;
        while proposer.propose(&[]).is_some() {
            drained += 1;
        }
        assert_eq!(drained, expected_len, "drained count matches initial remaining");
        assert_eq!(proposer.remaining(), 0);
    }

    #[test]
    fn test_meta_loop_report_defaults() {
        let report = MetaLoopReport {
            iterations: vec![],
            kept_count: 0,
            reverted_count: 0,
            failed_count: 0,
            proposer_name: "test".into(),
        };
        assert_eq!(report.iterations.len(), 0);
        assert_eq!(report.proposer_name, "test");
    }
}
