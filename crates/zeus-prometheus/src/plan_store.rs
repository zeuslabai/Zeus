//! PlanOutcomeStore — SQLite persistence for plan execution outcomes.
//!
//! Stores completed plans so the system can learn which decompositions and
//! tool combinations work best for different task types.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

/// A persisted plan outcome for learning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanOutcome {
    /// Unique outcome ID
    pub id: String,
    /// Original task description
    pub task: String,
    /// Number of steps in the plan
    pub step_count: usize,
    /// Steps that succeeded
    pub steps_succeeded: usize,
    /// Steps that failed
    pub steps_failed: usize,
    /// Steps that were skipped
    pub steps_skipped: usize,
    /// Tools used (comma-separated)
    pub tools_used: String,
    /// Final plan status
    pub status: String,
    /// Total execution time in ms
    pub total_time_ms: u64,
    /// Number of replans that occurred
    pub replan_count: usize,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

const PLAN_MIGRATIONS: &[&str] = &[
    // v1: initial schema
    "CREATE TABLE IF NOT EXISTS plan_outcomes (
                id TEXT PRIMARY KEY,
                task TEXT NOT NULL,
                step_count INTEGER NOT NULL,
                steps_succeeded INTEGER NOT NULL,
                steps_failed INTEGER NOT NULL,
                steps_skipped INTEGER NOT NULL,
                tools_used TEXT NOT NULL,
                status TEXT NOT NULL,
                total_time_ms INTEGER NOT NULL,
                replan_count INTEGER NOT NULL DEFAULT 0,
                timestamp TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_plan_outcomes_status ON plan_outcomes(status);
            CREATE INDEX IF NOT EXISTS idx_plan_outcomes_task ON plan_outcomes(task);",
];

/// SQLite-backed store for plan outcomes
pub struct PlanOutcomeStore {
    conn: Mutex<Connection>,
}

impl PlanOutcomeStore {
    /// Open or create the plan outcome store at the given path
    pub fn new(db_path: &Path) -> Result<Self, String> {
        let conn =
            Connection::open(db_path).map_err(|e| format!("Failed to open plan store: {}", e))?;

        crate::db::run_migrations(&conn, PLAN_MIGRATIONS)
            .map_err(|e| format!("Plan schema migration failed: {e}"))?;

        tracing::debug!("PlanOutcomeStore opened at {}", db_path.display());

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Record a plan outcome
    pub fn record(&self, outcome: &PlanOutcome) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        conn.execute(
            "INSERT OR REPLACE INTO plan_outcomes (id, task, step_count, steps_succeeded, steps_failed, steps_skipped, tools_used, status, total_time_ms, replan_count, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                outcome.id,
                outcome.task,
                outcome.step_count as i64,
                outcome.steps_succeeded as i64,
                outcome.steps_failed as i64,
                outcome.steps_skipped as i64,
                outcome.tools_used,
                outcome.status,
                outcome.total_time_ms as i64,
                outcome.replan_count as i64,
                outcome.timestamp.to_rfc3339(),
            ],
        )
        .map_err(|e| format!("Failed to record outcome: {}", e))?;
        Ok(())
    }

    /// Query outcomes for similar tasks (substring match)
    pub fn query_similar(
        &self,
        task_fragment: &str,
        limit: usize,
    ) -> Result<Vec<PlanOutcome>, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, task, step_count, steps_succeeded, steps_failed, steps_skipped, tools_used, status, total_time_ms, replan_count, timestamp FROM plan_outcomes WHERE task LIKE ?1 ORDER BY timestamp DESC LIMIT ?2",
            )
            .map_err(|e| format!("Query error: {}", e))?;

        let pattern = format!("%{}%", task_fragment);
        let rows = stmt
            .query_map(params![pattern, limit as i64], |row| {
                Ok(PlanOutcome {
                    id: row.get(0)?,
                    task: row.get(1)?,
                    step_count: row.get::<_, i64>(2)? as usize,
                    steps_succeeded: row.get::<_, i64>(3)? as usize,
                    steps_failed: row.get::<_, i64>(4)? as usize,
                    steps_skipped: row.get::<_, i64>(5)? as usize,
                    tools_used: row.get(6)?,
                    status: row.get(7)?,
                    total_time_ms: row.get::<_, i64>(8)? as u64,
                    replan_count: row.get::<_, i64>(9)? as usize,
                    timestamp: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(10)?)
                        .unwrap_or_default()
                        .with_timezone(&Utc),
                })
            })
            .map_err(|e| format!("Query error: {}", e))?;

        let mut results = Vec::new();
        for outcome in rows.flatten() {
            results.push(outcome);
        }
        Ok(results)
    }

    /// Get success rate for tasks containing a keyword
    pub fn success_rate(&self, keyword: &str) -> Result<f64, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let pattern = format!("%{}%", keyword);
        let (total, succeeded): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), SUM(CASE WHEN status = 'Completed' THEN 1 ELSE 0 END) FROM plan_outcomes WHERE task LIKE ?1",
                params![pattern],
                |row| Ok((row.get(0)?, row.get::<_, Option<i64>>(1)?.unwrap_or(0))),
            )
            .map_err(|e| format!("Query error: {}", e))?;

        if total == 0 {
            Ok(0.0)
        } else {
            Ok(succeeded as f64 / total as f64)
        }
    }

    /// Get total number of recorded outcomes
    pub fn count(&self) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| format!("Lock error: {}", e))?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM plan_outcomes", [], |row| row.get(0))
            .map_err(|e| format!("Query error: {}", e))?;
        Ok(count as usize)
    }

    /// Build a PlanOutcome from an ExecutionResult
    pub fn outcome_from_result(
        task: &str,
        result: &crate::executor::ExecutionResult,
        replan_count: usize,
    ) -> PlanOutcome {
        let steps_succeeded = result.step_results.iter().filter(|r| r.success).count();
        let steps_skipped = result
            .step_results
            .iter()
            .filter(|r| r.error.as_deref() == Some("Skipped: dependency failed"))
            .count();
        let steps_failed = result
            .step_results
            .iter()
            .filter(|r| !r.success && r.error.as_deref() != Some("Skipped: dependency failed"))
            .count();
        let tools_used: Vec<String> = result
            .step_results
            .iter()
            .flat_map(|r| r.tool_calls_executed.iter().map(|tc| tc.name.clone()))
            .collect();

        PlanOutcome {
            id: uuid::Uuid::new_v4().to_string(),
            task: task.to_string(),
            step_count: result.step_results.len(),
            steps_succeeded,
            steps_failed,
            steps_skipped,
            tools_used: tools_used.join(","),
            status: format!("{:?}", result.plan_status),
            total_time_ms: result.total_time_ms,
            replan_count,
            timestamp: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_store() -> (PlanOutcomeStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("plan_store.db");
        let store = PlanOutcomeStore::new(&db_path).unwrap();
        (store, dir)
    }

    fn sample_outcome(task: &str, status: &str) -> PlanOutcome {
        PlanOutcome {
            id: uuid::Uuid::new_v4().to_string(),
            task: task.to_string(),
            step_count: 3,
            steps_succeeded: 2,
            steps_failed: 1,
            steps_skipped: 0,
            tools_used: "shell,read_file".to_string(),
            status: status.to_string(),
            total_time_ms: 5000,
            replan_count: 0,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_store_new() {
        let (store, _dir) = temp_store();
        assert_eq!(store.count().unwrap(), 0);
    }

    #[test]
    fn test_record_and_count() {
        let (store, _dir) = temp_store();
        let outcome = sample_outcome("deploy app", "Completed");
        store.record(&outcome).unwrap();
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn test_record_multiple() {
        let (store, _dir) = temp_store();
        store
            .record(&sample_outcome("task 1", "Completed"))
            .unwrap();
        store.record(&sample_outcome("task 2", "Failed")).unwrap();
        store
            .record(&sample_outcome("task 3", "Completed"))
            .unwrap();
        assert_eq!(store.count().unwrap(), 3);
    }

    #[test]
    fn test_query_similar() {
        let (store, _dir) = temp_store();
        store
            .record(&sample_outcome("deploy web app", "Completed"))
            .unwrap();
        store
            .record(&sample_outcome("deploy api server", "Failed"))
            .unwrap();
        store
            .record(&sample_outcome("run tests", "Completed"))
            .unwrap();

        let results = store.query_similar("deploy", 10).unwrap();
        assert_eq!(results.len(), 2);

        let results = store.query_similar("tests", 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_query_similar_no_match() {
        let (store, _dir) = temp_store();
        store
            .record(&sample_outcome("deploy app", "Completed"))
            .unwrap();
        let results = store.query_similar("nonexistent", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_query_similar_limit() {
        let (store, _dir) = temp_store();
        for i in 0..10 {
            store
                .record(&sample_outcome(&format!("task {}", i), "Completed"))
                .unwrap();
        }
        let results = store.query_similar("task", 3).unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_success_rate_all_success() {
        let (store, _dir) = temp_store();
        store
            .record(&sample_outcome("build project", "Completed"))
            .unwrap();
        store
            .record(&sample_outcome("build app", "Completed"))
            .unwrap();
        let rate = store.success_rate("build").unwrap();
        assert!((rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_success_rate_mixed() {
        let (store, _dir) = temp_store();
        store
            .record(&sample_outcome("deploy alpha", "Completed"))
            .unwrap();
        store
            .record(&sample_outcome("deploy beta", "Failed"))
            .unwrap();
        let rate = store.success_rate("deploy").unwrap();
        assert!((rate - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_success_rate_no_matches() {
        let (store, _dir) = temp_store();
        let rate = store.success_rate("unknown").unwrap();
        assert!((rate - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_outcome_serialization() {
        let outcome = sample_outcome("test task", "Completed");
        let json = serde_json::to_string(&outcome).unwrap();
        let deser: PlanOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.task, "test task");
        assert_eq!(deser.status, "Completed");
        assert_eq!(deser.step_count, 3);
    }

    #[test]
    fn test_outcome_from_result() {
        use crate::executor::{ExecutionResult, StepResult, StepToolCall};
        use crate::planner::PlanStatus;

        let exec_result = ExecutionResult {
            plan_status: PlanStatus::Completed,
            step_results: vec![
                StepResult {
                    step_id: 1,
                    success: true,
                    output: "ok".to_string(),
                    error: None,
                    retries: 0,
                    tool_calls_executed: vec![StepToolCall {
                        name: "shell".to_string(),
                        call_id: "tc1".to_string(),
                        arguments: serde_json::json!({}),
                        success: true,
                        output: "ok".to_string(),
                    }],
                },
                StepResult {
                    step_id: 2,
                    success: false,
                    output: String::new(),
                    error: Some("timeout".to_string()),
                    retries: 2,
                    tool_calls_executed: vec![],
                },
            ],
            total_time_ms: 3000,
        };

        let outcome = PlanOutcomeStore::outcome_from_result("test task", &exec_result, 1);
        assert_eq!(outcome.task, "test task");
        assert_eq!(outcome.step_count, 2);
        assert_eq!(outcome.steps_succeeded, 1);
        assert_eq!(outcome.steps_failed, 1);
        assert_eq!(outcome.steps_skipped, 0);
        assert_eq!(outcome.tools_used, "shell");
        assert_eq!(outcome.replan_count, 1);
        assert_eq!(outcome.total_time_ms, 3000);
    }

    #[test]
    fn test_record_replace() {
        let (store, _dir) = temp_store();
        let mut outcome = sample_outcome("task", "Failed");
        let id = outcome.id.clone();
        store.record(&outcome).unwrap();

        // Update same ID
        outcome.status = "Completed".to_string();
        store.record(&outcome).unwrap();

        assert_eq!(store.count().unwrap(), 1);
        let results = store.query_similar("task", 1).unwrap();
        assert_eq!(results[0].status, "Completed");
        assert_eq!(results[0].id, id);
    }
}
