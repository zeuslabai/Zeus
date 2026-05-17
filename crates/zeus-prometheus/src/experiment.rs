//! Config experiment engine for auto-tuning.
//!
//! `ConfigExperiment` snapshots the current config, applies a candidate change,
//! runs a benchmark, compares scores against the baseline, and keeps or reverts
//! the change based on whether the candidate improves pass rate without regressions.

use crate::benchmark::{BenchmarkStore, RunComparison};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{info, warn};
use zeus_core::{Config, Error, Result};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single config change to test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigChange {
    /// Human-readable description of what's being changed.
    pub description: String,
    /// The TOML key path being modified (e.g. "max_iterations", "model").
    pub key: String,
    /// The new value to try (serialized as TOML string).
    pub value: String,
}

/// Outcome of a single experiment cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExperimentOutcome {
    /// Candidate was better — config change kept.
    Kept {
        change: ConfigChange,
        comparison: RunComparison,
    },
    /// Candidate was worse or equal — config reverted.
    Reverted {
        change: ConfigChange,
        comparison: RunComparison,
        reason: String,
    },
    /// Experiment failed to run (e.g. benchmark error).
    Failed {
        change: ConfigChange,
        error: String,
    },
}

/// Result of a full auto-tune cycle (one or more experiments).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoTuneReport {
    pub experiments: Vec<ExperimentOutcome>,
    pub kept_count: usize,
    pub reverted_count: usize,
    pub failed_count: usize,
}

// ---------------------------------------------------------------------------
// ConfigExperiment
// ---------------------------------------------------------------------------

/// Engine for running A/B config experiments against benchmarks.
pub struct ConfigExperiment {
    /// Path to config.toml (default: ~/.zeus/config.toml).
    config_path: PathBuf,
    /// Snapshot of the original config TOML before any experiment.
    snapshot: Option<String>,
    /// Benchmark store for persisting and comparing results.
    store: BenchmarkStore,
}

impl ConfigExperiment {
    /// Create a new experiment engine with the default config and benchmark paths.
    pub fn new() -> Result<Self> {
        let config_path = dirs::home_dir()
            .ok_or_else(|| Error::Config("No home directory".into()))?
            .join(".zeus")
            .join("config.toml");
        let store = BenchmarkStore::open_default()?;
        Ok(Self {
            config_path,
            snapshot: None,
            store,
        })
    }

    /// Create with explicit paths (for testing).
    pub fn with_paths(config_path: PathBuf, db_path: PathBuf) -> Result<Self> {
        let store = BenchmarkStore::new(&db_path)?;
        Ok(Self {
            config_path,
            snapshot: None,
            store,
        })
    }

    /// Snapshot the current config file contents.
    pub fn snapshot_config(&mut self) -> Result<()> {
        let content = std::fs::read_to_string(&self.config_path).map_err(|e| {
            Error::Config(format!(
                "Failed to read {}: {}",
                self.config_path.display(),
                e
            ))
        })?;
        self.snapshot = Some(content);
        info!(path = %self.config_path.display(), "Config snapshot taken");
        Ok(())
    }

    /// Restore config from snapshot.
    pub fn restore_config(&self) -> Result<()> {
        let content = self
            .snapshot
            .as_ref()
            .ok_or_else(|| Error::Config("No snapshot to restore from".into()))?;
        atomic_write(&self.config_path, content)?;
        info!(path = %self.config_path.display(), "Config restored from snapshot");
        Ok(())
    }

    /// Apply a `ConfigChange` to the live config file.
    ///
    /// Loads the current config, modifies the field, and saves it back.
    /// Only supports top-level keys that are representable as TOML scalars.
    pub fn apply_change(&self, change: &ConfigChange) -> Result<()> {
        let content = std::fs::read_to_string(&self.config_path).map_err(|e| {
            Error::Config(format!("read config: {}", e))
        })?;

        let mut doc = content
            .parse::<toml::Table>()
            .map_err(|e| Error::Config(format!("parse config TOML: {}", e)))?;

        // Parse the new value as a TOML value.
        // A bare value like `30` isn't valid TOML on its own — wrap it in a
        // dummy key assignment so the TOML parser can handle it, then extract
        // the value.
        let new_value: toml::Value = change
            .value
            .parse::<toml::Value>()
            .or_else(|_| {
                // Try as `_k = <value>` so integers, strings, etc. all work
                let wrapped = format!("_k = {}", change.value);
                wrapped
                    .parse::<toml::Table>()
                    .map_err(|e| Error::Config(format!("parse value '{}': {}", change.value, e)))
                    .and_then(|t| {
                        t.get("_k")
                            .cloned()
                            .ok_or_else(|| Error::Config("empty parse result".into()))
                    })
            })?;

        // Handle nested keys with dot notation (e.g. "prometheus.max_iterations")
        let parts: Vec<&str> = change.key.split('.').collect();
        if parts.len() == 1 {
            doc.insert(parts[0].to_string(), new_value);
        } else if parts.len() == 2 {
            let section = doc
                .entry(parts[0])
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            if let Some(table) = section.as_table_mut() {
                table.insert(parts[1].to_string(), new_value);
            } else {
                return Err(Error::Config(format!(
                    "'{}' is not a TOML table",
                    parts[0]
                )));
            }
        } else {
            return Err(Error::Config(format!(
                "Key path depth > 2 not supported: '{}'",
                change.key
            )));
        }

        let new_content =
            toml::to_string_pretty(&doc).map_err(|e| Error::Config(format!("serialize: {}", e)))?;
        atomic_write(&self.config_path, &new_content)?;

        info!(key = %change.key, value = %change.value, "Config change applied");
        Ok(())
    }

    /// Run a benchmark using the current config and store results under `run_id`.
    /// Returns the run_id used.
    pub async fn run_benchmark(&self, run_id: &str) -> Result<String> {
        let config = Config::load_from(&self.config_path)?;

        // Reuse the benchmark runner logic inline — load tasks, run agent, score
        let benchmarks_dir = self
            .config_path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("benchmarks");

        let tasks = load_benchmark_tasks(&benchmarks_dir)?;
        let llm = zeus_llm::LlmClient::from_config(&config)?;
        let workspace = zeus_memory::Workspace::from_config(&config);
        workspace.init().await?;

        for task in &tasks {
            let session = zeus_session::Session::new(&config.sessions);
            let mut agent =
                zeus_agent::Agent::new(config.clone(), llm.clone(), workspace.clone(), session, None);

            let start = std::time::Instant::now();
            let run_result = tokio::time::timeout(
                std::time::Duration::from_secs(task.timeout),
                agent.run(&task.task),
            )
            .await;
            let duration_ms = start.elapsed().as_millis() as u64;

            let (output, timed_out) = match run_result {
                Ok(Ok(output)) => (output, false),
                Ok(Err(e)) => (format!("ERROR: {}", e), false),
                Err(_) => (format!("TIMEOUT after {}s", task.timeout), true),
            };

            let (passed, score) = if timed_out {
                (false, 0.0)
            } else {
                score_output(&output, &task.expected)
            };

            let result = crate::benchmark::BenchmarkResult {
                task_name: task.name.clone(),
                passed,
                score,
                duration_ms,
                tokens: 0,
                timestamp: chrono::Utc::now(),
                metadata: None,
            };

            self.store.store_result(run_id, &result)?;
        }

        Ok(run_id.to_string())
    }

    /// Compare two runs and decide whether the candidate is better.
    pub fn compare(&self, baseline_id: &str, candidate_id: &str) -> Result<RunComparison> {
        self.store.compare_runs(baseline_id, candidate_id)
    }

    /// Run a single experiment cycle: snapshot → apply change → benchmark → compare → keep/revert.
    pub async fn run_experiment(&mut self, change: ConfigChange) -> Result<ExperimentOutcome> {
        // 1. Snapshot
        self.snapshot_config()?;

        // 2. Run baseline benchmark with current config
        let baseline_id = format!("baseline-{}", ulid::Ulid::new());
        info!(run_id = %baseline_id, "Running baseline benchmark");
        if let Err(e) = self.run_benchmark(&baseline_id).await {
            return Ok(ExperimentOutcome::Failed {
                change,
                error: format!("Baseline benchmark failed: {}", e),
            });
        }

        // 3. Apply the candidate config change
        if let Err(e) = self.apply_change(&change) {
            self.restore_config()?;
            return Ok(ExperimentOutcome::Failed {
                change,
                error: format!("Failed to apply change: {}", e),
            });
        }

        // 4. Run candidate benchmark with modified config
        let candidate_id = format!("candidate-{}", ulid::Ulid::new());
        info!(run_id = %candidate_id, "Running candidate benchmark");
        let bench_result = self.run_benchmark(&candidate_id).await;
        if let Err(e) = bench_result {
            // Benchmark failed — revert
            self.restore_config()?;
            return Ok(ExperimentOutcome::Failed {
                change,
                error: format!("Candidate benchmark failed: {}", e),
            });
        }

        // 5. Compare
        let comparison = match self.compare(&baseline_id, &candidate_id) {
            Ok(c) => c,
            Err(e) => {
                self.restore_config()?;
                return Ok(ExperimentOutcome::Failed {
                    change,
                    error: format!("Comparison failed: {}", e),
                });
            }
        };

        // 6. Decide: keep if pass_rate improved and no regressions
        let dominated = comparison.pass_rate_delta > 0.0 && comparison.regressions.is_empty();
        let neutral_but_faster =
            comparison.pass_rate_delta >= 0.0
                && comparison.regressions.is_empty()
                && comparison.avg_duration_delta_ms > 0.0;

        if dominated || neutral_but_faster {
            info!(
                pass_rate_delta = comparison.pass_rate_delta,
                duration_delta_ms = comparison.avg_duration_delta_ms,
                "Keeping config change: {}",
                change.description
            );
            Ok(ExperimentOutcome::Kept { change, comparison })
        } else {
            let reason = if !comparison.regressions.is_empty() {
                format!(
                    "Regressions on: {}",
                    comparison.regressions.join(", ")
                )
            } else if comparison.pass_rate_delta < 0.0 {
                format!(
                    "Pass rate decreased by {:.1}%",
                    comparison.pass_rate_delta.abs() * 100.0
                )
            } else {
                "No improvement detected".to_string()
            };
            warn!(reason = %reason, "Reverting config change: {}", change.description);
            self.restore_config()?;
            Ok(ExperimentOutcome::Reverted {
                change,
                comparison,
                reason,
            })
        }
    }

    /// Run a full auto-tune cycle with a predefined set of experiments.
    ///
    /// Each experiment is run sequentially. If a change is kept, subsequent
    /// experiments run against the new baseline.
    pub async fn auto_tune(&mut self) -> Result<AutoTuneReport> {
        let changes = default_experiment_changes();
        let mut experiments = Vec::new();
        let mut kept = 0;
        let mut reverted = 0;
        let mut failed = 0;

        println!("Auto-tune: {} experiments queued\n", changes.len());

        for (i, change) in changes.into_iter().enumerate() {
            println!(
                "[{}/] Experiment: {}",
                i + 1,
                change.description
            );

            match self.run_experiment(change).await? {
                outcome @ ExperimentOutcome::Kept { .. } => {
                    println!("  → ✅ KEPT\n");
                    kept += 1;
                    experiments.push(outcome);
                }
                ExperimentOutcome::Reverted { change, comparison, reason } => {
                    println!("  → ↩️  REVERTED: {}\n", reason);
                    reverted += 1;
                    experiments.push(ExperimentOutcome::Reverted { change, comparison, reason });
                }
                ExperimentOutcome::Failed { change, error } => {
                    println!("  → ❌ FAILED: {}\n", error);
                    failed += 1;
                    experiments.push(ExperimentOutcome::Failed { change, error });
                }
            }
        }

        let report = AutoTuneReport {
            experiments,
            kept_count: kept,
            reverted_count: reverted,
            failed_count: failed,
        };

        println!("─────────────────────────");
        println!(
            "Auto-tune complete: {} kept, {} reverted, {} failed",
            report.kept_count, report.reverted_count, report.failed_count
        );

        Ok(report)
    }

    /// Get a reference to the benchmark store.
    pub fn store(&self) -> &BenchmarkStore {
        &self.store
    }
}

// ---------------------------------------------------------------------------
// Default experiment changes
// ---------------------------------------------------------------------------

/// Returns a set of config changes to try during auto-tune.
pub(crate) fn default_experiment_changes() -> Vec<ConfigChange> {
    vec![
        ConfigChange {
            description: "Increase max_iterations from default to 25".to_string(),
            key: "max_iterations".to_string(),
            value: "25".to_string(),
        },
        ConfigChange {
            description: "Try thinking_level = \"high\" for better reasoning".to_string(),
            key: "thinking_level".to_string(),
            value: "\"high\"".to_string(),
        },
        ConfigChange {
            description: "Reduce max_subagent_iterations to 10 for speed".to_string(),
            key: "max_subagent_iterations".to_string(),
            value: "10".to_string(),
        },
    ]
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Atomic write: write to a temp file then rename.
fn atomic_write(path: &PathBuf, content: &str) -> Result<()> {
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, content).map_err(|e| Error::Config(format!("write tmp: {}", e)))?;
    std::fs::rename(&tmp, path).map_err(|e| Error::Config(format!("rename: {}", e)))?;
    Ok(())
}

/// Score an agent's output against the expected string.
fn score_output(output: &str, expected: &str) -> (bool, f64) {
    let output_lower = output.to_lowercase();
    let expected_lower = expected.to_lowercase();
    let passed = output_lower.contains(&expected_lower);
    (passed, if passed { 1.0 } else { 0.0 })
}

/// Load benchmark tasks from a directory (same as benchmark.rs loader).
fn load_benchmark_tasks(dir: &PathBuf) -> Result<Vec<BenchmarkTask>> {
    let mut tasks = Vec::new();

    if !dir.exists() {
        // Return defaults
        return Ok(default_benchmark_tasks());
    }

    for entry in
        std::fs::read_dir(dir).map_err(|e| Error::Config(format!("read benchmarks dir: {}", e)))?
    {
        let entry = entry.map_err(|e| Error::Config(e.to_string()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            let content =
                std::fs::read_to_string(&path).map_err(|e| Error::Config(e.to_string()))?;
            if let Ok(file) = toml::from_str::<BenchmarkFile>(&content) {
                tasks.extend(file.tasks);
            }
        }
    }

    if tasks.is_empty() {
        tasks = default_benchmark_tasks();
    }
    Ok(tasks)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BenchmarkTask {
    pub name: String,
    pub task: String,
    pub expected: String,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_timeout() -> u64 {
    60
}

#[derive(Debug, Deserialize)]
struct BenchmarkFile {
    #[serde(rename = "task")]
    tasks: Vec<BenchmarkTask>,
}

fn default_benchmark_tasks() -> Vec<BenchmarkTask> {
    vec![
        BenchmarkTask {
            name: "arithmetic".to_string(),
            task: "What is 7 * 8? Reply with just the number.".to_string(),
            expected: "56".to_string(),
            timeout: 30,
        },
        BenchmarkTask {
            name: "reasoning".to_string(),
            task: "If a train travels 120 miles in 2 hours, what is its average speed in mph? Reply with just the number.".to_string(),
            expected: "60".to_string(),
            timeout: 30,
        },
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_env() -> (TempDir, PathBuf, PathBuf) {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");
        let db_path = dir.path().join("benchmarks.db");

        // Write a minimal valid config
        let config_content = r#"
model = "ollama/llama3.2"
workspace = "/tmp/zeus-test-workspace"
sessions = "/tmp/zeus-test-sessions"
max_iterations = 20
onboarding_complete = true
"#;
        std::fs::write(&config_path, config_content).unwrap();
        (dir, config_path, db_path)
    }

    #[test]
    fn test_snapshot_and_restore() {
        let (_dir, config_path, db_path) = setup_test_env();
        let mut exp = ConfigExperiment::with_paths(config_path.clone(), db_path).unwrap();

        let original = std::fs::read_to_string(&config_path).unwrap();
        exp.snapshot_config().unwrap();

        // Modify config
        std::fs::write(&config_path, "model = \"openai/gpt-4o\"\n").unwrap();
        assert_ne!(std::fs::read_to_string(&config_path).unwrap(), original);

        // Restore
        exp.restore_config().unwrap();
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), original);
    }

    #[test]
    fn test_apply_change_top_level() {
        let (_dir, config_path, db_path) = setup_test_env();
        let exp = ConfigExperiment::with_paths(config_path.clone(), db_path).unwrap();

        let change = ConfigChange {
            description: "Bump max_iterations".to_string(),
            key: "max_iterations".to_string(),
            value: "30".to_string(),
        };

        exp.apply_change(&change).unwrap();

        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("max_iterations = 30"));
    }

    #[test]
    fn test_apply_change_nested() {
        let (_dir, config_path, db_path) = setup_test_env();
        let exp = ConfigExperiment::with_paths(config_path.clone(), db_path).unwrap();

        let change = ConfigChange {
            description: "Set ollama URL".to_string(),
            key: "ollama.url".to_string(),
            value: "\"http://localhost:11434\"".to_string(),
        };

        exp.apply_change(&change).unwrap();

        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("[ollama]"));
        assert!(content.contains("http://localhost:11434"));
    }

    #[test]
    fn test_restore_without_snapshot_errors() {
        let (_dir, config_path, db_path) = setup_test_env();
        let exp = ConfigExperiment::with_paths(config_path, db_path).unwrap();
        assert!(exp.restore_config().is_err());
    }

    #[test]
    fn test_score_output() {
        assert_eq!(score_output("The answer is 56", "56"), (true, 1.0));
        assert_eq!(score_output("The answer is 57", "56"), (false, 0.0));
        assert_eq!(score_output("HELLO WORLD", "hello"), (true, 1.0));
    }

    #[test]
    fn test_default_experiment_changes() {
        let changes = default_experiment_changes();
        assert!(changes.len() >= 2);
        assert!(changes.iter().any(|c| c.key == "max_iterations"));
    }
}
