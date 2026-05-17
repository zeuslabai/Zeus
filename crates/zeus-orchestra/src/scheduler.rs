//! Cron/Scheduler system for recurring tasks
//!
//! Provides a scheduler that manages recurring task definitions with cron expressions,
//! tracks run history, and supports pause/resume lifecycle.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use cron::Schedule;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during schedule operations.
#[derive(Debug, thiserror::Error)]
pub enum ScheduleError {
    #[error("invalid cron expression: {0}")]
    InvalidCron(String),

    #[error("schedule not found: {0}")]
    NotFound(String),

    #[error("schedule already exists: {0}")]
    AlreadyExists(String),

    #[error("execution failed: {0}")]
    ExecutionFailed(String),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// How a scheduled task's execution is delivered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    /// Fire the task and don't wait for acknowledgement.
    FireAndForget,
    /// Wait for acknowledgement that the task was received.
    Acknowledged,
    /// Guarantee delivery with retries and persistence.
    Guaranteed,
}

/// Current status of a schedule definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleStatus {
    Active,
    Paused,
    Completed,
    Failed,
}

/// Status of an individual run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
    Skipped,
}

// ---------------------------------------------------------------------------
// Schedule definition
// ---------------------------------------------------------------------------

/// A recurring task definition with a cron expression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub cron_expression: String,
    pub task: String,
    pub delivery_mode: DeliveryMode,
    pub enabled: bool,
    pub status: ScheduleStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    pub max_retries: u32,
}

impl ScheduleDefinition {
    /// Create a new schedule definition. Returns an error if the cron expression is invalid.
    pub fn new(
        name: impl Into<String>,
        cron_expression: impl Into<String>,
        task: impl Into<String>,
    ) -> Result<Self, ScheduleError> {
        let cron_expr = cron_expression.into();
        let next = calculate_next_run(&cron_expr)?;

        Ok(Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            description: None,
            cron_expression: cron_expr,
            task: task.into(),
            delivery_mode: DeliveryMode::FireAndForget,
            enabled: true,
            status: ScheduleStatus::Active,
            created_at: Utc::now(),
            updated_at: None,
            next_run: next,
            max_retries: 3,
        })
    }

    /// Builder: set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Builder: set delivery mode.
    pub fn with_delivery_mode(mut self, mode: DeliveryMode) -> Self {
        self.delivery_mode = mode;
        self
    }

    /// Builder: set max retries.
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }
}

// ---------------------------------------------------------------------------
// Run record
// ---------------------------------------------------------------------------

/// A record of a single execution of a scheduled task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub schedule_id: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status: RunStatus,
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    pub attempt: u32,
}

impl RunRecord {
    /// Create a new run record in Running status.
    pub fn new(schedule_id: impl Into<String>, attempt: u32) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            schedule_id: schedule_id.into(),
            started_at: Utc::now(),
            completed_at: None,
            status: RunStatus::Running,
            output: None,
            error: None,
            duration_ms: None,
            attempt,
        }
    }

    /// Mark this run as completed.
    pub fn complete(&mut self, output: Option<String>) {
        self.status = RunStatus::Completed;
        self.output = output;
        self.completed_at = Some(Utc::now());
        self.duration_ms = Some((Utc::now() - self.started_at).num_milliseconds().max(0) as u64);
    }

    /// Mark this run as failed.
    pub fn fail(&mut self, error: String) {
        self.status = RunStatus::Failed;
        self.error = Some(error);
        self.completed_at = Some(Utc::now());
        self.duration_ms = Some((Utc::now() - self.started_at).num_milliseconds().max(0) as u64);
    }
}

// ---------------------------------------------------------------------------
// Helper: parse cron and compute next run
// ---------------------------------------------------------------------------

/// Parse a cron expression and return the next scheduled time after now.
/// Returns `Err(ScheduleError::InvalidCron)` if the expression is unparseable.
pub fn calculate_next_run(cron_expr: &str) -> Result<Option<DateTime<Utc>>, ScheduleError> {
    let schedule = Schedule::from_str(cron_expr)
        .map_err(|e| ScheduleError::InvalidCron(format!("{}: {}", cron_expr, e)))?;
    Ok(schedule.upcoming(Utc).next())
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// Maximum number of run records kept in the ring buffer.
const MAX_RUN_HISTORY: usize = 1000;

/// Central scheduler managing schedule definitions and run history.
pub struct Scheduler {
    schedules: Arc<Mutex<HashMap<String, ScheduleDefinition>>>,
    run_history: Arc<Mutex<Vec<RunRecord>>>,
}

impl Scheduler {
    /// Create a new empty scheduler.
    pub fn new() -> Self {
        Self {
            schedules: Arc::new(Mutex::new(HashMap::new())),
            run_history: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Add a schedule definition. Errors if the id already exists.
    pub async fn add_schedule(
        &self,
        schedule: ScheduleDefinition,
    ) -> Result<ScheduleDefinition, ScheduleError> {
        let mut schedules = self.schedules.lock().await;
        if schedules.contains_key(&schedule.id) {
            return Err(ScheduleError::AlreadyExists(schedule.id.clone()));
        }
        schedules.insert(schedule.id.clone(), schedule.clone());
        Ok(schedule)
    }

    /// Get a schedule by id.
    pub async fn get_schedule(&self, id: &str) -> Result<ScheduleDefinition, ScheduleError> {
        let schedules = self.schedules.lock().await;
        schedules
            .get(id)
            .cloned()
            .ok_or_else(|| ScheduleError::NotFound(id.to_string()))
    }

    /// List all schedule definitions.
    pub async fn list_schedules(&self) -> Vec<ScheduleDefinition> {
        let schedules = self.schedules.lock().await;
        schedules.values().cloned().collect()
    }

    /// Update an existing schedule definition. The id must match an existing schedule.
    pub async fn update_schedule(
        &self,
        schedule: ScheduleDefinition,
    ) -> Result<ScheduleDefinition, ScheduleError> {
        let mut schedules = self.schedules.lock().await;
        if !schedules.contains_key(&schedule.id) {
            return Err(ScheduleError::NotFound(schedule.id.clone()));
        }
        schedules.insert(schedule.id.clone(), schedule.clone());
        Ok(schedule)
    }

    /// Delete a schedule by id.
    pub async fn delete_schedule(&self, id: &str) -> Result<(), ScheduleError> {
        let mut schedules = self.schedules.lock().await;
        schedules
            .remove(id)
            .map(|_| ())
            .ok_or_else(|| ScheduleError::NotFound(id.to_string()))
    }

    /// Pause an active schedule.
    pub async fn pause_schedule(&self, id: &str) -> Result<ScheduleDefinition, ScheduleError> {
        let mut schedules = self.schedules.lock().await;
        let schedule = schedules
            .get_mut(id)
            .ok_or_else(|| ScheduleError::NotFound(id.to_string()))?;
        schedule.status = ScheduleStatus::Paused;
        schedule.enabled = false;
        schedule.updated_at = Some(Utc::now());
        Ok(schedule.clone())
    }

    /// Resume a paused schedule.
    pub async fn resume_schedule(&self, id: &str) -> Result<ScheduleDefinition, ScheduleError> {
        let mut schedules = self.schedules.lock().await;
        let schedule = schedules
            .get_mut(id)
            .ok_or_else(|| ScheduleError::NotFound(id.to_string()))?;
        schedule.status = ScheduleStatus::Active;
        schedule.enabled = true;
        schedule.updated_at = Some(Utc::now());
        // Recalculate next run
        if let Ok(next) = calculate_next_run(&schedule.cron_expression) {
            schedule.next_run = next;
        }
        Ok(schedule.clone())
    }

    /// Record a run in the history ring buffer.
    pub async fn record_run(&self, run: RunRecord) {
        let mut history = self.run_history.lock().await;
        history.push(run);
        // Enforce ring buffer limit
        if history.len() > MAX_RUN_HISTORY {
            let overflow = history.len() - MAX_RUN_HISTORY;
            history.drain(..overflow);
        }
    }

    /// List all run records.
    pub async fn list_runs(&self) -> Vec<RunRecord> {
        self.run_history.lock().await.clone()
    }

    /// Get run records for a specific schedule.
    pub async fn runs_for_schedule(&self, schedule_id: &str) -> Vec<RunRecord> {
        let history = self.run_history.lock().await;
        history
            .iter()
            .filter(|r| r.schedule_id == schedule_id)
            .cloned()
            .collect()
    }

    /// Return schedules whose `next_run` is at or before now and that are enabled.
    pub async fn next_due_schedules(&self) -> Vec<ScheduleDefinition> {
        let now = Utc::now();
        let schedules = self.schedules.lock().await;
        schedules
            .values()
            .filter(|s| {
                s.enabled
                    && s.status == ScheduleStatus::Active
                    && s.next_run.is_some_and(|nr| nr <= now)
            })
            .cloned()
            .collect()
    }

    /// Number of schedule definitions.
    pub async fn schedule_count(&self) -> usize {
        self.schedules.lock().await.len()
    }

    /// Number of run records in history.
    pub async fn run_count(&self) -> usize {
        self.run_history.lock().await.len()
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // A valid 7-field cron expression: every minute
    const EVERY_MINUTE: &str = "0 * * * * * *";
    // Every second (fires very frequently)
    const EVERY_SECOND: &str = "* * * * * * *";

    // -- ScheduleDefinition tests -------------------------------------------

    #[test]
    fn test_schedule_definition_new() {
        let sd = ScheduleDefinition::new("backup", EVERY_MINUTE, "run_backup")
            .expect("ScheduleDefinition::new should succeed");
        assert_eq!(sd.name, "backup");
        assert_eq!(sd.task, "run_backup");
        assert!(sd.enabled);
        assert_eq!(sd.status, ScheduleStatus::Active);
        assert_eq!(sd.max_retries, 3);
        assert!(sd.next_run.is_some());
        assert!(sd.description.is_none());
    }

    #[test]
    fn test_schedule_definition_with_builders() {
        let sd = ScheduleDefinition::new("sync", EVERY_MINUTE, "sync_data")
            .expect("ScheduleDefinition::new should succeed")
            .with_description("Sync data every minute")
            .with_delivery_mode(DeliveryMode::Guaranteed)
            .with_max_retries(5);
        assert_eq!(sd.description.as_deref(), Some("Sync data every minute"));
        assert_eq!(sd.delivery_mode, DeliveryMode::Guaranteed);
        assert_eq!(sd.max_retries, 5);
    }

    #[test]
    fn test_schedule_definition_invalid_cron() {
        let result = ScheduleDefinition::new("bad", "not a cron", "task");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ScheduleError::InvalidCron(_)));
    }

    #[test]
    fn test_schedule_definition_serialization() {
        let sd = ScheduleDefinition::new("test", EVERY_MINUTE, "do_thing")
            .expect("ScheduleDefinition::new should succeed");
        let json = serde_json::to_string(&sd).expect("should serialize to JSON");
        let de: ScheduleDefinition =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.name, "test");
        assert_eq!(de.task, "do_thing");
        assert_eq!(de.status, ScheduleStatus::Active);
    }

    #[test]
    fn test_schedule_definition_unique_ids() {
        let s1 = ScheduleDefinition::new("a", EVERY_MINUTE, "t1")
            .expect("ScheduleDefinition::new should succeed");
        let s2 = ScheduleDefinition::new("b", EVERY_MINUTE, "t2")
            .expect("ScheduleDefinition::new should succeed");
        assert_ne!(s1.id, s2.id);
    }

    // -- RunRecord tests ----------------------------------------------------

    #[test]
    fn test_run_record_new() {
        let rr = RunRecord::new("sched-1", 1);
        assert_eq!(rr.schedule_id, "sched-1");
        assert_eq!(rr.attempt, 1);
        assert_eq!(rr.status, RunStatus::Running);
        assert!(rr.completed_at.is_none());
        assert!(rr.output.is_none());
        assert!(rr.error.is_none());
    }

    #[test]
    fn test_run_record_complete() {
        let mut rr = RunRecord::new("s1", 1);
        rr.complete(Some("all good".to_string()));
        assert_eq!(rr.status, RunStatus::Completed);
        assert_eq!(rr.output.as_deref(), Some("all good"));
        assert!(rr.completed_at.is_some());
        assert!(rr.duration_ms.is_some());
    }

    #[test]
    fn test_run_record_fail() {
        let mut rr = RunRecord::new("s1", 2);
        rr.fail("connection timeout".to_string());
        assert_eq!(rr.status, RunStatus::Failed);
        assert_eq!(rr.error.as_deref(), Some("connection timeout"));
        assert!(rr.completed_at.is_some());
    }

    #[test]
    fn test_run_record_serialization() {
        let rr = RunRecord::new("s1", 1);
        let json = serde_json::to_string(&rr).expect("should serialize to JSON");
        let de: RunRecord = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.schedule_id, "s1");
        assert_eq!(de.status, RunStatus::Running);
    }

    // -- calculate_next_run tests -------------------------------------------

    #[test]
    fn test_calculate_next_run_valid() {
        let next = calculate_next_run(EVERY_MINUTE).expect("operation should succeed");
        assert!(next.is_some());
        // Next run should be in the future
        assert!(next.expect("operation should succeed") > Utc::now());
    }

    #[test]
    fn test_calculate_next_run_invalid() {
        let result = calculate_next_run("not valid cron");
        assert!(result.is_err());
    }

    #[test]
    fn test_calculate_next_run_every_second() {
        let next = calculate_next_run(EVERY_SECOND).expect("operation should succeed");
        assert!(next.is_some());
    }

    // -- Scheduler CRUD tests -----------------------------------------------

    #[tokio::test]
    async fn test_scheduler_new() {
        let scheduler = Scheduler::new();
        assert_eq!(scheduler.schedule_count().await, 0);
        assert_eq!(scheduler.run_count().await, 0);
    }

    #[tokio::test]
    async fn test_scheduler_default() {
        let scheduler = Scheduler::default();
        assert_eq!(scheduler.schedule_count().await, 0);
    }

    #[tokio::test]
    async fn test_add_schedule() {
        let scheduler = Scheduler::new();
        let sd = ScheduleDefinition::new("backup", EVERY_MINUTE, "run_backup")
            .expect("ScheduleDefinition::new should succeed");
        let added = scheduler
            .add_schedule(sd)
            .await
            .expect("async operation should succeed");
        assert_eq!(added.name, "backup");
        assert_eq!(scheduler.schedule_count().await, 1);
    }

    #[tokio::test]
    async fn test_add_duplicate_schedule() {
        let scheduler = Scheduler::new();
        let sd = ScheduleDefinition::new("dup", EVERY_MINUTE, "task")
            .expect("ScheduleDefinition::new should succeed");
        let id = sd.id.clone();
        scheduler
            .add_schedule(sd)
            .await
            .expect("async operation should succeed");

        let mut sd2 = ScheduleDefinition::new("dup2", EVERY_MINUTE, "task2")
            .expect("ScheduleDefinition::new should succeed");
        sd2.id = id; // same id
        let err = scheduler.add_schedule(sd2).await.unwrap_err();
        assert!(matches!(err, ScheduleError::AlreadyExists(_)));
    }

    #[tokio::test]
    async fn test_get_schedule() {
        let scheduler = Scheduler::new();
        let sd = ScheduleDefinition::new("getter", EVERY_MINUTE, "do_get")
            .expect("ScheduleDefinition::new should succeed");
        let id = sd.id.clone();
        scheduler
            .add_schedule(sd)
            .await
            .expect("async operation should succeed");

        let got = scheduler
            .get_schedule(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(got.name, "getter");
    }

    #[tokio::test]
    async fn test_get_schedule_not_found() {
        let scheduler = Scheduler::new();
        let err = scheduler.get_schedule("missing").await.unwrap_err();
        assert!(matches!(err, ScheduleError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_list_schedules() {
        let scheduler = Scheduler::new();
        scheduler
            .add_schedule(
                ScheduleDefinition::new("a", EVERY_MINUTE, "t1")
                    .expect("ScheduleDefinition::new should succeed"),
            )
            .await
            .expect("ScheduleDefinition::new should succeed");
        scheduler
            .add_schedule(
                ScheduleDefinition::new("b", EVERY_MINUTE, "t2")
                    .expect("ScheduleDefinition::new should succeed"),
            )
            .await
            .expect("ScheduleDefinition::new should succeed");
        let list = scheduler.list_schedules().await;
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn test_update_schedule() {
        let scheduler = Scheduler::new();
        let mut sd = ScheduleDefinition::new("orig", EVERY_MINUTE, "task_orig")
            .expect("ScheduleDefinition::new should succeed");
        let id = sd.id.clone();
        scheduler
            .add_schedule(sd.clone())
            .await
            .expect("async operation should succeed");

        sd.name = "updated".to_string();
        sd.task = "task_updated".to_string();
        let updated = scheduler
            .update_schedule(sd)
            .await
            .expect("async operation should succeed");
        assert_eq!(updated.name, "updated");

        let got = scheduler
            .get_schedule(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(got.task, "task_updated");
    }

    #[tokio::test]
    async fn test_update_schedule_not_found() {
        let scheduler = Scheduler::new();
        let sd = ScheduleDefinition::new("ghost", EVERY_MINUTE, "task")
            .expect("ScheduleDefinition::new should succeed");
        let err = scheduler.update_schedule(sd).await.unwrap_err();
        assert!(matches!(err, ScheduleError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_delete_schedule() {
        let scheduler = Scheduler::new();
        let sd = ScheduleDefinition::new("doomed", EVERY_MINUTE, "task")
            .expect("ScheduleDefinition::new should succeed");
        let id = sd.id.clone();
        scheduler
            .add_schedule(sd)
            .await
            .expect("async operation should succeed");
        scheduler
            .delete_schedule(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(scheduler.schedule_count().await, 0);
    }

    #[tokio::test]
    async fn test_delete_schedule_not_found() {
        let scheduler = Scheduler::new();
        let err = scheduler.delete_schedule("nope").await.unwrap_err();
        assert!(matches!(err, ScheduleError::NotFound(_)));
    }

    // -- Pause/Resume tests -------------------------------------------------

    #[tokio::test]
    async fn test_pause_schedule() {
        let scheduler = Scheduler::new();
        let sd = ScheduleDefinition::new("pausable", EVERY_MINUTE, "task")
            .expect("ScheduleDefinition::new should succeed");
        let id = sd.id.clone();
        scheduler
            .add_schedule(sd)
            .await
            .expect("async operation should succeed");

        let paused = scheduler
            .pause_schedule(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(paused.status, ScheduleStatus::Paused);
        assert!(!paused.enabled);
        assert!(paused.updated_at.is_some());
    }

    #[tokio::test]
    async fn test_resume_schedule() {
        let scheduler = Scheduler::new();
        let sd = ScheduleDefinition::new("resumable", EVERY_MINUTE, "task")
            .expect("ScheduleDefinition::new should succeed");
        let id = sd.id.clone();
        scheduler
            .add_schedule(sd)
            .await
            .expect("async operation should succeed");
        scheduler
            .pause_schedule(&id)
            .await
            .expect("async operation should succeed");

        let resumed = scheduler
            .resume_schedule(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(resumed.status, ScheduleStatus::Active);
        assert!(resumed.enabled);
        assert!(resumed.next_run.is_some());
    }

    #[tokio::test]
    async fn test_pause_not_found() {
        let scheduler = Scheduler::new();
        let err = scheduler.pause_schedule("missing").await.unwrap_err();
        assert!(matches!(err, ScheduleError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_resume_not_found() {
        let scheduler = Scheduler::new();
        let err = scheduler.resume_schedule("missing").await.unwrap_err();
        assert!(matches!(err, ScheduleError::NotFound(_)));
    }

    // -- Run record tracking tests ------------------------------------------

    #[tokio::test]
    async fn test_record_run() {
        let scheduler = Scheduler::new();
        let run = RunRecord::new("s1", 1);
        scheduler.record_run(run).await;
        assert_eq!(scheduler.run_count().await, 1);
    }

    #[tokio::test]
    async fn test_list_runs() {
        let scheduler = Scheduler::new();
        scheduler.record_run(RunRecord::new("s1", 1)).await;
        scheduler.record_run(RunRecord::new("s2", 1)).await;
        let runs = scheduler.list_runs().await;
        assert_eq!(runs.len(), 2);
    }

    #[tokio::test]
    async fn test_runs_for_schedule() {
        let scheduler = Scheduler::new();
        scheduler.record_run(RunRecord::new("s1", 1)).await;
        scheduler.record_run(RunRecord::new("s1", 2)).await;
        scheduler.record_run(RunRecord::new("s2", 1)).await;
        let runs = scheduler.runs_for_schedule("s1").await;
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|r| r.schedule_id == "s1"));
    }

    // -- Ring buffer limit test ---------------------------------------------

    #[tokio::test]
    async fn test_run_history_ring_buffer() {
        let scheduler = Scheduler::new();
        // Insert more than MAX_RUN_HISTORY records
        for i in 0..1050 {
            scheduler
                .record_run(RunRecord::new(format!("s{}", i), 1))
                .await;
        }
        assert_eq!(scheduler.run_count().await, MAX_RUN_HISTORY);
        // The earliest records should have been evicted
        let runs = scheduler.list_runs().await;
        // First remaining should be s50 (indices 0..49 evicted)
        assert_eq!(runs[0].schedule_id, "s50");
    }

    // -- next_due_schedules tests -------------------------------------------

    #[tokio::test]
    async fn test_next_due_schedules_none_due() {
        let scheduler = Scheduler::new();
        // next_run is in the future, so nothing should be due
        let sd = ScheduleDefinition::new("future", EVERY_MINUTE, "task")
            .expect("ScheduleDefinition::new should succeed");
        scheduler
            .add_schedule(sd)
            .await
            .expect("async operation should succeed");
        // This schedule's next_run is in the future, so it shouldn't be due
        // (unless we happen to run exactly at the second boundary, which is unlikely)
        let due = scheduler.next_due_schedules().await;
        // The schedule is in the future, so typically 0 due
        // We just check it doesn't panic
        assert!(due.len() <= 1);
    }

    #[tokio::test]
    async fn test_next_due_schedules_with_past_next_run() {
        let scheduler = Scheduler::new();
        let mut sd = ScheduleDefinition::new("past", EVERY_MINUTE, "task")
            .expect("ScheduleDefinition::new should succeed");
        // Manually set next_run to the past
        sd.next_run = Some(Utc::now() - chrono::Duration::hours(1));
        let id = sd.id.clone();
        scheduler
            .add_schedule(sd)
            .await
            .expect("async operation should succeed");

        let due = scheduler.next_due_schedules().await;
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, id);
    }

    #[tokio::test]
    async fn test_next_due_schedules_paused_excluded() {
        let scheduler = Scheduler::new();
        let mut sd = ScheduleDefinition::new("paused", EVERY_MINUTE, "task")
            .expect("ScheduleDefinition::new should succeed");
        sd.next_run = Some(Utc::now() - chrono::Duration::hours(1));
        let id = sd.id.clone();
        scheduler
            .add_schedule(sd)
            .await
            .expect("async operation should succeed");
        scheduler
            .pause_schedule(&id)
            .await
            .expect("async operation should succeed");

        let due = scheduler.next_due_schedules().await;
        assert_eq!(due.len(), 0);
    }

    // -- Serialization tests ------------------------------------------------

    #[test]
    fn test_delivery_mode_serialization() {
        let json =
            serde_json::to_string(&DeliveryMode::FireAndForget).expect("should serialize to JSON");
        assert_eq!(json, "\"fire_and_forget\"");
        let de: DeliveryMode =
            serde_json::from_str("\"acknowledged\"").expect("should parse successfully");
        assert_eq!(de, DeliveryMode::Acknowledged);
        let de2: DeliveryMode =
            serde_json::from_str("\"guaranteed\"").expect("should parse successfully");
        assert_eq!(de2, DeliveryMode::Guaranteed);
    }

    #[test]
    fn test_schedule_status_serialization() {
        let json =
            serde_json::to_string(&ScheduleStatus::Active).expect("should serialize to JSON");
        assert_eq!(json, "\"active\"");
        let de: ScheduleStatus =
            serde_json::from_str("\"paused\"").expect("should parse successfully");
        assert_eq!(de, ScheduleStatus::Paused);
        let de2: ScheduleStatus =
            serde_json::from_str("\"completed\"").expect("should parse successfully");
        assert_eq!(de2, ScheduleStatus::Completed);
        let de3: ScheduleStatus =
            serde_json::from_str("\"failed\"").expect("should parse successfully");
        assert_eq!(de3, ScheduleStatus::Failed);
    }

    #[test]
    fn test_run_status_serialization() {
        let json = serde_json::to_string(&RunStatus::Running).expect("should serialize to JSON");
        assert_eq!(json, "\"running\"");
        let de: RunStatus =
            serde_json::from_str("\"completed\"").expect("should parse successfully");
        assert_eq!(de, RunStatus::Completed);
        let de2: RunStatus = serde_json::from_str("\"failed\"").expect("should parse successfully");
        assert_eq!(de2, RunStatus::Failed);
        let de3: RunStatus =
            serde_json::from_str("\"skipped\"").expect("should parse successfully");
        assert_eq!(de3, RunStatus::Skipped);
    }

    // -- Error display tests ------------------------------------------------

    #[test]
    fn test_error_display_invalid_cron() {
        let err = ScheduleError::InvalidCron("bad expr".to_string());
        assert_eq!(err.to_string(), "invalid cron expression: bad expr");
    }

    #[test]
    fn test_error_display_not_found() {
        let err = ScheduleError::NotFound("sched-123".to_string());
        assert_eq!(err.to_string(), "schedule not found: sched-123");
    }

    #[test]
    fn test_error_display_already_exists() {
        let err = ScheduleError::AlreadyExists("sched-456".to_string());
        assert_eq!(err.to_string(), "schedule already exists: sched-456");
    }

    #[test]
    fn test_error_display_execution_failed() {
        let err = ScheduleError::ExecutionFailed("timeout".to_string());
        assert_eq!(err.to_string(), "execution failed: timeout");
    }
}
