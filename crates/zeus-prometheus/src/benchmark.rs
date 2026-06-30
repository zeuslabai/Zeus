//! Benchmark results tracking and persistence.
//!
//! Stores benchmark run results in SQLite at `~/.zeus/benchmarks.db`.
//! Provides storage, retrieval, and run-over-run comparison.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use zeus_core::{Error, Result};

use crate::db::run_migrations;

// ---------------------------------------------------------------------------
// Schema migrations
// ---------------------------------------------------------------------------

const MIGRATIONS: &[&str] = &[
    // v1: initial schema
    "CREATE TABLE IF NOT EXISTS benchmark_results (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        run_id      TEXT    NOT NULL,
        task_name   TEXT    NOT NULL,
        passed      INTEGER NOT NULL DEFAULT 0,
        score       REAL    NOT NULL DEFAULT 0.0,
        duration_ms INTEGER NOT NULL DEFAULT 0,
        tokens      INTEGER NOT NULL DEFAULT 0,
        timestamp   TEXT    NOT NULL,
        metadata    TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_bench_run    ON benchmark_results(run_id);
    CREATE INDEX IF NOT EXISTS idx_bench_task   ON benchmark_results(task_name);
    CREATE INDEX IF NOT EXISTS idx_bench_ts     ON benchmark_results(timestamp);",
];

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single benchmark task result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    /// Name / identifier of the task.
    pub task_name: String,
    /// Whether the task passed.
    pub passed: bool,
    /// Numeric score (0.0–1.0). 1.0 = perfect match.
    pub score: f64,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Tokens consumed (prompt + completion).
    pub tokens: u64,
    /// When this result was recorded.
    pub timestamp: DateTime<Utc>,
    /// Optional JSON metadata blob.
    pub metadata: Option<String>,
}

/// Summary of a full benchmark run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub run_id: String,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub pass_rate: f64,
    pub avg_duration_ms: f64,
    pub total_tokens: u64,
    pub timestamp: DateTime<Utc>,
}

/// Comparison between two runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunComparison {
    pub baseline: RunSummary,
    pub candidate: RunSummary,
    /// Positive = improvement.
    pub pass_rate_delta: f64,
    /// Positive = faster.
    pub avg_duration_delta_ms: f64,
    /// Per-task regressions (tasks that passed in baseline but failed in candidate).
    pub regressions: Vec<String>,
    /// Per-task improvements (tasks that failed in baseline but passed in candidate).
    pub improvements: Vec<String>,
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Persistent benchmark result store backed by SQLite.
pub struct BenchmarkStore {
    conn: Connection,
    #[allow(dead_code)]
    db_path: PathBuf,
}

impl BenchmarkStore {
    /// Open (or create) the benchmark database at `path`.
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Database(format!("mkdir {}: {}", parent.display(), e)))?;
        }
        let conn = Connection::open(path)
            .map_err(|e| Error::Database(format!("open {}: {}", path.display(), e)))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| Error::Database(e.to_string()))?;
        run_migrations(&conn, MIGRATIONS)?;
        Ok(Self {
            conn,
            db_path: path.to_path_buf(),
        })
    }

    /// Open the default database at `~/.zeus/benchmarks.db`.
    pub fn open_default() -> Result<Self> {
        let path = dirs::home_dir()
            .unwrap_or_default()
            .join(".zeus")
            .join("benchmarks.db");
        Self::new(&path)
    }

    // -- writes ---------------------------------------------------------------

    /// Store a single result for a given run.
    pub fn store_result(&self, run_id: &str, result: &BenchmarkResult) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO benchmark_results
                    (run_id, task_name, passed, score, duration_ms, tokens, timestamp, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    run_id,
                    result.task_name,
                    result.passed as i32,
                    result.score,
                    result.duration_ms as i64,
                    result.tokens as i64,
                    result.timestamp.to_rfc3339(),
                    result.metadata,
                ],
            )
            .map_err(|e| Error::Database(format!("store_result: {}", e)))?;
        Ok(())
    }

    /// Store a batch of results for a run.
    pub fn store_run(&self, run_id: &str, results: &[BenchmarkResult]) -> Result<()> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| Error::Database(e.to_string()))?;
        for r in results {
            tx.execute(
                "INSERT INTO benchmark_results
                    (run_id, task_name, passed, score, duration_ms, tokens, timestamp, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    run_id,
                    r.task_name,
                    r.passed as i32,
                    r.score,
                    r.duration_ms as i64,
                    r.tokens as i64,
                    r.timestamp.to_rfc3339(),
                    r.metadata,
                ],
            )
            .map_err(|e| Error::Database(format!("store_run: {}", e)))?;
        }
        tx.commit()
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(())
    }

    // -- reads ----------------------------------------------------------------

    /// Get all results for a specific run.
    pub fn get_results(&self, run_id: &str) -> Result<Vec<BenchmarkResult>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT task_name, passed, score, duration_ms, tokens, timestamp, metadata
                 FROM benchmark_results WHERE run_id = ?1 ORDER BY id",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![run_id], |row| {
                Ok(BenchmarkResult {
                    task_name: row.get(0)?,
                    passed: row.get::<_, i32>(1)? != 0,
                    score: row.get(2)?,
                    duration_ms: row.get::<_, i64>(3)? as u64,
                    tokens: row.get::<_, i64>(4)? as u64,
                    timestamp: row
                        .get::<_, String>(5)
                        .ok()
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(Utc::now),
                    metadata: row.get(6)?,
                })
            })
            .map_err(|e| Error::Database(e.to_string()))?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(e.to_string()))
    }

    /// Summarize a run.
    pub fn summarize_run(&self, run_id: &str) -> Result<RunSummary> {
        let results = self.get_results(run_id)?;
        if results.is_empty() {
            return Err(Error::Database(format!("No results for run '{}'", run_id)));
        }
        let total = results.len();
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = total - passed;
        let pass_rate = passed as f64 / total as f64;
        let avg_duration_ms =
            results.iter().map(|r| r.duration_ms as f64).sum::<f64>() / total as f64;
        let total_tokens = results.iter().map(|r| r.tokens).sum();
        let timestamp = results
            .first()
            .map(|r| r.timestamp)
            .unwrap_or_else(Utc::now);

        Ok(RunSummary {
            run_id: run_id.to_string(),
            total,
            passed,
            failed,
            pass_rate,
            avg_duration_ms,
            total_tokens,
            timestamp,
        })
    }

    /// List all run IDs, most recent first.
    pub fn list_runs(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT run_id, MIN(timestamp) as first_ts FROM benchmark_results
                 GROUP BY run_id ORDER BY first_ts DESC",
            )
            .map_err(|e| Error::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| Error::Database(e.to_string()))?;

        rows.collect::<std::result::Result<Vec<String>, _>>()
            .map_err(|e| Error::Database(e.to_string()))
    }

    /// Compare two runs (baseline vs candidate).
    pub fn compare_runs(&self, baseline_id: &str, candidate_id: &str) -> Result<RunComparison> {
        let baseline = self.summarize_run(baseline_id)?;
        let candidate = self.summarize_run(candidate_id)?;

        let baseline_results = self.get_results(baseline_id)?;
        let candidate_results = self.get_results(candidate_id)?;

        // Build task→passed maps
        let baseline_map: std::collections::HashMap<&str, bool> = baseline_results
            .iter()
            .map(|r| (r.task_name.as_str(), r.passed))
            .collect();
        let candidate_map: std::collections::HashMap<&str, bool> = candidate_results
            .iter()
            .map(|r| (r.task_name.as_str(), r.passed))
            .collect();

        let mut regressions = Vec::new();
        let mut improvements = Vec::new();

        // Check baseline tasks for regressions
        for (task, &b_passed) in &baseline_map {
            match candidate_map.get(task) {
                Some(&c_passed) if b_passed && !c_passed => {
                    regressions.push(task.to_string());
                }
                Some(&c_passed) if !b_passed && c_passed => {
                    improvements.push(task.to_string());
                }
                _ => {}
            }
        }

        // Check for new tasks that passed in candidate but weren't in baseline
        for (task, &c_passed) in &candidate_map {
            if c_passed && !baseline_map.contains_key(task) {
                improvements.push(task.to_string());
            }
        }

        Ok(RunComparison {
            pass_rate_delta: candidate.pass_rate - baseline.pass_rate,
            avg_duration_delta_ms: baseline.avg_duration_ms - candidate.avg_duration_ms,
            baseline,
            candidate,
            regressions,
            improvements,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store() -> (BenchmarkStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("bench.db");
        let store = BenchmarkStore::new(&db).unwrap();
        (store, dir)
    }

    fn make_result(name: &str, passed: bool) -> BenchmarkResult {
        BenchmarkResult {
            task_name: name.to_string(),
            passed,
            score: if passed { 1.0 } else { 0.0 },
            duration_ms: 150,
            tokens: 500,
            timestamp: Utc::now(),
            metadata: None,
        }
    }

    #[test]
    fn test_store_and_retrieve() {
        let (store, _dir) = test_store();
        let r = make_result("greet", true);
        store.store_result("run-1", &r).unwrap();

        let results = store.get_results("run-1").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].task_name, "greet");
        assert!(results[0].passed);
    }

    #[test]
    fn test_store_run_batch() {
        let (store, _dir) = test_store();
        let results = vec![
            make_result("task_a", true),
            make_result("task_b", false),
            make_result("task_c", true),
        ];
        store.store_run("run-2", &results).unwrap();

        let summary = store.summarize_run("run-2").unwrap();
        assert_eq!(summary.total, 3);
        assert_eq!(summary.passed, 2);
        assert_eq!(summary.failed, 1);
        assert!((summary.pass_rate - 2.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn test_list_runs() {
        let (store, _dir) = test_store();
        store.store_result("alpha", &make_result("t1", true)).unwrap();
        store.store_result("beta", &make_result("t1", false)).unwrap();

        let runs = store.list_runs().unwrap();
        assert_eq!(runs.len(), 2);
    }

    #[test]
    fn test_compare_runs() {
        let (store, _dir) = test_store();
        store
            .store_run(
                "baseline",
                &[
                    make_result("a", true),
                    make_result("b", true),
                    make_result("c", false),
                ],
            )
            .unwrap();
        store
            .store_run(
                "candidate",
                &[
                    make_result("a", true),
                    make_result("b", false), // regression
                    make_result("c", true),  // improvement
                ],
            )
            .unwrap();

        let cmp = store.compare_runs("baseline", "candidate").unwrap();
        assert_eq!(cmp.regressions, vec!["b"]);
        assert_eq!(cmp.improvements, vec!["c"]);
        assert!((cmp.pass_rate_delta).abs() < 0.001); // same pass rate (2/3 each)
    }

    #[test]
    fn test_empty_run_errors() {
        let (store, _dir) = test_store();
        assert!(store.summarize_run("nonexistent").is_err());
    }
}
