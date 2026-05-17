//! Cron-based task scheduler
//!
//! Replaces the simple HEARTBEAT.md frequency system with real cron expressions.
//! Tasks can be defined in configuration or added at runtime.

use chrono::{DateTime, Utc};
use cron::Schedule;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::{RwLock, Semaphore, watch, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use zeus_core::{Message, Result};
use zeus_llm::LlmClient;
use zeus_memory::Workspace;

// ============================================================================
// Types
// ============================================================================

/// The type of task to execute when a schedule fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskType {
    /// Run workspace heartbeat tasks for a given frequency.
    Heartbeat { frequency: String },
    /// Send a prompt to the LLM with workspace context.
    LlmPrompt { prompt: String },
    /// Run a shell command via `sh -c`.
    Shell { command: String },
    /// Add a note to today's daily note.
    WorkspaceNote { content: String },
    /// Drain the content batch queue: process all ready jobs through the content pipeline.
    ///
    /// Polls `ContentQueue::get_ready_jobs()` and runs each through the FFmpeg +
    /// upload pipeline, updating job status to `Published` or `Failed`.
    /// Schedule this every 1–5 minutes to process jobs enqueued via the API or
    /// `ContentQueue::enqueue()`.
    ContentQueueDrain {
        /// Path to the SQLite database file used by `ContentQueue`.
        db_path: String,
    },
    /// Run a content production pipeline: trim → platform resize → captions → upload.
    ///
    /// Chains FFmpeg editing tools then uploads to YouTube, TikTok, or Instagram.
    /// Requires `ffmpeg` in PATH and the appropriate platform token env vars.
    ContentPipeline {
        /// Local source video file path (used for ffmpeg stages + youtube/tiktok upload).
        input_path: String,
        /// Upload target: `"youtube"` · `"tiktok"` · `"instagram"`.
        platform: String,
        /// Video title for upload metadata.
        title: String,
        /// Video description for upload metadata.
        description: String,
        /// Optional trim start time (HH:MM:SS or seconds).
        #[serde(skip_serializing_if = "Option::is_none")]
        trim_start: Option<String>,
        /// Optional trim end time (HH:MM:SS or seconds).
        #[serde(skip_serializing_if = "Option::is_none")]
        trim_end: Option<String>,
        /// Optional path to `.srt` subtitle file to burn in (non-fatal if missing).
        #[serde(skip_serializing_if = "Option::is_none")]
        captions_srt: Option<String>,
        /// Public video URL for Instagram uploads (Graph API requires URL, not file path).
        #[serde(skip_serializing_if = "Option::is_none")]
        media_url: Option<String>,
    },
}

/// Configuration for a single scheduled task (serializable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    /// Human-readable name for this task.
    pub name: String,
    /// Cron expression (standard 5-field or 7-field).
    /// 5-field expressions like "0 9 * * *" are automatically converted to
    /// 7-field by prepending "0 " (seconds) and appending " *" (year).
    pub cron: String,
    /// What to do when the schedule fires.
    pub task_type: TaskType,
    /// Whether this task is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Top-level scheduler configuration (serializable).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SchedulerConfig {
    /// Whether the scheduler is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Maximum number of cron jobs that can run concurrently.
    /// Prevents runaway resource consumption when many tasks fire at once.
    /// Default: 4. Set to 0 for unlimited (not recommended).
    #[serde(default = "default_max_concurrent_jobs")]
    pub max_concurrent_jobs: u32,
    /// Scheduled tasks.
    #[serde(default)]
    pub tasks: Vec<TaskConfig>,
}

fn default_max_concurrent_jobs() -> u32 {
    4
}

impl SchedulerConfig {
    /// Create a scheduler config with sensible default tasks.
    pub fn with_defaults() -> Self {
        Self {
            enabled: true,
            max_concurrent_jobs: default_max_concurrent_jobs(),
            tasks: vec![
                TaskConfig {
                    name: "Daily review".to_string(),
                    cron: "0 9 * * *".to_string(),
                    task_type: TaskType::Heartbeat {
                        frequency: "daily".to_string(),
                    },
                    enabled: true,
                },
                TaskConfig {
                    name: "Weekly review".to_string(),
                    cron: "0 9 * * 1".to_string(),
                    task_type: TaskType::Heartbeat {
                        frequency: "weekly".to_string(),
                    },
                    enabled: true,
                },
                TaskConfig {
                    name: "Hourly check".to_string(),
                    cron: "0 * * * *".to_string(),
                    task_type: TaskType::Heartbeat {
                        frequency: "hourly".to_string(),
                    },
                    enabled: true,
                },
                TaskConfig {
                    name: "Daily memory consolidation".to_string(),
                    cron: "0 2 * * *".to_string(),
                    task_type: TaskType::LlmPrompt {
                        prompt: "Consolidate recent memories: review today's interactions, extract key facts and decisions, prune stale or redundant entries. Respond with a brief summary of what was consolidated.".to_string(),
                    },
                    enabled: true,
                },
                TaskConfig {
                    name: "Weekly session cleanup".to_string(),
                    cron: "0 3 * * 0".to_string(),
                    task_type: TaskType::LlmPrompt {
                        prompt: "Review old sessions from the past week. Summarize key outcomes and clean up any stale session state. Respond with a brief cleanup report.".to_string(),
                    },
                    enabled: true,
                },
            ],
        }
    }
}

/// A live scheduled task with runtime state.
#[derive(Debug, Clone)]
pub struct ScheduledTask {
    /// Unique task identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Original cron expression (as provided by the user).
    pub cron_expr: String,
    /// What to do when the schedule fires.
    pub task_type: TaskType,
    /// Whether this task is enabled.
    pub enabled: bool,
    /// When this task last ran.
    pub last_run: Option<DateTime<Utc>>,
    /// Next scheduled execution time.
    pub next_run: Option<DateTime<Utc>>,
}

/// Record of a single task execution.
#[derive(Debug, Clone)]
pub struct TaskExecution {
    /// ID of the task that ran.
    pub task_id: String,
    /// Name of the task that ran.
    pub task_name: String,
    /// When execution started.
    pub started_at: DateTime<Utc>,
    /// When execution completed (None if still running).
    pub completed_at: Option<DateTime<Utc>>,
    /// Whether the execution succeeded.
    pub success: bool,
    /// Output or error message.
    pub output: String,
}

// ============================================================================
// CronScheduler
// ============================================================================

/// Cron-based task scheduler.
///
/// Manages a list of [`ScheduledTask`]s, each with a cron expression that
/// determines when it fires. The scheduler can run as a background loop
/// or be ticked manually (useful for testing).
pub struct CronScheduler {
    tasks: Arc<RwLock<Vec<ScheduledTask>>>,
    history: Arc<RwLock<Vec<TaskExecution>>>,
    shutdown_tx: Option<watch::Sender<bool>>,
    db: Option<SchedulerDb>,
    /// Semaphore limiting concurrent job execution.
    concurrency_limit: Arc<Semaphore>,
    /// Number of jobs currently executing (for observability).
    active_jobs: Arc<AtomicU32>,
    /// Configured max concurrent jobs.
    max_concurrent_jobs: u32,
    /// Cancellation tokens for currently running tasks (task_id → token).
    /// Used to abort tasks mid-execution via the API.
    running_tokens: Arc<RwLock<HashMap<String, CancellationToken>>>,
    /// Channel to send trigger execution results back to the gateway
    /// so they can be injected into the agent's context as a system message.
    trigger_result_tx: Option<mpsc::UnboundedSender<String>>,
}

impl CronScheduler {
    /// Create a new scheduler from configuration.
    ///
    /// Parses all cron expressions and computes initial `next_run` times.
    /// Invalid cron expressions are logged as warnings and the corresponding
    /// tasks are added in a disabled state.
    pub fn new(config: SchedulerConfig) -> Self {
        let mut tasks = Vec::with_capacity(config.tasks.len());

        for tc in &config.tasks {
            let id = uuid::Uuid::new_v4().to_string();

            // Parse human-readable schedules into cron expressions.
            let cron_expr = match parse_human_schedule(&tc.cron) {
                Ok(expr) => expr,
                Err(_) => tc.cron.clone(),
            };

            let next_run = if tc.enabled {
                compute_next_run(&cron_expr)
            } else {
                None
            };

            if tc.enabled && next_run.is_none() {
                warn!(
                    "Task '{}' has an invalid cron expression '{}'; disabling it",
                    tc.name, tc.cron
                );
            }

            tasks.push(ScheduledTask {
                id,
                name: tc.name.clone(),
                cron_expr,
                task_type: tc.task_type.clone(),
                enabled: tc.enabled && next_run.is_some(),
                last_run: None,
                next_run,
            });
        }

        let max_concurrent = if config.max_concurrent_jobs == 0 {
            1024 // "unlimited" — use a very large semaphore
        } else {
            config.max_concurrent_jobs as usize
        };

        Self {
            tasks: Arc::new(RwLock::new(tasks)),
            history: Arc::new(RwLock::new(Vec::new())),
            shutdown_tx: None,
            db: None,
            concurrency_limit: Arc::new(Semaphore::new(max_concurrent)),
            active_jobs: Arc::new(AtomicU32::new(0)),
            max_concurrent_jobs: config.max_concurrent_jobs,
            running_tokens: Arc::new(RwLock::new(HashMap::new())),
            trigger_result_tx: None,
        }
    }

    /// Set the channel for delivering trigger execution results to the gateway.
    ///
    /// When set, every trigger execution result is sent through this channel.
    /// The gateway listens on the receiving end and injects the output as a
    /// system message into the agent's next turn.
    pub fn set_trigger_result_tx(&mut self, tx: mpsc::UnboundedSender<String>) {
        self.trigger_result_tx = Some(tx);
    }

    /// Attach a SQLite database for task persistence.
    ///
    /// Initializes the database schema, loads any previously persisted tasks
    /// (appending them to tasks already loaded from config), and saves all
    /// current tasks to the database.
    pub fn with_db(mut self, db_path: impl Into<PathBuf>) -> Result<Self> {
        let db = SchedulerDb::new(db_path);
        db.init()?;

        // Load previously persisted tasks.
        let persisted = db.load_tasks()?;

        // We have exclusive ownership of `self` so the Arc refcount is 1.
        let tasks = Arc::get_mut(&mut self.tasks)
            .expect("with_db must be called before the scheduler is shared")
            .get_mut();

        for task in persisted {
            // Avoid duplicates by id.
            if !tasks.iter().any(|t| t.id == task.id) {
                tasks.push(task);
            }
        }

        // Save all current tasks to db.
        for task in tasks.iter() {
            db.save_task(task)?;
        }

        self.db = Some(db);
        Ok(self)
    }

    /// Add a task at runtime. Returns the generated task ID.
    ///
    /// The `cron` field in the config can be either a raw cron expression or
    /// a human-readable string (e.g. "every 5 minutes", "daily at 3pm").
    pub async fn add_task(&self, config: TaskConfig) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();

        // Parse human-readable schedules into cron expressions.
        let cron_expr = parse_human_schedule(&config.cron)?;

        let next_run = if config.enabled {
            compute_next_run(&cron_expr)
        } else {
            None
        };

        if config.enabled && next_run.is_none() {
            return Err(zeus_core::Error::Config(format!(
                "Invalid cron expression: '{}'",
                config.cron
            )));
        }

        let task = ScheduledTask {
            id: id.clone(),
            name: config.name,
            cron_expr,
            task_type: config.task_type,
            enabled: config.enabled,
            last_run: None,
            next_run,
        };

        if let Some(ref db) = self.db {
            db.save_task(&task)?;
        }

        self.tasks.write().await.push(task);
        Ok(id)
    }

    /// Remove a task by ID.
    pub async fn remove_task(&self, id: &str) -> Result<()> {
        let mut tasks = self.tasks.write().await;
        let before = tasks.len();
        tasks.retain(|t| t.id != id);
        if tasks.len() == before {
            return Err(zeus_core::Error::NotFound(format!(
                "No task with id '{}'",
                id
            )));
        }

        if let Some(ref db) = self.db {
            db.delete_task(id)?;
        }

        Ok(())
    }

    /// List all tasks.
    pub async fn list_tasks(&self) -> Vec<ScheduledTask> {
        self.tasks.read().await.clone()
    }

    /// Get recent execution history, most recent first.
    pub async fn get_history(&self, limit: usize) -> Vec<TaskExecution> {
        let history = self.history.read().await;
        history.iter().rev().take(limit).cloned().collect()
    }

    /// Start the background scheduler loop.
    ///
    /// The loop wakes every 30 seconds, checks which tasks are due, and
    /// executes them. Requires `Arc` references to workspace and LLM client.
    pub async fn start(&mut self, workspace: Arc<Workspace>, llm: Arc<LlmClient>) -> Result<()> {
        if self.shutdown_tx.is_some() {
            return Ok(()); // Already running
        }

        let (tx, rx) = watch::channel(false);
        self.shutdown_tx = Some(tx);

        let tasks = self.tasks.clone();
        let history = self.history.clone();
        let concurrency_limit = self.concurrency_limit.clone();
        let active_jobs = self.active_jobs.clone();
        let running_tokens = self.running_tokens.clone();
        let trigger_result_tx = self.trigger_result_tx.clone();

        tokio::spawn(async move {
            scheduler_loop(
                tasks,
                history,
                workspace,
                llm,
                rx,
                concurrency_limit,
                active_jobs,
                running_tokens,
                trigger_result_tx,
            )
            .await;
        });

        info!("Cron scheduler started");
        Ok(())
    }

    /// Stop the background scheduler loop.
    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
            info!("Cron scheduler stopped");
        }
    }

    /// Returns `true` if the background loop is running.
    pub fn is_running(&self) -> bool {
        self.shutdown_tx.is_some()
    }

    /// Manually tick the scheduler: check all tasks and execute any that are due.
    ///
    /// This is the same logic used by the background loop and is exposed for
    /// testing and one-shot evaluation.
    pub async fn tick(&self, workspace: &Workspace, llm: &LlmClient) -> Result<Vec<TaskExecution>> {
        let now = Utc::now();
        let mut due_tasks: Vec<(usize, ScheduledTask)> = Vec::new();

        // Collect tasks that are due.
        {
            let tasks = self.tasks.read().await;
            for (idx, task) in tasks.iter().enumerate() {
                if !task.enabled {
                    continue;
                }
                if let Some(next) = task.next_run
                    && now >= next
                {
                    due_tasks.push((idx, task.clone()));
                }
            }
        }

        let mut executions = Vec::new();

        for (idx, task) in &due_tasks {
            // Acquire a semaphore permit before executing — this gates
            // concurrency for tick() the same way the background loop is gated.
            let _permit = self.concurrency_limit.acquire().await.map_err(|_| {
                zeus_core::Error::Config("Concurrency semaphore closed".to_string())
            })?;
            self.active_jobs.fetch_add(1, Ordering::Relaxed);

            let execution = execute_task(workspace, llm, task).await;

            self.active_jobs.fetch_sub(1, Ordering::Relaxed);
            // _permit dropped here, releasing the semaphore slot.

            // Update the task's timing.
            let next_run;
            {
                let mut tasks = self.tasks.write().await;
                if let Some(t) = tasks.get_mut(*idx) {
                    t.last_run = Some(execution.started_at);
                    t.next_run = compute_next_run(&t.cron_expr);
                    next_run = t.next_run;
                    debug!("Task '{}' executed; next run: {:?}", t.name, t.next_run);
                } else {
                    next_run = None;
                }
            }

            // Persist execution state to SQLite if available.
            if let Some(ref db) = self.db {
                let status = if execution.success { "ok" } else { "error" };
                let _ = db.update_execution(
                    &task.id,
                    execution.started_at,
                    next_run,
                    status,
                    &execution.output,
                );
            }

            // Record in history.
            self.history.write().await.push(execution.clone());

            // Send trigger result to gateway for agent context injection.
            if let Some(ref tx) = self.trigger_result_tx {
                let status = if execution.success { "✅" } else { "❌" };
                let msg = format!(
                    "[Trigger: {}] {}\n{}",
                    task.name, status, execution.output
                );
                let _ = tx.send(msg);
            }

            executions.push(execution);
        }

        Ok(executions)
    }
}

// ============================================================================
// SchedulerDb — SQLite persistence
// ============================================================================

const SCHEDULER_MIGRATIONS: &[&str] = &[
    // v1: initial schema
    "CREATE TABLE IF NOT EXISTS scheduled_tasks (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                schedule TEXT NOT NULL,
                schedule_type TEXT NOT NULL,
                task_payload TEXT NOT NULL,
                enabled INTEGER DEFAULT 1,
                last_run INTEGER,
                next_run INTEGER NOT NULL,
                last_status TEXT,
                last_output TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );",
];

/// SQLite-backed persistence for scheduled tasks.
pub struct SchedulerDb {
    path: PathBuf,
}

impl SchedulerDb {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Initialize the database schema.
    pub fn init(&self) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.path)
            .map_err(|e| zeus_core::Error::Config(format!("Failed to open scheduler db: {}", e)))?;
        crate::db::run_migrations(&conn, SCHEDULER_MIGRATIONS)?;
        Ok(())
    }

    /// Save a task to the database.
    pub fn save_task(&self, task: &ScheduledTask) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.path)
            .map_err(|e| zeus_core::Error::Config(format!("Failed to open scheduler db: {}", e)))?;

        let payload = serde_json::to_string(&task.task_type).map_err(|e| {
            zeus_core::Error::Config(format!("Failed to serialize task type: {}", e))
        })?;

        let now = Utc::now().timestamp();
        let next_run = task.next_run.map(|t| t.timestamp()).unwrap_or(now);
        let last_run = task.last_run.map(|t| t.timestamp());

        conn.execute(
            "INSERT OR REPLACE INTO scheduled_tasks
                (id, name, schedule, schedule_type, task_payload, enabled, last_run, next_run, last_status, last_output, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                task.id,
                task.name,
                task.cron_expr,
                "cron",
                payload,
                task.enabled as i32,
                last_run,
                next_run,
                Option::<String>::None,
                Option::<String>::None,
                now,
                now,
            ],
        )
        .map_err(|e| zeus_core::Error::Config(format!("Failed to save task: {}", e)))?;

        Ok(())
    }

    /// Load all tasks from the database.
    pub fn load_tasks(&self) -> Result<Vec<ScheduledTask>> {
        let conn = rusqlite::Connection::open(&self.path)
            .map_err(|e| zeus_core::Error::Config(format!("Failed to open scheduler db: {}", e)))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, name, schedule, task_payload, enabled, last_run, next_run FROM scheduled_tasks"
            )
            .map_err(|e| zeus_core::Error::Config(format!("Failed to prepare query: {}", e)))?;

        let rows = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let name: String = row.get(1)?;
                let schedule: String = row.get(2)?;
                let payload: String = row.get(3)?;
                let enabled: i32 = row.get(4)?;
                let last_run: Option<i64> = row.get(5)?;
                let next_run: Option<i64> = row.get(6)?;
                Ok((id, name, schedule, payload, enabled, last_run, next_run))
            })
            .map_err(|e| zeus_core::Error::Config(format!("Failed to query tasks: {}", e)))?;

        let mut tasks = Vec::new();
        for row in rows {
            let (id, name, schedule, payload, enabled, last_run, next_run) =
                row.map_err(|e| zeus_core::Error::Config(format!("Failed to read row: {}", e)))?;

            let task_type: TaskType = serde_json::from_str(&payload).map_err(|e| {
                zeus_core::Error::Config(format!("Failed to deserialize task payload: {}", e))
            })?;

            let last_run_dt = last_run.and_then(|ts| DateTime::from_timestamp(ts, 0));
            let next_run_dt = next_run.and_then(|ts| DateTime::from_timestamp(ts, 0));

            tasks.push(ScheduledTask {
                id,
                name,
                cron_expr: schedule,
                task_type,
                enabled: enabled != 0,
                last_run: last_run_dt,
                next_run: next_run_dt,
            });
        }

        Ok(tasks)
    }

    /// Delete a task by ID.
    pub fn delete_task(&self, id: &str) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.path)
            .map_err(|e| zeus_core::Error::Config(format!("Failed to open scheduler db: {}", e)))?;

        conn.execute(
            "DELETE FROM scheduled_tasks WHERE id = ?1",
            rusqlite::params![id],
        )
        .map_err(|e| zeus_core::Error::Config(format!("Failed to delete task: {}", e)))?;

        Ok(())
    }

    /// Update task execution state.
    pub fn update_execution(
        &self,
        task_id: &str,
        last_run: DateTime<Utc>,
        next_run: Option<DateTime<Utc>>,
        status: &str,
        output: &str,
    ) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.path)
            .map_err(|e| zeus_core::Error::Config(format!("Failed to open scheduler db: {}", e)))?;

        let now = Utc::now().timestamp();
        let last_run_ts = last_run.timestamp();
        let next_run_ts = next_run.map(|t| t.timestamp());

        conn.execute(
            "UPDATE scheduled_tasks SET last_run = ?1, next_run = ?2, last_status = ?3, last_output = ?4, updated_at = ?5 WHERE id = ?6",
            rusqlite::params![
                last_run_ts,
                next_run_ts,
                status,
                output,
                now,
                task_id,
            ],
        )
        .map_err(|e| zeus_core::Error::Config(format!("Failed to update execution: {}", e)))?;

        Ok(())
    }
}

// ============================================================================
// Background loop
// ============================================================================

#[allow(clippy::too_many_arguments)]
async fn scheduler_loop(
    tasks: Arc<RwLock<Vec<ScheduledTask>>>,
    history: Arc<RwLock<Vec<TaskExecution>>>,
    workspace: Arc<Workspace>,
    llm: Arc<LlmClient>,
    mut shutdown: watch::Receiver<bool>,
    concurrency_limit: Arc<Semaphore>,
    active_jobs: Arc<AtomicU32>,
    running_tokens: Arc<RwLock<HashMap<String, CancellationToken>>>,
    trigger_result_tx: Option<mpsc::UnboundedSender<String>>,
) {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                debug!("Scheduler tick (active jobs: {})", active_jobs.load(Ordering::Relaxed));
                let now = Utc::now();
                let mut due: Vec<(usize, ScheduledTask)> = Vec::new();

                {
                    let tasks_r = tasks.read().await;
                    for (idx, task) in tasks_r.iter().enumerate() {
                        if !task.enabled {
                            continue;
                        }
                        if let Some(next) = task.next_run
                            && now >= next {
                                due.push((idx, task.clone()));
                            }
                    }
                }

                // Execute due tasks concurrently, gated by the semaphore.
                let mut handles: Vec<JoinHandle<()>> = Vec::new();
                for (idx, task) in due {
                    let permit = match concurrency_limit.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => {
                            warn!("Concurrency semaphore closed; stopping scheduler");
                            return;
                        }
                    };
                    let ws = workspace.clone();
                    let llm_c = llm.clone();
                    let active = active_jobs.clone();
                    let tasks_c = tasks.clone();
                    let history_c = history.clone();
                    let tokens = running_tokens.clone();
                    let cancel = CancellationToken::new();
                    let task_id = task.id.clone();
                    let task_name = task.name.clone();
                    let result_tx = trigger_result_tx.clone();

                    // Register the cancellation token so abort_task() can find it.
                    tokens.write().await.insert(task_id.clone(), cancel.clone());

                    active.fetch_add(1, Ordering::Relaxed);

                    handles.push(tokio::spawn(async move {
                        let execution = execute_task_cancellable(&ws, &llm_c, &task, &cancel).await;

                        // Unregister the cancellation token.
                        tokens.write().await.remove(&task_id);

                        // Update the task's timing.
                        {
                            let mut tasks_w = tasks_c.write().await;
                            if let Some(t) = tasks_w.get_mut(idx) {
                                t.last_run = Some(execution.started_at);
                                t.next_run = compute_next_run(&t.cron_expr);
                            }
                        }

                        // Send trigger result to gateway for agent context injection.
                        if let Some(ref tx) = result_tx {
                            let status = if execution.success { "✅" } else { "❌" };
                            let msg = format!(
                                "[Trigger: {}] {}\n{}",
                                task_name, status, execution.output
                            );
                            let _ = tx.send(msg);
                        }

                        history_c.write().await.push(execution);

                        active.fetch_sub(1, Ordering::Relaxed);
                        drop(permit); // release semaphore permit
                    }));
                }

                // Wait for all spawned tasks to complete before next tick.
                for handle in handles {
                    let _ = handle.await;
                }
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    info!("Scheduler loop shutting down (active jobs: {})", active_jobs.load(Ordering::Relaxed));
                    // Cancel all running tasks on shutdown.
                    let tokens = running_tokens.read().await;
                    for (id, token) in tokens.iter() {
                        info!("Cancelling task '{}' on shutdown", id);
                        token.cancel();
                    }
                    return;
                }
            }
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Normalize a cron expression to 7-field format expected by the `cron` crate.
///
/// The `cron` crate uses: `sec min hour dom month dow year`.
/// Standard (5-field) cron: `min hour dom month dow` -- we prepend `0` for
/// seconds and append `*` for year.
/// 6-field: `sec min hour dom month dow` -- we append `*` for year.
/// 7-field: passed through as-is.
fn normalize_cron_expr(expr: &str) -> String {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    match fields.len() {
        5 => format!("0 {} *", expr),
        6 => format!("{} *", expr),
        7 => expr.to_string(),
        _ => expr.to_string(), // let the parser report an error
    }
}

/// Parse a human-readable schedule string into a cron expression.
///
/// Supports patterns like:
/// - "every 5 minutes" -> "*/5 * * * *"
/// - "every hour" / "hourly" -> "0 * * * *"
/// - "every 2 hours" -> "0 */2 * * *"
/// - "daily" / "every day" -> "0 9 * * *"
/// - "daily at 3pm" / "daily at 15:00" -> "0 15 * * *"
/// - "weekly" / "every week" -> "0 9 * * 1"
/// - "weekly on friday" -> "0 9 * * 5"
/// - "monthly" -> "0 9 1 * *"
/// - Raw cron expressions are passed through unchanged.
pub fn parse_human_schedule(input: &str) -> Result<String> {
    let input = input.trim().to_lowercase();

    // "every N minutes"
    if let Some(rest) = input.strip_prefix("every ") {
        if let Some(min_str) = rest.strip_suffix(" minutes")
            && let Ok(n) = min_str.trim().parse::<u32>()
        {
            return Ok(format!("*/{} * * * *", n));
        }
        if let Some(min_str) = rest.strip_suffix(" minute")
            && let Ok(n) = min_str.trim().parse::<u32>()
        {
            return Ok(format!("*/{} * * * *", n));
        }

        // "every N hours"
        if let Some(hr_str) = rest.strip_suffix(" hours")
            && let Ok(n) = hr_str.trim().parse::<u32>()
        {
            return Ok(format!("0 */{} * * *", n));
        }
        if let Some(hr_str) = rest.strip_suffix(" hour") {
            if hr_str.trim().is_empty() {
                // "every hour"
                return Ok("0 * * * *".to_string());
            }
            if let Ok(n) = hr_str.trim().parse::<u32>() {
                return Ok(format!("0 */{} * * *", n));
            }
        }

        // "every day"
        if rest == "day" {
            return Ok("0 9 * * *".to_string());
        }

        // "every week"
        if rest == "week" {
            return Ok("0 9 * * 1".to_string());
        }
    }

    // "hourly"
    if input == "hourly" {
        return Ok("0 * * * *".to_string());
    }

    // "daily at Xam/pm" or "daily at HH:MM" or just "daily"
    if input.starts_with("daily") {
        if let Some(rest) = input.strip_prefix("daily at ") {
            let rest = rest.trim();
            // Try HH:MM format
            if let Some((hh, mm)) = rest.split_once(':')
                && let (Ok(hour), Ok(minute)) = (hh.trim().parse::<u32>(), mm.trim().parse::<u32>())
            {
                return Ok(format!("{} {} * * *", minute, hour));
            }
            // Try Xam/pm format
            let rest_clean = rest.replace(' ', "");
            if let Some(hr_str) = rest_clean.strip_suffix("pm")
                && let Ok(h) = hr_str.parse::<u32>()
            {
                let hour = if h == 12 { 12 } else { h + 12 };
                return Ok(format!("0 {} * * *", hour));
            }
            if let Some(hr_str) = rest_clean.strip_suffix("am")
                && let Ok(h) = hr_str.parse::<u32>()
            {
                let hour = if h == 12 { 0 } else { h };
                return Ok(format!("0 {} * * *", hour));
            }
            // Try plain hour number
            if let Ok(h) = rest.parse::<u32>() {
                return Ok(format!("0 {} * * *", h));
            }
        }
        if input == "daily" {
            return Ok("0 9 * * *".to_string());
        }
    }

    // "weekly on DAY" or just "weekly"
    if input.starts_with("weekly") {
        if let Some(rest) = input.strip_prefix("weekly on ") {
            let day = parse_day_name(rest.trim());
            if let Some(d) = day {
                return Ok(format!("0 9 * * {}", d));
            }
        }
        if input == "weekly" {
            return Ok("0 9 * * 1".to_string());
        }
    }

    // "monthly"
    if input == "monthly" {
        return Ok("0 9 1 * *".to_string());
    }

    // Not a recognized human pattern -- assume raw cron.
    // Validate that it parses as cron.
    let normalized = normalize_cron_expr(&input);
    if Schedule::from_str(&normalized).is_ok() {
        return Ok(input);
    }

    Err(zeus_core::Error::Config(format!(
        "Unrecognized schedule: '{}' (not a known human pattern or valid cron expression)",
        input
    )))
}

/// Map a day name to its Quartz cron text equivalent (SUN, MON, ..., SAT).
fn parse_day_name(name: &str) -> Option<&'static str> {
    // Return numeric day-of-week (0=Sunday .. 6=Saturday) for cron compatibility
    match name.to_lowercase().as_str() {
        "sunday" | "sun" => Some("0"),
        "monday" | "mon" => Some("1"),
        "tuesday" | "tue" | "tues" => Some("2"),
        "wednesday" | "wed" => Some("3"),
        "thursday" | "thu" | "thur" | "thurs" => Some("4"),
        "friday" | "fri" => Some("5"),
        "saturday" | "sat" => Some("6"),
        _ => None,
    }
}

/// Parse a cron expression and return the next upcoming fire time.
fn compute_next_run(expr: &str) -> Option<DateTime<Utc>> {
    let normalized = normalize_cron_expr(expr);
    match Schedule::from_str(&normalized) {
        Ok(schedule) => schedule.upcoming(Utc).next(),
        Err(e) => {
            warn!(
                "Failed to parse cron expression '{}' (normalized: '{}'): {}",
                expr, normalized, e
            );
            None
        }
    }
}

/// Execute a single task based on its [`TaskType`].
async fn execute_task(
    workspace: &Workspace,
    llm: &LlmClient,
    task: &ScheduledTask,
) -> TaskExecution {
    let started_at = Utc::now();
    let (success, output) = match &task.task_type {
        TaskType::Heartbeat { frequency } => execute_heartbeat(workspace, llm, frequency).await,
        TaskType::LlmPrompt { prompt } => execute_llm_prompt(workspace, llm, prompt).await,
        TaskType::Shell { command } => execute_shell(command).await,
        TaskType::WorkspaceNote { content } => execute_workspace_note(workspace, content).await,
        TaskType::ContentQueueDrain { db_path } => {
            crate::content_queue_drain::execute_content_queue_drain(db_path).await
        }
        TaskType::ContentPipeline {
            input_path,
            platform,
            title,
            description,
            trim_start,
            trim_end,
            captions_srt,
            media_url,
        } => {
            crate::content_pipeline::execute_content_pipeline(
                input_path, platform, title, description,
                trim_start, trim_end, captions_srt, media_url,
            )
            .await
        }
    };
    let completed_at = Some(Utc::now());

    TaskExecution {
        task_id: task.id.clone(),
        task_name: task.name.clone(),
        started_at,
        completed_at,
        success,
        output,
    }
}

/// Execute a task with cancellation support.
///
/// Races the normal task execution against the cancellation token. If cancelled,
/// produces an execution record with `success: false` and an "aborted" message.
async fn execute_task_cancellable(
    workspace: &Workspace,
    llm: &LlmClient,
    task: &ScheduledTask,
    cancel: &CancellationToken,
) -> TaskExecution {
    let started_at = Utc::now();

    tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            info!("Task '{}' ({}) was aborted", task.name, task.id);
            TaskExecution {
                task_id: task.id.clone(),
                task_name: task.name.clone(),
                started_at,
                completed_at: Some(Utc::now()),
                success: false,
                output: "Task aborted by user".to_string(),
            }
        }
        result = execute_task_inner(workspace, llm, task, cancel) => result,
    }
}

/// Inner task execution that is cancellation-aware for shell commands.
///
/// For shell tasks, spawns the child process and monitors the cancellation
/// token, killing the process if cancelled. For other task types, runs
/// normally (they will be interrupted at the next .await by the outer select).
async fn execute_task_inner(
    workspace: &Workspace,
    llm: &LlmClient,
    task: &ScheduledTask,
    cancel: &CancellationToken,
) -> TaskExecution {
    let started_at = Utc::now();
    let (success, output) = match &task.task_type {
        TaskType::Shell { command } => execute_shell_cancellable(command, cancel).await,
        TaskType::Heartbeat { frequency } => execute_heartbeat(workspace, llm, frequency).await,
        TaskType::LlmPrompt { prompt } => execute_llm_prompt(workspace, llm, prompt).await,
        TaskType::WorkspaceNote { content } => execute_workspace_note(workspace, content).await,
        TaskType::ContentQueueDrain { db_path } => {
            crate::content_queue_drain::execute_content_queue_drain(db_path).await
        }
        TaskType::ContentPipeline {
            input_path,
            platform,
            title,
            description,
            trim_start,
            trim_end,
            captions_srt,
            media_url,
        } => {
            crate::content_pipeline::execute_content_pipeline(
                input_path, platform, title, description,
                trim_start, trim_end, captions_srt, media_url,
            )
            .await
        }
    };
    let completed_at = Some(Utc::now());

    TaskExecution {
        task_id: task.id.clone(),
        task_name: task.name.clone(),
        started_at,
        completed_at,
        success,
        output,
    }
}

/// Execute heartbeat tasks for a given frequency.
async fn execute_heartbeat(
    workspace: &Workspace,
    llm: &LlmClient,
    frequency: &str,
) -> (bool, String) {
    let tasks = match workspace.get_heartbeat_tasks(frequency).await {
        Ok(t) => t,
        Err(e) => {
            return (false, format!("Failed to get heartbeat tasks: {}", e));
        }
    };

    if tasks.is_empty() {
        return (true, format!("No {} heartbeat tasks to run", frequency));
    }

    let mut outputs = Vec::new();
    let mut all_ok = true;

    for task_desc in &tasks {
        let context = workspace.get_context().await.unwrap_or_default();
        let system = format!(
            "{}\n\n\
             You are running a proactive heartbeat task. Complete it concisely and report results.\n\
             If you need to note something for the user, say so clearly.",
            context
        );
        let messages = vec![Message::user(format!(
            "Complete this heartbeat task: {}",
            task_desc
        ))];

        match llm.complete(&messages, &[], Some(&system)).await {
            Ok(response) => {
                outputs.push(format!("[OK] {}: {}", task_desc, response.content));
            }
            Err(e) => {
                all_ok = false;
                outputs.push(format!("[FAIL] {}: {}", task_desc, e));
            }
        }
    }

    let summary = outputs.join("\n");
    let note = format!(
        "[Scheduler] Ran {} {} tasks:\n{}",
        tasks.len(),
        frequency,
        summary
    );
    let _ = workspace.note(&note).await;

    (all_ok, summary)
}

/// Send a prompt to the LLM with workspace context.
async fn execute_llm_prompt(
    workspace: &Workspace,
    llm: &LlmClient,
    prompt: &str,
) -> (bool, String) {
    let context = workspace.get_context().await.unwrap_or_default();
    let messages = vec![Message::user(prompt)];

    match llm.complete(&messages, &[], Some(&context)).await {
        Ok(response) => (true, response.content),
        Err(e) => (false, format!("LLM error: {}", e)),
    }
}

/// Execute a shell command via `sh -c`.
///
/// Validates the command against basic safety checks before execution.
/// Cron commands come from the config/database, but defense-in-depth
/// still rejects null bytes, oversized commands, and obviously destructive patterns.
async fn execute_shell(command: &str) -> (bool, String) {
    // Defense-in-depth: reject null bytes and oversized commands
    if command.contains('\0') {
        return (false, "Command contains null bytes".to_string());
    }
    if command.len() > 10_000 {
        return (false, "Command too long (max 10,000 chars)".to_string());
    }
    // Block obviously destructive patterns even from config
    let blocked = [
        "rm -rf /",
        "mkfs.",
        "dd if=/dev/zero of=/dev/",
        ":(){ :|:& };:",
    ];
    for pattern in &blocked {
        if command.contains(pattern) {
            return (
                false,
                format!("Command blocked by safety check: contains '{}'", pattern),
            );
        }
    }

    match tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let combined = if stderr.is_empty() {
                stdout
            } else {
                format!("{}\n{}", stdout, stderr)
            };
            (output.status.success(), combined)
        }
        Err(e) => (false, format!("Failed to execute command: {}", e)),
    }
}

/// Execute a shell command with cancellation support.
///
/// Spawns the child process and monitors the cancellation token. If cancelled,
/// kills the child process and returns an "aborted" result.
async fn execute_shell_cancellable(command: &str, cancel: &CancellationToken) -> (bool, String) {
    // Run the same validation as execute_shell.
    if command.contains('\0') {
        return (false, "Command contains null bytes".to_string());
    }
    if command.len() > 10_000 {
        return (false, "Command too long (max 10,000 chars)".to_string());
    }
    let blocked = [
        "rm -rf /",
        "mkfs.",
        "dd if=/dev/zero of=/dev/",
        ":(){ :|:& };:",
    ];
    for pattern in &blocked {
        if command.contains(pattern) {
            return (
                false,
                format!("Command blocked by safety check: contains '{}'", pattern),
            );
        }
    }

    let mut child = match tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return (false, format!("Failed to spawn command: {}", e)),
    };

    // Take the pipe handles so wait() doesn't consume child.
    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();

    tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            // Kill the child process on cancellation.
            let _ = child.kill().await;
            (false, "Shell command aborted by user".to_string())
        }
        result = child.wait() => {
            match result {
                Ok(status) => {
                    use tokio::io::AsyncReadExt;
                    let mut stdout_buf = Vec::new();
                    let mut stderr_buf = Vec::new();
                    if let Some(mut h) = stdout_handle {
                        let _ = h.read_to_end(&mut stdout_buf).await;
                    }
                    if let Some(mut h) = stderr_handle {
                        let _ = h.read_to_end(&mut stderr_buf).await;
                    }
                    let stdout = String::from_utf8_lossy(&stdout_buf).to_string();
                    let stderr = String::from_utf8_lossy(&stderr_buf).to_string();
                    let combined = if stderr.is_empty() {
                        stdout
                    } else {
                        format!("{}\n{}", stdout, stderr)
                    };
                    (status.success(), combined)
                }
                Err(e) => (false, format!("Failed to execute command: {}", e)),
            }
        }
    }
}

/// Add a note to today's daily workspace note.
async fn execute_workspace_note(workspace: &Workspace, content: &str) -> (bool, String) {
    match workspace.note(content).await {
        Ok(()) => (true, "Note added".to_string()),
        Err(e) => (false, format!("Failed to add note: {}", e)),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_scheduler_config_defaults() {
        let config = SchedulerConfig::with_defaults();
        assert!(config.enabled);
        assert_eq!(config.tasks.len(), 5);

        let names: Vec<&str> = config.tasks.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"Daily review"));
        assert!(names.contains(&"Weekly review"));
        assert!(names.contains(&"Hourly check"));
        assert!(names.contains(&"Daily memory consolidation"));
        assert!(names.contains(&"Weekly session cleanup"));

        // All default tasks should be enabled
        for task in &config.tasks {
            assert!(task.enabled);
        }
    }

    #[tokio::test]
    async fn test_add_remove_task() {
        let config = SchedulerConfig::default();
        let scheduler = CronScheduler::new(config);

        // Initially empty
        assert!(scheduler.list_tasks().await.is_empty());

        // Add a task
        let id = scheduler
            .add_task(TaskConfig {
                name: "Test task".to_string(),
                cron: "*/5 * * * *".to_string(),
                task_type: TaskType::Shell {
                    command: "echo hello".to_string(),
                },
                enabled: true,
            })
            .await
            .unwrap();

        assert_eq!(scheduler.list_tasks().await.len(), 1);
        assert_eq!(scheduler.list_tasks().await[0].name, "Test task");

        // Remove it
        scheduler.remove_task(&id).await.unwrap();
        assert!(scheduler.list_tasks().await.is_empty());

        // Removing again should fail
        let result = scheduler.remove_task(&id).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_cron_parsing() {
        // 5-field standard cron should work
        assert!(compute_next_run("0 9 * * *").is_some());
        assert!(compute_next_run("0 9 * * 1").is_some());
        assert!(compute_next_run("*/5 * * * *").is_some());
        assert!(compute_next_run("0 * * * *").is_some());

        // 7-field cron should work
        assert!(compute_next_run("0 0 9 * * * *").is_some());

        // Invalid expressions should return None
        assert!(compute_next_run("not a cron").is_none());
        assert!(compute_next_run("").is_none());
    }

    #[test]
    fn test_normalize_cron_expr() {
        // 5-field -> 7-field
        assert_eq!(normalize_cron_expr("0 9 * * *"), "0 0 9 * * * *");
        assert_eq!(normalize_cron_expr("*/5 * * * *"), "0 */5 * * * * *");

        // 6-field -> 7-field
        assert_eq!(normalize_cron_expr("0 0 9 * * *"), "0 0 9 * * * *");

        // 7-field -> unchanged
        assert_eq!(normalize_cron_expr("0 0 9 * * * *"), "0 0 9 * * * *");
    }

    #[test]
    fn test_task_type_serialization() {
        // Heartbeat
        let heartbeat = TaskType::Heartbeat {
            frequency: "daily".to_string(),
        };
        let json = serde_json::to_string(&heartbeat).unwrap();
        let deserialized: TaskType = serde_json::from_str(&json).unwrap();
        match deserialized {
            TaskType::Heartbeat { frequency } => assert_eq!(frequency, "daily"),
            _ => panic!("Expected Heartbeat variant"),
        }

        // LlmPrompt
        let prompt = TaskType::LlmPrompt {
            prompt: "Summarize today".to_string(),
        };
        let json = serde_json::to_string(&prompt).unwrap();
        let deserialized: TaskType = serde_json::from_str(&json).unwrap();
        match deserialized {
            TaskType::LlmPrompt { prompt } => assert_eq!(prompt, "Summarize today"),
            _ => panic!("Expected LlmPrompt variant"),
        }

        // Shell
        let shell = TaskType::Shell {
            command: "echo hello".to_string(),
        };
        let json = serde_json::to_string(&shell).unwrap();
        let deserialized: TaskType = serde_json::from_str(&json).unwrap();
        match deserialized {
            TaskType::Shell { command } => assert_eq!(command, "echo hello"),
            _ => panic!("Expected Shell variant"),
        }

        // WorkspaceNote
        let note = TaskType::WorkspaceNote {
            content: "Remember this".to_string(),
        };
        let json = serde_json::to_string(&note).unwrap();
        let deserialized: TaskType = serde_json::from_str(&json).unwrap();
        match deserialized {
            TaskType::WorkspaceNote { content } => assert_eq!(content, "Remember this"),
            _ => panic!("Expected WorkspaceNote variant"),
        }

        // ContentPipeline — with optional fields omitted
        let pipeline = TaskType::ContentPipeline {
            input_path: "/tmp/video.mp4".to_string(),
            platform: "youtube".to_string(),
            title: "My Video".to_string(),
            description: "A test upload".to_string(),
            trim_start: Some("00:00:05".to_string()),
            trim_end: None,
            captions_srt: None,
            media_url: None,
        };
        let json = serde_json::to_string(&pipeline).unwrap();
        // Verify skip_serializing_if works: None fields should be absent
        assert!(!json.contains("trim_end"), "None fields should be omitted");
        let deserialized: TaskType = serde_json::from_str(&json).unwrap();
        match deserialized {
            TaskType::ContentPipeline { platform, title, trim_start, .. } => {
                assert_eq!(platform, "youtube");
                assert_eq!(title, "My Video");
                assert_eq!(trim_start, Some("00:00:05".to_string()));
            }
            _ => panic!("Expected ContentPipeline variant"),
        }

        // ContentPipeline — instagram with media_url
        let ig_pipeline = TaskType::ContentPipeline {
            input_path: "/tmp/reel.mp4".to_string(),
            platform: "instagram".to_string(),
            title: "My Reel".to_string(),
            description: "Reel caption".to_string(),
            trim_start: None,
            trim_end: None,
            captions_srt: None,
            media_url: Some("https://example.com/reel.mp4".to_string()),
        };
        let json = serde_json::to_string(&ig_pipeline).unwrap();
        assert!(json.contains("media_url"), "media_url should be serialized when Some");
        let deserialized: TaskType = serde_json::from_str(&json).unwrap();
        match deserialized {
            TaskType::ContentPipeline { platform, media_url, .. } => {
                assert_eq!(platform, "instagram");
                assert_eq!(media_url, Some("https://example.com/reel.mp4".to_string()));
            }
            _ => panic!("Expected ContentPipeline variant"),
        }
    }

    #[tokio::test]
    async fn test_shell_task_execution() {
        let (success, output) = execute_shell("echo 'scheduler test'").await;
        assert!(success, "Shell command should succeed");
        assert!(
            output.contains("scheduler test"),
            "Output should contain 'scheduler test', got: {}",
            output
        );
    }

    #[tokio::test]
    async fn test_workspace_note_task() {
        let tmp = TempDir::new().unwrap();
        let workspace = Workspace::new(tmp.path());
        workspace.init().await.unwrap();

        let (success, output) = execute_workspace_note(&workspace, "Test scheduler note").await;
        assert!(success, "Workspace note should succeed: {}", output);
        assert_eq!(output, "Note added");

        // Verify the note was actually written
        let daily = workspace.get_daily().await.unwrap();
        assert!(
            daily.contains("Test scheduler note"),
            "Daily note should contain our content, got: {}",
            daily
        );
    }

    #[tokio::test]
    async fn test_scheduler_tick() {
        let tmp = TempDir::new().unwrap();
        let workspace = Workspace::new(tmp.path());
        workspace.init().await.unwrap();

        // Create a scheduler with a task that uses a "every second" cron
        // (which means its next_run is always in the past relative to now).
        let config = SchedulerConfig {
            enabled: true,
            tasks: vec![TaskConfig {
                name: "Always due shell".to_string(),
                cron: "* * * * *".to_string(), // every minute
                task_type: TaskType::Shell {
                    command: "echo tick_output".to_string(),
                },
                enabled: true,
            }],
            ..Default::default()
        };

        let scheduler = CronScheduler::new(config);

        // The task's next_run should be set.
        {
            let tasks = scheduler.list_tasks().await;
            assert_eq!(tasks.len(), 1);
            assert!(tasks[0].next_run.is_some());
            assert!(tasks[0].enabled);
        }

        // Force next_run into the past so tick() picks it up.
        {
            let mut tasks = scheduler.tasks.write().await;
            tasks[0].next_run = Some(Utc::now() - chrono::Duration::seconds(60));
        }

        // We need a stub LlmClient. Since Shell tasks don't use the LLM,
        // we can create one that will never be called. We use Ollama pointed
        // at a non-existent server -- the Shell branch doesn't touch it.
        let llm = LlmClient::new(zeus_core::Provider::Ollama, "dummy".to_string()).unwrap();

        let executions = scheduler.tick(&workspace, &llm).await.unwrap();
        assert_eq!(executions.len(), 1, "Should have executed 1 task");
        assert!(executions[0].success, "Shell echo should succeed");
        assert!(
            executions[0].output.contains("tick_output"),
            "Output should contain tick_output, got: {}",
            executions[0].output
        );

        // History should be recorded.
        let history = scheduler.get_history(10).await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].task_name, "Always due shell");

        // Task should have been rescheduled.
        {
            let tasks = scheduler.list_tasks().await;
            assert!(tasks[0].last_run.is_some());
            assert!(tasks[0].next_run.is_some());
            // next_run should be in the future now
            assert!(tasks[0].next_run.unwrap() > Utc::now() - chrono::Duration::seconds(5));
        }
    }

    // ========================================================================
    // parse_human_schedule tests
    // ========================================================================

    #[test]
    fn test_parse_human_schedule_minutes() {
        let result = parse_human_schedule("every 5 minutes").unwrap();
        assert_eq!(result, "*/5 * * * *");
    }

    #[test]
    fn test_parse_human_schedule_hours() {
        let result = parse_human_schedule("every 2 hours").unwrap();
        assert_eq!(result, "0 */2 * * *");
    }

    #[test]
    fn test_parse_human_schedule_daily() {
        let result = parse_human_schedule("daily").unwrap();
        assert_eq!(result, "0 9 * * *");
    }

    #[test]
    fn test_parse_human_schedule_daily_at() {
        let result = parse_human_schedule("daily at 3pm").unwrap();
        assert_eq!(result, "0 15 * * *");
    }

    #[test]
    fn test_parse_human_schedule_daily_at_24h() {
        let result = parse_human_schedule("daily at 15:00").unwrap();
        assert_eq!(result, "0 15 * * *");
    }

    #[test]
    fn test_parse_human_schedule_weekly() {
        let result = parse_human_schedule("weekly on friday").unwrap();
        assert_eq!(result, "0 9 * * 5");
    }

    #[test]
    fn test_parse_human_schedule_monthly() {
        let result = parse_human_schedule("monthly").unwrap();
        assert_eq!(result, "0 9 1 * *");
    }

    #[test]
    fn test_parse_human_schedule_passthrough() {
        let result = parse_human_schedule("*/15 * * * *").unwrap();
        assert_eq!(result, "*/15 * * * *");
    }

    // ========================================================================
    // SchedulerDb tests
    // ========================================================================

    #[test]
    fn test_scheduler_db_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("scheduler.db");
        let db = SchedulerDb::new(&db_path);
        db.init().unwrap();

        let task = ScheduledTask {
            id: "test-id-1".to_string(),
            name: "Roundtrip task".to_string(),
            cron_expr: "*/10 * * * *".to_string(),
            task_type: TaskType::Shell {
                command: "echo roundtrip".to_string(),
            },
            enabled: true,
            last_run: None,
            next_run: Some(Utc::now()),
        };

        db.save_task(&task).unwrap();

        let loaded = db.load_tasks().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "test-id-1");
        assert_eq!(loaded[0].name, "Roundtrip task");
        assert_eq!(loaded[0].cron_expr, "*/10 * * * *");
        assert!(loaded[0].enabled);
        match &loaded[0].task_type {
            TaskType::Shell { command } => assert_eq!(command, "echo roundtrip"),
            _ => panic!("Expected Shell variant"),
        }
    }

    #[test]
    fn test_scheduler_db_delete() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("scheduler.db");
        let db = SchedulerDb::new(&db_path);
        db.init().unwrap();

        let task = ScheduledTask {
            id: "delete-me".to_string(),
            name: "Deletable task".to_string(),
            cron_expr: "0 9 * * *".to_string(),
            task_type: TaskType::WorkspaceNote {
                content: "hello".to_string(),
            },
            enabled: true,
            last_run: None,
            next_run: Some(Utc::now()),
        };

        db.save_task(&task).unwrap();
        assert_eq!(db.load_tasks().unwrap().len(), 1);

        db.delete_task("delete-me").unwrap();
        assert!(db.load_tasks().unwrap().is_empty());
    }

    // ========================================================================
    // Concurrency limit tests
    // ========================================================================

    #[test]
    fn test_max_concurrent_jobs_default() {
        // Serde default (from config files) should be 4.
        let config: SchedulerConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.max_concurrent_jobs, 4);

        // with_defaults() should also set 4.
        let config = SchedulerConfig::with_defaults();
        assert_eq!(config.max_concurrent_jobs, 4);
    }

    #[test]
    fn test_max_concurrent_jobs_config_serde() {
        let json_str = r#"{
            "enabled": true,
            "max_concurrent_jobs": 8,
            "tasks": [{
                "name": "Test",
                "cron": "0 * * * *",
                "enabled": true,
                "task_type": { "type": "shell", "command": "echo hi" }
            }]
        }"#;
        let config: SchedulerConfig = serde_json::from_str(json_str).unwrap();
        assert_eq!(config.max_concurrent_jobs, 8);
        assert_eq!(config.tasks.len(), 1);
    }

    #[test]
    fn test_scheduler_concurrency_fields() {
        let config = SchedulerConfig {
            max_concurrent_jobs: 2,
            ..Default::default()
        };
        let scheduler = CronScheduler::new(config);
        assert_eq!(scheduler.max_concurrent_jobs(), 2);
        assert_eq!(scheduler.active_job_count(), 0);
        assert_eq!(scheduler.available_slots(), 2);
    }

    #[test]
    fn test_scheduler_unlimited_concurrency() {
        let config = SchedulerConfig {
            max_concurrent_jobs: 0,
            ..Default::default()
        };
        let scheduler = CronScheduler::new(config);
        assert_eq!(scheduler.max_concurrent_jobs(), 0);
        assert_eq!(scheduler.available_slots(), 1024); // "unlimited" sentinel
    }

    #[tokio::test]
    async fn test_concurrency_limit_respected() {
        // Create a scheduler with max 2 concurrent jobs and 4 due tasks.
        // Each task sleeps briefly. With limit=2, tasks run 2-at-a-time.
        let config = SchedulerConfig {
            enabled: true,
            max_concurrent_jobs: 2,
            tasks: (0..4)
                .map(|i| TaskConfig {
                    name: format!("Sleep task {}", i),
                    cron: "* * * * *".to_string(),
                    task_type: TaskType::Shell {
                        command: "sleep 0.1 && echo done".to_string(),
                    },
                    enabled: true,
                })
                .collect(),
        };

        let scheduler = CronScheduler::new(config);

        // Force all tasks into the past.
        {
            let mut tasks = scheduler.tasks.write().await;
            for t in tasks.iter_mut() {
                t.next_run = Some(Utc::now() - chrono::Duration::seconds(60));
            }
        }

        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::new(tmp.path());
        workspace.init().await.unwrap();
        let llm = LlmClient::new(zeus_core::Provider::Ollama, "dummy".to_string()).unwrap();

        let executions = scheduler.tick(&workspace, &llm).await.unwrap();
        assert_eq!(executions.len(), 4, "All 4 tasks should have executed");
        assert!(
            executions.iter().all(|e| e.success),
            "All tasks should succeed"
        );
        assert_eq!(
            scheduler.active_job_count(),
            0,
            "No jobs should be active after tick"
        );
    }

    // ========================================================================
    // Cancellation / abort tests
    // ========================================================================

    #[tokio::test]
    async fn test_abort_nonexistent_task() {
        let config = SchedulerConfig::default();
        let scheduler = CronScheduler::new(config);

        let result = scheduler.abort_task("nonexistent").await;
        assert!(result.is_err(), "Aborting non-running task should fail");
    }

    #[tokio::test]
    async fn test_running_task_ids_empty() {
        let config = SchedulerConfig::default();
        let scheduler = CronScheduler::new(config);
        assert!(scheduler.running_task_ids().await.is_empty());
    }

    #[tokio::test]
    async fn test_cancellation_token_aborts_shell() {
        let cancel = CancellationToken::new();

        // Start a long-running shell command.
        let cancel_clone = cancel.clone();
        let handle =
            tokio::spawn(async move { execute_shell_cancellable("sleep 30", &cancel_clone).await });

        // Give the process a moment to start.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Cancel it.
        cancel.cancel();

        let (success, output) = handle.await.unwrap();
        assert!(!success, "Aborted task should not be successful");
        assert!(
            output.contains("aborted"),
            "Output should mention abort, got: {}",
            output
        );
    }

    #[tokio::test]
    async fn test_execute_task_cancellable_normal_completion() {
        let cancel = CancellationToken::new();
        let task = ScheduledTask {
            id: "test-cancel-ok".to_string(),
            name: "Quick task".to_string(),
            cron_expr: "* * * * *".to_string(),
            task_type: TaskType::Shell {
                command: "echo cancellable_ok".to_string(),
            },
            enabled: true,
            last_run: None,
            next_run: None,
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::new(tmp.path());
        workspace.init().await.unwrap();
        let llm = LlmClient::new(zeus_core::Provider::Ollama, "dummy".to_string()).unwrap();

        let execution = execute_task_cancellable(&workspace, &llm, &task, &cancel).await;
        assert!(execution.success, "Normal completion should succeed");
        assert!(
            execution.output.contains("cancellable_ok"),
            "Output should contain echo output, got: {}",
            execution.output
        );
    }

    #[tokio::test]
    async fn test_execute_task_cancellable_pre_cancelled() {
        let cancel = CancellationToken::new();
        cancel.cancel(); // Cancel before execution starts.

        let task = ScheduledTask {
            id: "test-pre-cancel".to_string(),
            name: "Pre-cancelled task".to_string(),
            cron_expr: "* * * * *".to_string(),
            task_type: TaskType::Shell {
                command: "echo should_not_run".to_string(),
            },
            enabled: true,
            last_run: None,
            next_run: None,
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::new(tmp.path());
        workspace.init().await.unwrap();
        let llm = LlmClient::new(zeus_core::Provider::Ollama, "dummy".to_string()).unwrap();

        let execution = execute_task_cancellable(&workspace, &llm, &task, &cancel).await;
        assert!(!execution.success, "Pre-cancelled task should fail");
        assert!(
            execution.output.contains("aborted") || execution.output.contains("abort"),
            "Output should mention abort, got: {}",
            execution.output
        );
    }

    #[tokio::test]
    async fn test_trigger_result_channel() {
        // Verify that trigger execution results are sent through the
        // trigger_result_tx channel.
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();

        let config = SchedulerConfig {
            enabled: true,
            tasks: vec![TaskConfig {
                name: "test_echo".to_string(),
                cron: "* * * * * * *".to_string(), // every second
                task_type: TaskType::Shell {
                    command: "echo 'trigger test output'".to_string(),
                },
                enabled: true,
            }],
            max_concurrent_jobs: 4,
        };

        let mut sched = CronScheduler::new(config);
        sched.set_trigger_result_tx(tx);

        // Force next_run into the past so tick() picks it up.
        {
            let mut tasks = sched.tasks.write().await;
            tasks[0].next_run = Some(Utc::now() - chrono::Duration::seconds(60));
        }

        let tmp = TempDir::new().unwrap();
        let workspace = Workspace::new(tmp.path().to_path_buf());
        let llm = LlmClient::new(zeus_core::Provider::Ollama, "dummy".to_string()).unwrap();

        let results = sched.tick(&workspace, &llm).await.unwrap();
        assert!(!results.is_empty(), "Expected at least one task to execute");

        // The result should have been sent through the channel.
        let msg = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            rx.recv(),
        )
        .await
        .expect("timed out waiting for trigger result")
        .expect("channel closed unexpectedly");

        assert!(
            msg.contains("test_echo"),
            "Trigger result should mention task name, got: {}",
            msg
        );
        assert!(
            msg.contains("trigger test output"),
            "Trigger result should contain command output, got: {}",
            msg
        );
    }
}

// ============================================================================
// CronJobHistory — per-job execution tracking
// ============================================================================

/// Per-job execution history and statistics.
#[derive(Debug, Clone, Serialize)]
pub struct CronJobHistory {
    /// Task ID
    pub job_id: String,
    /// Task name
    pub job_name: String,
    /// Total number of times this job has run
    pub run_count: u64,
    /// Last time this job ran
    pub last_run: Option<DateTime<Utc>>,
    /// Next scheduled run
    pub next_run: Option<DateTime<Utc>>,
    /// Status of the last execution ("ok", "error", or "pending")
    pub last_status: String,
    /// Output from the last execution
    pub last_output: String,
    /// Recent execution log (last 10 runs)
    pub recent_runs: Vec<TaskExecution>,
}

impl CronScheduler {
    /// Number of jobs currently executing.
    pub fn active_job_count(&self) -> u32 {
        self.active_jobs.load(Ordering::Relaxed)
    }

    /// Configured maximum concurrent jobs (0 = unlimited).
    pub fn max_concurrent_jobs(&self) -> u32 {
        self.max_concurrent_jobs
    }

    /// Available concurrency slots (semaphore permits remaining).
    pub fn available_slots(&self) -> usize {
        self.concurrency_limit.available_permits()
    }

    /// IDs of tasks currently executing.
    pub async fn running_task_ids(&self) -> Vec<String> {
        self.running_tokens.read().await.keys().cloned().collect()
    }

    /// Abort a running task by ID.
    ///
    /// Cancels the task's `CancellationToken`, causing the task to stop at the
    /// next cancellation-aware await point. Shell tasks are killed via
    /// `Child::kill()`, while LLM/heartbeat tasks abort at the next `.await`.
    ///
    /// Returns `Ok(())` if the task was found and cancelled, or an error if
    /// the task is not currently running.
    pub async fn abort_task(&self, task_id: &str) -> Result<()> {
        let tokens = self.running_tokens.read().await;
        if let Some(token) = tokens.get(task_id) {
            info!("Aborting task '{}'", task_id);
            token.cancel();
            Ok(())
        } else {
            Err(zeus_core::Error::NotFound(format!(
                "Task '{}' is not currently running",
                task_id
            )))
        }
    }

    /// Get history for a specific job by ID.
    pub async fn get_job_history(&self, job_id: &str) -> Option<CronJobHistory> {
        let tasks = self.tasks.read().await;
        let task = tasks.iter().find(|t| t.id == job_id)?;

        let history = self.history.read().await;
        let runs: Vec<TaskExecution> = history
            .iter()
            .filter(|e| e.task_id == job_id)
            .cloned()
            .collect();

        let run_count = runs.len() as u64;
        let last_exec = runs.last();
        let last_status = last_exec
            .map(|e| if e.success { "ok" } else { "error" })
            .unwrap_or("pending")
            .to_string();
        let last_output = last_exec.map(|e| e.output.clone()).unwrap_or_default();
        let recent_runs: Vec<TaskExecution> = runs.into_iter().rev().take(10).collect();

        Some(CronJobHistory {
            job_id: job_id.to_string(),
            job_name: task.name.clone(),
            run_count,
            last_run: task.last_run,
            next_run: task.next_run,
            last_status,
            last_output,
            recent_runs,
        })
    }

    /// Get a single task by ID.
    pub async fn get_task(&self, id: &str) -> Option<ScheduledTask> {
        self.tasks.read().await.iter().find(|t| t.id == id).cloned()
    }
}

// ============================================================================
// Serde for ScheduledTask (JSON API responses)
// ============================================================================

impl Serialize for ScheduledTask {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("ScheduledTask", 7)?;
        s.serialize_field("id", &self.id)?;
        s.serialize_field("name", &self.name)?;
        s.serialize_field("cron_expr", &self.cron_expr)?;
        s.serialize_field("task_type", &self.task_type)?;
        s.serialize_field("enabled", &self.enabled)?;
        s.serialize_field("last_run", &self.last_run)?;
        s.serialize_field("next_run", &self.next_run)?;
        s.end()
    }
}

impl Serialize for TaskExecution {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("TaskExecution", 6)?;
        s.serialize_field("task_id", &self.task_id)?;
        s.serialize_field("task_name", &self.task_name)?;
        s.serialize_field("started_at", &self.started_at)?;
        s.serialize_field("completed_at", &self.completed_at)?;
        s.serialize_field("success", &self.success)?;
        s.serialize_field("output", &self.output)?;
        s.end()
    }
}

// ============================================================================
// Job templates — pre-configured cron job definitions
// ============================================================================

/// Built-in cron job templates.
pub struct CronJobTemplate;

impl CronJobTemplate {
    /// Daily summary: review workspace activity and write a summary note.
    pub fn daily_summary() -> TaskConfig {
        TaskConfig {
            name: "Daily Summary".to_string(),
            cron: "daily at 6pm".to_string(),
            task_type: TaskType::LlmPrompt {
                prompt: "Review today's workspace activity. Summarize what was accomplished, \
                         any outstanding tasks, and what should be prioritized tomorrow. \
                         Write the summary as a daily note."
                    .to_string(),
            },
            enabled: true,
        }
    }

    /// Weekly report: comprehensive week-in-review.
    pub fn weekly_report() -> TaskConfig {
        TaskConfig {
            name: "Weekly Report".to_string(),
            cron: "weekly on friday".to_string(),
            task_type: TaskType::LlmPrompt {
                prompt: "Generate a weekly report summarizing: (1) completed tasks, \
                         (2) ongoing work, (3) blockers encountered, (4) metrics and progress. \
                         Format it as a structured report in the daily note."
                    .to_string(),
            },
            enabled: true,
        }
    }

    /// Memory cleanup: prune stale facts and consolidate memory.
    pub fn memory_cleanup() -> TaskConfig {
        TaskConfig {
            name: "Memory Cleanup".to_string(),
            cron: "daily at 3am".to_string(),
            task_type: TaskType::LlmPrompt {
                prompt: "Review the workspace memory store. Identify stale or redundant facts, \
                         outdated notes, and unused context. Suggest which entries can be archived \
                         or removed to keep the memory lean and relevant."
                    .to_string(),
            },
            enabled: true,
        }
    }

    /// Health check: verify system components are running.
    pub fn health_check() -> TaskConfig {
        TaskConfig {
            name: "Health Check".to_string(),
            cron: "every 15 minutes".to_string(),
            task_type: TaskType::Shell {
                command: "echo 'health_ok'".to_string(),
            },
            enabled: true,
        }
    }

    /// Community detection: re-cluster entities in the knowledge graph.
    pub fn community_detection() -> TaskConfig {
        TaskConfig {
            name: "Community Detection".to_string(),
            cron: "daily at 4am".to_string(),
            task_type: TaskType::LlmPrompt {
                prompt: "Run community detection on the memory knowledge graph to re-cluster \
                         related entities. Report how many communities were found and any \
                         notable changes in entity groupings."
                    .to_string(),
            },
            enabled: true,
        }
    }

    /// Memory promotion: promote high-value episodic memories to semantic.
    pub fn memory_promotion() -> TaskConfig {
        TaskConfig {
            name: "Memory Promotion".to_string(),
            cron: "every 6 hours".to_string(),
            task_type: TaskType::LlmPrompt {
                prompt: "Scan episodic memories for high-importance entries that should be \
                         promoted to semantic memory. Also run garbage collection to remove \
                         stale low-importance episodic memories older than 30 days."
                    .to_string(),
            },
            enabled: true,
        }
    }

    /// List all available templates.
    pub fn all() -> Vec<(&'static str, TaskConfig)> {
        vec![
            ("daily_summary", Self::daily_summary()),
            ("weekly_report", Self::weekly_report()),
            ("memory_cleanup", Self::memory_cleanup()),
            ("health_check", Self::health_check()),
            ("community_detection", Self::community_detection()),
            ("memory_promotion", Self::memory_promotion()),
        ]
    }

    /// Get a template by name.
    pub fn get(name: &str) -> Option<TaskConfig> {
        match name {
            "daily_summary" => Some(Self::daily_summary()),
            "weekly_report" => Some(Self::weekly_report()),
            "memory_cleanup" => Some(Self::memory_cleanup()),
            "health_check" => Some(Self::health_check()),
            "community_detection" => Some(Self::community_detection()),
            "memory_promotion" => Some(Self::memory_promotion()),
            _ => None,
        }
    }
}
