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
use zeus_core::{Config, Message, Result};
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
    /// Fire one autonomy-subsystem delivery tick.
    ///
    /// `kind` routes to the owning subsystem's delivery loop:
    /// - `"commitments"` → drain due commitments into a heartbeat note
    ///   (daily-capped, dedup-latched at the store boundary).
    /// - `"dreaming:light"` / `"dreaming:rem"` → run the matching wake phase.
    ///
    /// This variant is *only* emitted by a subsystem's `scheduler_tasks()`,
    /// which returns an empty `Vec` while the subsystem is disabled — so a
    /// `SubsystemTick` never reaches the scheduler unless the owner is ON.
    /// Off-by-default holds at that `enabled` gate; this arm adds no second
    /// gate and forks no second scheduler.
    SubsystemTick { kind: String },
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

/// When a due task's body is actually executed relative to the scheduler tick.
///
/// The scheduler ticks on a fixed (~30s) cadence. `WakeMode` decides whether a
/// task whose `next_run` has elapsed fires immediately on that tick, or is held
/// until the next heartbeat boundary so its delivery aligns with the agent's
/// heartbeat cadence rather than the finer scheduler granularity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WakeMode {
    /// Fire immediately on the scheduler tick once `next_run` has elapsed.
    /// This is the historical behavior and the default.
    #[default]
    Now,
    /// Hold a due task until the next heartbeat-frequency tick, snapping its
    /// execution to the heartbeat cadence instead of the 30s scheduler tick.
    NextHeartbeat,
}

impl WakeMode {
    /// Stable string token used for DB persistence and tool serialization.
    pub fn as_str(self) -> &'static str {
        match self {
            WakeMode::Now => "now",
            WakeMode::NextHeartbeat => "next_heartbeat",
        }
    }

    /// Parse from the persisted token. Unknown values fall back to `Now`.
    pub fn from_token(s: &str) -> Self {
        match s {
            "next_heartbeat" => WakeMode::NextHeartbeat,
            _ => WakeMode::Now,
        }
    }
}

/// Where a fired task's execution result is delivered.
///
/// When a task fires, its output can be routed to the agent in three ways.
/// `Channel` (default) sends the result to the gateway for immediate injection
/// into the agent's live context — the historical behavior. `HeartbeatNote`
/// also sends to the gateway but tags the message so it surfaces as a passive
/// heartbeat note rather than an interrupt. `SilentLedger` suppresses the
/// gateway send entirely — the execution is still recorded in history (the
/// ledger), but the agent is not actively notified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    /// Send the result to the gateway for immediate agent-context injection.
    /// Historical behavior and the default.
    #[default]
    Channel,
    /// Send to the gateway tagged as a passive heartbeat note (non-interrupt).
    HeartbeatNote,
    /// Suppress the gateway send; record in history only (silent ledger).
    SilentLedger,
}

impl DeliveryMode {
    /// Stable string token used for DB persistence and tool serialization.
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryMode::Channel => "channel",
            DeliveryMode::HeartbeatNote => "heartbeat_note",
            DeliveryMode::SilentLedger => "silent_ledger",
        }
    }

    /// Parse from the persisted token. Unknown values fall back to `Channel`.
    pub fn from_token(s: &str) -> Self {
        match s {
            "heartbeat_note" => DeliveryMode::HeartbeatNote,
            "silent_ledger" => DeliveryMode::SilentLedger,
            _ => DeliveryMode::Channel,
        }
    }
}

/// Configuration for a single scheduled task (serializable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    /// Human-readable name for this task.
    pub name: String,
    /// Cron expression (standard 5-field or 7-field).
    /// 5-field expressions like "0 9 * * *" are automatically converted to
    /// 7-field by prepending "0 " (seconds) and appending " *" (year).
    ///
    /// Ignored when [`run_at`](Self::run_at) is set (one-shot tasks fire at the
    /// timestamp, not on a recurring cron). May be left empty for one-shots.
    #[serde(default)]
    pub cron: String,
    /// What to do when the schedule fires.
    pub task_type: TaskType,
    /// Whether this task is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// One-shot fire time. When set, the task fires once at/after this instant
    /// rather than on the `cron` schedule. Combine with `run_once` (the usual
    /// case) to self-delete after firing. `None` = recurring cron task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_at: Option<DateTime<Utc>>,
    /// Fire exactly once, then self-delete via `delete_task` (no re-fire).
    /// Defaults to `false` (recurring). Set automatically for `run_at` tasks
    /// created through the agent tool, but independent here for flexibility.
    #[serde(default)]
    pub run_once: bool,
    /// When a due task fires relative to the scheduler tick. `Now` (default)
    /// fires immediately; `NextHeartbeat` defers to the next heartbeat boundary.
    #[serde(default)]
    pub wake_mode: WakeMode,
    /// Where a fired task's result is delivered. `Channel` (default) injects
    /// into live agent context; `HeartbeatNote` surfaces as a passive note;
    /// `SilentLedger` records to history only without notifying the agent.
    #[serde(default)]
    pub delivery_mode: DeliveryMode,
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

/// #187 backlog reconcile: grace window for stale one-shots on load.
///
/// A one-shot (`run_once=true`) whose `next_run` is overdue by more than this
/// is DROPPED at load instead of replayed — these are dead polls / old
/// `loop`-spawned one-shots whose moment has passed (firing them replays
/// history with zero future value).
///
/// Trade-off (tunable): if a seat is down for longer than this while a
/// legitimate one-shot was due (e.g. a 5pm post missed across a >1h restart),
/// that one-shot is dropped rather than fired late. This is the correct
/// default — late-firing a stale post is worse than skipping it — and every
/// drop is `warn!`-logged (see `with_db`), so a wrongly-dropped task is
/// recoverable from the log, not silent.
const STALE_GRACE: chrono::Duration = chrono::Duration::hours(1);

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
                    run_at: None,
                    run_once: false,
                    wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
                },
                TaskConfig {
                    name: "Weekly review".to_string(),
                    cron: "0 9 * * 1".to_string(),
                    task_type: TaskType::Heartbeat {
                        frequency: "weekly".to_string(),
                    },
                    enabled: true,
                    run_at: None,
                    run_once: false,
                    wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
                },
                TaskConfig {
                    name: "Hourly check".to_string(),
                    cron: "0 * * * *".to_string(),
                    task_type: TaskType::Heartbeat {
                        frequency: "hourly".to_string(),
                    },
                    enabled: true,
                    run_at: None,
                    run_once: false,
                    wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
                },
                TaskConfig {
                    name: "Daily memory consolidation".to_string(),
                    cron: "0 2 * * *".to_string(),
                    task_type: TaskType::LlmPrompt {
                        prompt: "Consolidate recent memories: review today's interactions, extract key facts and decisions, prune stale or redundant entries. Respond with a brief summary of what was consolidated.".to_string(),
                    },
                    enabled: true,
                    run_at: None,
                    run_once: false,
                    wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
                },
                TaskConfig {
                    name: "Weekly session cleanup".to_string(),
                    cron: "0 3 * * 0".to_string(),
                    task_type: TaskType::LlmPrompt {
                        prompt: "Review old sessions from the past week. Summarize key outcomes and clean up any stale session state. Respond with a brief cleanup report.".to_string(),
                    },
                    enabled: true,
                    run_at: None,
                    run_once: false,
                    wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
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
    /// Fire exactly once, then self-delete (no re-fire). Set from
    /// [`TaskConfig::run_once`] (or implied by a `run_at` one-shot).
    pub run_once: bool,
    /// When this task fires relative to the scheduler tick. See [`WakeMode`].
    pub wake_mode: WakeMode,
    /// Where this task's fired result is delivered. See [`DeliveryMode`].
    pub delivery_mode: DeliveryMode,
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

            // One-shot (`run_at`) tasks fire at the timestamp; recurring tasks
            // derive `next_run` from the cron expression.
            let next_run = if !tc.enabled {
                None
            } else if let Some(at) = tc.run_at {
                Some(at)
            } else {
                compute_next_run(&cron_expr)
            };

            if tc.enabled && next_run.is_none() {
                warn!(
                    "Task '{}' has an invalid cron expression '{}'; disabling it",
                    tc.name, tc.cron
                );
            }

            // A `run_at` one-shot is implicitly run-once even if the caller
            // didn't set the flag — there's no cron to recur on.
            let run_once = tc.run_once || tc.run_at.is_some();

            tasks.push(ScheduledTask {
                id,
                name: tc.name.clone(),
                cron_expr,
                task_type: tc.task_type.clone(),
                enabled: tc.enabled && next_run.is_some(),
                last_run: None,
                next_run,
                run_once,
                wake_mode: tc.wake_mode,
                delivery_mode: tc.delivery_mode,
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

        // #187 backlog reconcile. On load we must NOT let the first tick replay
        // days of stale history. `max_concurrent_jobs` only caps the burst
        // *width* (semaphore acquired inside the due-loop) — it does not stop
        // all N overdue tasks from eventually firing. The reconcile is the real
        // backlog filter, applied here at the single load chokepoint. Every
        // action is audit-logged: these rows are real history and the reconcile
        // mutates/drops them, so a wrongly-dropped legit task must leave a trail.
        let now = Utc::now();
        let mut stale_ids: Vec<String> = Vec::new();
        for mut task in persisted {
            // Avoid duplicates by id OR name. Config-default heartbeats (Daily/
            // Weekly/Hourly/Memory) get a fresh uuid each startup, so an id-only
            // check never matches the persisted copy → they pile up (+4/start),
            // becoming a trigger storm that wedges the gateway (#238). Name-dedup
            // keeps exactly one task per name across restarts.
            if tasks.iter().any(|t| t.id == task.id || t.name == task.name) {
                continue;
            }

            let overdue = task.next_run.map(|nr| nr < now).unwrap_or(false);

            if overdue {
                if task.run_once {
                    // One-shot overdue beyond the grace window → drop (don't replay).
                    let overdue_by = task.next_run.map(|nr| now - nr).unwrap_or_default();
                    if overdue_by > STALE_GRACE {
                        warn!(
                            "scheduler reconcile: DROPPING stale one-shot '{}' ({}) — was due {} ({}m overdue, grace {}m)",
                            task.id,
                            task.name,
                            task.next_run.map(|n| n.to_rfc3339()).unwrap_or_else(|| "?".into()),
                            overdue_by.num_minutes(),
                            STALE_GRACE.num_minutes(),
                        );
                        stale_ids.push(task.id.clone());
                        continue;
                    }
                    // Within grace → keep; it'll fire on the next tick (intended).
                } else {
                    // Recurring overdue → re-anchor to the next FUTURE slot via the
                    // same advance primitive tick_inner uses post-fire. Never replay
                    // N catch-up fires; resume on the normal cadence.
                    let old = task.next_run;
                    match compute_next_run(&task.cron_expr) {
                        Some(next) => {
                            info!(
                                "scheduler reconcile: RE-ANCHORING recurring '{}' ({}) next_run {} -> {} (was overdue, no catch-up replay)",
                                task.id,
                                task.name,
                                old.map(|n| n.to_rfc3339()).unwrap_or_else(|| "?".into()),
                                next.to_rfc3339(),
                            );
                            task.next_run = Some(next);
                        }
                        None => {
                            // cron can't produce a future slot (disabled/invalid) →
                            // skip+warn, don't poison the load.
                            warn!(
                                "scheduler reconcile: SKIPPING recurring '{}' ({}) — cron '{}' yields no future run",
                                task.id, task.name, task.cron_expr,
                            );
                            stale_ids.push(task.id.clone());
                            continue;
                        }
                    }
                }
            }

            tasks.push(task);
        }

        // Delete dropped/skipped rows from the DB so they don't reload next start.
        for id in &stale_ids {
            if let Err(e) = db.delete_task(id) {
                warn!("scheduler reconcile: failed to delete stale task '{}': {}", id, e);
            }
        }

        // Save all surviving tasks to db (persists re-anchored next_run too).
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

        // A `run_at` one-shot is implicitly run-once — there's no cron to recur on.
        let run_once = config.run_once || config.run_at.is_some();

        // One-shot (`run_at`) tasks fire at the timestamp and need no cron;
        // recurring tasks parse the cron expression up front.
        let cron_expr = if config.run_at.is_some() {
            // Keep any provided cron for display, else empty.
            parse_human_schedule(&config.cron).unwrap_or_else(|_| config.cron.clone())
        } else {
            parse_human_schedule(&config.cron)?
        };

        let next_run = if !config.enabled {
            None
        } else if let Some(at) = config.run_at {
            Some(at)
        } else {
            compute_next_run(&cron_expr)
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
            run_once,
            wake_mode: config.wake_mode,
            delivery_mode: config.delivery_mode,
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
    pub async fn start(
        &mut self,
        config: Arc<Config>,
        workspace: Arc<Workspace>,
        llm: Arc<LlmClient>,
    ) -> Result<()> {
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
        let db = self.db.clone();

        tokio::spawn(async move {
            scheduler_loop(
                config,
                tasks,
                history,
                workspace,
                llm,
                rx,
                concurrency_limit,
                active_jobs,
                running_tokens,
                trigger_result_tx,
                db,
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
    pub async fn tick(
        &self,
        config: &Config,
        workspace: &Workspace,
        llm: &LlmClient,
    ) -> Result<Vec<TaskExecution>> {
        // A manual `tick()` is treated as a heartbeat boundary, so both
        // `WakeMode::Now` and `WakeMode::NextHeartbeat` due tasks fire.
        self.tick_inner(config, workspace, llm, true).await
    }

    /// Single-step tick honoring [`WakeMode`]. When `is_heartbeat_tick` is
    /// false, `WakeMode::NextHeartbeat` due tasks are held back until a tick
    /// that aligns with the heartbeat boundary.
    pub async fn tick_inner(
        &self,
        config: &Config,
        workspace: &Workspace,
        llm: &LlmClient,
        is_heartbeat_tick: bool,
    ) -> Result<Vec<TaskExecution>> {
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
                    // WakeMode::NextHeartbeat tasks only fire on a heartbeat tick.
                    if task.wake_mode == WakeMode::NextHeartbeat && !is_heartbeat_tick {
                        continue;
                    }
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

            let execution = execute_task(config, workspace, llm, task).await;

            self.active_jobs.fetch_sub(1, Ordering::Relaxed);
            // _permit dropped here, releasing the semaphore slot.

            // Update the task's timing — or self-delete a one-shot.
            let next_run;
            {
                let mut tasks = self.tasks.write().await;
                if task.run_once {
                    // One-shot fired: remove from the live set so it never
                    // re-fires. Done regardless of DB presence; the persisted
                    // row (if any) is deleted below.
                    tasks.retain(|t| t.id != task.id);
                    next_run = None;
                    debug!("One-shot task '{}' fired and self-deleted", task.name);
                } else if let Some(t) = tasks.get_mut(*idx) {
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
                if task.run_once {
                    if let Err(e) = db.delete_task(&task.id) {
                        warn!("Failed to delete one-shot task '{}': {}", task.id, e);
                    }
                } else {
                    let status = if execution.success { "ok" } else { "error" };
                    let _ = db.update_execution(
                        &task.id,
                        execution.started_at,
                        next_run,
                        status,
                        &execution.output,
                    );
                }
            }

            // Record in history.
            self.history.write().await.push(execution.clone());

            // Send trigger result to gateway for agent context injection —
            // gated by the task's DeliveryMode. SilentLedger suppresses the
            // send entirely (history already recorded above); HeartbeatNote
            // tags the message as a passive note.
            if task.delivery_mode != DeliveryMode::SilentLedger
                && let Some(ref tx) = self.trigger_result_tx
            {
                let status = if execution.success { "✅" } else { "❌" };
                let prefix = if task.delivery_mode == DeliveryMode::HeartbeatNote {
                    "Heartbeat-note"
                } else {
                    "Trigger"
                };
                let msg = format!(
                    "[{}: {}] {}\n{}",
                    prefix, task.name, status, execution.output
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
    // v2: one-shot support — fire once then self-delete (#174 P1).
    "ALTER TABLE scheduled_tasks ADD COLUMN run_once INTEGER NOT NULL DEFAULT 0;",
    // v3: wake-modes — 'now' (default) | 'next_heartbeat' (#174 P2).
    "ALTER TABLE scheduled_tasks ADD COLUMN wake_mode TEXT NOT NULL DEFAULT 'now';",
    // v4: delivery-modes — 'channel' (default) | 'heartbeat_note' | 'silent_ledger' (#174 P3).
    "ALTER TABLE scheduled_tasks ADD COLUMN delivery_mode TEXT NOT NULL DEFAULT 'channel';",
];

/// SQLite-backed persistence for scheduled tasks.
#[derive(Clone)]
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
                (id, name, schedule, schedule_type, task_payload, enabled, last_run, next_run, last_status, last_output, created_at, updated_at, run_once, wake_mode, delivery_mode)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
                task.run_once as i32,
                task.wake_mode.as_str(),
                task.delivery_mode.as_str(),
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
                "SELECT id, name, schedule, task_payload, enabled, last_run, next_run, run_once, wake_mode, delivery_mode FROM scheduled_tasks"
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
                let run_once: i32 = row.get(7)?;
                let wake_mode: String = row.get(8)?;
                let delivery_mode: String = row.get(9)?;
                Ok((id, name, schedule, payload, enabled, last_run, next_run, run_once, wake_mode, delivery_mode))
            })
            .map_err(|e| zeus_core::Error::Config(format!("Failed to query tasks: {}", e)))?;

        let mut tasks = Vec::new();
        for row in rows {
            // #187 load-harden: one malformed row must NOT poison the entire
            // load (the old `?` aborted the whole pass, silently dropping ALL
            // persistence). Skip+warn the bad row and keep going. This is also
            // the defense against a talos/prometheus schema race where a row
            // predates a migration.
            let (id, name, schedule, payload, enabled, last_run, next_run, run_once, wake_mode, delivery_mode) =
                match row {
                    Ok(r) => r,
                    Err(e) => {
                        warn!("scheduler: skipping unreadable task row: {}", e);
                        continue;
                    }
                };

            let task_type: TaskType = match serde_json::from_str(&payload) {
                Ok(t) => t,
                Err(e) => {
                    warn!(
                        "scheduler: skipping task '{}' ({}): malformed payload: {}",
                        id, name, e
                    );
                    continue;
                }
            };

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
                wake_mode: WakeMode::from_token(&wake_mode),
                delivery_mode: DeliveryMode::from_token(&delivery_mode),
                run_once: run_once != 0,
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
    config: Arc<Config>,
    tasks: Arc<RwLock<Vec<ScheduledTask>>>,
    history: Arc<RwLock<Vec<TaskExecution>>>,
    workspace: Arc<Workspace>,
    llm: Arc<LlmClient>,
    mut shutdown: watch::Receiver<bool>,
    concurrency_limit: Arc<Semaphore>,
    active_jobs: Arc<AtomicU32>,
    running_tokens: Arc<RwLock<HashMap<String, CancellationToken>>>,
    trigger_result_tx: Option<mpsc::UnboundedSender<String>>,
    db: Option<SchedulerDb>,
) {
    // The scheduler ticks every 30s. A heartbeat boundary is the coarser agent
    // cadence (~5 min = every 10th tick). `WakeMode::NextHeartbeat` tasks that
    // are due are held until a heartbeat-aligned tick so their delivery snaps to
    // the heartbeat cadence rather than the fine 30s scheduler granularity.
    const TICKS_PER_HEARTBEAT: u64 = 10;
    let mut tick_count: u64 = 0;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                tick_count = tick_count.wrapping_add(1);
                let is_heartbeat_tick = tick_count % TICKS_PER_HEARTBEAT == 0;
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
                                // WakeMode::NextHeartbeat: a due task is held
                                // until a heartbeat-aligned tick. WakeMode::Now
                                // (default) fires on the tick it became due.
                                if task.wake_mode == WakeMode::NextHeartbeat
                                    && !is_heartbeat_tick {
                                        continue;
                                    }
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
                    let cfg = config.clone();
                    let ws = workspace.clone();
                    let llm_c = llm.clone();
                    let active = active_jobs.clone();
                    let tasks_c = tasks.clone();
                    let history_c = history.clone();
                    let tokens = running_tokens.clone();
                    let cancel = CancellationToken::new();
                    let task_id = task.id.clone();
                    let task_name = task.name.clone();
                    let delivery_mode = task.delivery_mode;
                    let result_tx = trigger_result_tx.clone();
                    let db_c = db.clone();

                    // Register the cancellation token so abort_task() can find it.
                    tokens.write().await.insert(task_id.clone(), cancel.clone());

                    active.fetch_add(1, Ordering::Relaxed);

                    handles.push(tokio::spawn(async move {
                        let execution = execute_task_cancellable(&cfg, &ws, &llm_c, &task, &cancel).await;

                        // Unregister the cancellation token.
                        tokens.write().await.remove(&task_id);

                        // Update the task's timing — or self-delete a one-shot.
                        {
                            let mut tasks_w = tasks_c.write().await;
                            // The index may have shifted if another one-shot was
                            // removed this tick; match by id to stay correct.
                            if let Some(pos) = tasks_w.iter().position(|t| t.id == task_id) {
                                if tasks_w[pos].run_once {
                                    // One-shot fired: remove from the live set so it
                                    // never re-fires, and delete the persisted row.
                                    tasks_w.remove(pos);
                                    if let Some(ref db) = db_c
                                        && let Err(e) = db.delete_task(&task_id) {
                                            warn!(
                                                "Failed to delete one-shot task '{}': {}",
                                                task_id, e
                                            );
                                        }
                                    debug!("One-shot task '{}' fired and self-deleted", task_name);
                                } else {
                                    tasks_w[pos].last_run = Some(execution.started_at);
                                    tasks_w[pos].next_run =
                                        compute_next_run(&tasks_w[pos].cron_expr);
                                }
                            }
                        }

                        // Send trigger result to gateway for agent context
                        // injection — gated by DeliveryMode. SilentLedger
                        // suppresses the send (history already recorded below);
                        // HeartbeatNote tags the message as a passive note.
                        if delivery_mode != DeliveryMode::SilentLedger
                            && let Some(ref tx) = result_tx
                        {
                            let status = if execution.success { "✅" } else { "❌" };
                            let prefix = if delivery_mode == DeliveryMode::HeartbeatNote {
                                "Heartbeat-note"
                            } else {
                                "Trigger"
                            };
                            let msg = format!(
                                "[{}: {}] {}\n{}",
                                prefix, task_name, status, execution.output
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
    config: &Config,
    workspace: &Workspace,
    llm: &LlmClient,
    task: &ScheduledTask,
) -> TaskExecution {
    let started_at = Utc::now();
    let (success, output) = match &task.task_type {
        TaskType::Heartbeat { frequency } => execute_heartbeat(workspace, llm, frequency).await,
        TaskType::LlmPrompt { prompt } => execute_llm_prompt(config, workspace, llm, prompt).await,
        TaskType::Shell { command } => execute_shell(command).await,
        TaskType::WorkspaceNote { content } => execute_workspace_note(workspace, content).await,
        TaskType::SubsystemTick { kind } => execute_subsystem_tick(workspace, llm, kind).await,
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
    config: &Config,
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
        result = execute_task_inner(config, workspace, llm, task, cancel) => result,
    }
}

/// Inner task execution that is cancellation-aware for shell commands.
///
/// For shell tasks, spawns the child process and monitors the cancellation
/// token, killing the process if cancelled. For other task types, runs
/// normally (they will be interrupted at the next .await by the outer select).
async fn execute_task_inner(
    config: &Config,
    workspace: &Workspace,
    llm: &LlmClient,
    task: &ScheduledTask,
    cancel: &CancellationToken,
) -> TaskExecution {
    let started_at = Utc::now();
    let (success, output) = match &task.task_type {
        TaskType::Shell { command } => execute_shell_cancellable(command, cancel).await,
        TaskType::Heartbeat { frequency } => execute_heartbeat(workspace, llm, frequency).await,
        TaskType::LlmPrompt { prompt } => execute_llm_prompt(config, workspace, llm, prompt).await,
        TaskType::WorkspaceNote { content } => execute_workspace_note(workspace, content).await,
        TaskType::SubsystemTick { kind } => execute_subsystem_tick(workspace, llm, kind).await,
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

/// Run a scheduled agent turn: wake a tool-capable `Agent` and let the model
/// execute a real turn (it can call tools — post on X, run shell, write files).
///
/// This is the #173 upgrade. Previously this fired a raw, one-shot
/// `llm.complete(&messages, &[], …)` with an EMPTY tools slice — the model
/// could think but could not act. We now build an `Agent` (via the same path
/// `experiment.rs` uses) and call `Agent::run`, so scheduled prompts can
/// actually take action.
///
/// `channels` is `None` here (option A, #173): the scheduled agent gets the
/// full API tool loop (X, shell, web, files) but cannot push to a *live*
/// Discord/Slack channel mid-turn. Result delivery is handled out-of-band via
/// the `trigger_result_tx` → gateway path, independent of the channels handle.
/// Live-channel push from scheduled turns is deferred to #174 phase-2.
async fn execute_llm_prompt(
    config: &Config,
    workspace: &Workspace,
    llm: &LlmClient,
    prompt: &str,
) -> (bool, String) {
    let session = zeus_session::Session::new(&config.sessions);
    let mut agent = zeus_agent::Agent::new(
        config.clone(),
        (*llm).clone(),
        (*workspace).clone(),
        session,
        None, // option A: no live ChannelManager — full tool loop, no mid-turn channel push
    );

    // Timeout watchdog: a scheduled agent turn must not hold its semaphore
    // permit forever if the model call stalls (network hang, provider wedge).
    // The agent loop has its own internal iteration cap, but this is a
    // wall-clock backstop. On timeout we report run-failure (NOT false-green)
    // so the executor records a job-error and the row can be rescheduled.
    const AGENT_TURN_TIMEOUT_SECS: u64 = 600;
    match tokio::time::timeout(
        std::time::Duration::from_secs(AGENT_TURN_TIMEOUT_SECS),
        agent.run(prompt),
    )
    .await
    {
        Ok(Ok(response)) => (true, response),
        Ok(Err(e)) => (false, format!("Agent run error: {}", e)),
        Err(_) => (
            false,
            format!(
                "Agent turn timed out after {}s (watchdog)",
                AGENT_TURN_TIMEOUT_SECS
            ),
        ),
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

/// Route one autonomy-subsystem delivery tick to its owning subsystem.
///
/// Reaching this arm already means the owner is enabled (a disabled subsystem
/// emits no `SubsystemTick` from its `scheduler_tasks()`), so this fn does no
/// second enable-check — it just delivers.
async fn execute_subsystem_tick(
    workspace: &Workspace,
    llm: &LlmClient,
    kind: &str,
) -> (bool, String) {
    match kind {
        "commitments" => {
            // Store-backed delivery: drain due commitments into a heartbeat
            // note. The store enforces the daily cap on *creation*; delivery
            // dedups by flipping each delivered commitment to `Attempted` so it
            // never resurfaces.
            let store = match crate::commitments::CommitmentStore::default_path() {
                Ok(s) => s,
                Err(e) => return (false, format!("commitments: open store failed: {}", e)),
            };
            match crate::commitments::deliver_due(&store, workspace).await {
                Ok(0) => (true, "commitments: nothing due".to_string()),
                Ok(n) => (true, format!("commitments: delivered {}", n)),
                Err(e) => (false, format!("commitments: delivery failed: {}", e)),
            }
        }
        "standing-orders" => {
            // Store-backed surfacing: push any priority-windowed overdue orders
            // into a heartbeat note. No latch — a standing order re-surfaces
            // each cadence until the agent acts on it (record_action resets
            // last_acted_at, dropping it out of stale()).
            let store = match crate::standing_orders::StandingOrderStore::default_path() {
                Ok(s) => s,
                Err(e) => {
                    return (false, format!("standing-orders: open store failed: {}", e));
                }
            };
            match crate::standing_orders::surface_stale(&store, workspace).await {
                Ok(0) => (true, "standing-orders: nothing overdue".to_string()),
                Ok(n) => (true, format!("standing-orders: surfaced {} stale", n)),
                Err(e) => (false, format!("standing-orders: surface failed: {}", e)),
            }
        }
        "dreaming:light" | "dreaming:rem" => {
            // Production `DreamProvider` (#143): recall via Mnemosyne, narrative
            // via the LLM, lesson/narrative writes via the Workspace. Off-by-
            // default still holds upstream — this arm is only reached once the
            // owner flips `enabled` (a disabled DreamingEngine emits no
            // `SubsystemTick`, so no tick lands here).
            let mnemosyne = match zeus_mnemosyne::Mnemosyne::default().await {
                Ok(m) => m,
                Err(e) => return (false, format!("{}: open mnemosyne failed: {}", kind, e)),
            };
            let provider =
                crate::dreaming::WorkspaceDreamProvider::new(&mnemosyne, workspace, llm);

            // Default config drives lookback/cap; the phase is carried by `kind`.
            let engine =
                crate::dreaming::DreamingEngine::new(crate::dreaming::DreamingConfig::default());
            let phase = if kind.ends_with(":rem") {
                crate::dreaming::DreamPhase::Rem
            } else {
                crate::dreaming::DreamPhase::Light
            };

            match engine.tick(phase, &provider).await {
                Ok(r) if r.llm_called => (
                    true,
                    format!(
                        "{}: reviewed {}, promoted {} (1 LLM call)",
                        kind, r.items_reviewed, r.lessons_promoted
                    ),
                ),
                Ok(r) => (
                    true,
                    format!("{}: reviewed {} (no LLM call)", kind, r.items_reviewed),
                ),
                Err(e) => (false, format!("{}: tick failed: {}", kind, e)),
            }
        }
        "task-flows" => {
            // Run-engine delivery: drive every pending (and resume every
            // restart-orphaned running) flow through the pending→running→done
            // state machine, surfacing a one-line summary into the workspace
            // note. The store's transition guards keep this idempotent — a
            // done flow re-ticks idle.
            let store = match crate::taskflow::FlowStore::default_path() {
                Ok(s) => s,
                Err(e) => return (false, format!("task-flows: open store failed: {}", e)),
            };
            match store.tick(workspace).await {
                Ok(0) => (true, "task-flows: nothing to run".to_string()),
                Ok(n) => (true, format!("task-flows: completed {}", n)),
                Err(e) => (false, format!("task-flows: run failed: {}", e)),
            }
        }
        other => (false, format!("unknown subsystem tick kind: {}", other)),
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
                run_at: None,
                run_once: false,
                wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
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
                run_at: None,
                run_once: false,
                wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
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

        let executions = scheduler.tick(&Config::default(), &workspace, &llm).await.unwrap();
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

    /// #174 P1 — a one-shot (`run_at` + implied `run_once`) task fires exactly
    /// once at/after its timestamp, then self-deletes (no re-fire).
    #[tokio::test]
    async fn test_one_shot_fires_once_then_self_deletes() {
        let tmp = TempDir::new().unwrap();
        let workspace = Workspace::new(tmp.path());
        workspace.init().await.unwrap();

        // run_at in the (recent) past so the very first tick picks it up.
        // run_at implies run_once, even though we don't set the flag.
        let fire_at = Utc::now() - chrono::Duration::seconds(5);
        let config = SchedulerConfig {
            enabled: true,
            tasks: vec![TaskConfig {
                name: "one-shot echo".to_string(),
                cron: String::new(), // no recurring schedule
                task_type: TaskType::Shell {
                    command: "echo one_shot_output".to_string(),
                },
                enabled: true,
                run_at: Some(fire_at),
                run_once: false, // implied by run_at
                wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
            }],
            ..Default::default()
        };

        let scheduler = CronScheduler::new(config);

        // next_run must be exactly the run_at instant, and run_once implied true.
        {
            let tasks = scheduler.list_tasks().await;
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0].next_run, Some(fire_at));
            assert!(tasks[0].run_once, "run_at must imply run_once");
        }

        let llm = LlmClient::new(zeus_core::Provider::Ollama, "dummy".to_string()).unwrap();

        // First tick: fires once.
        let execs = scheduler.tick(&Config::default(), &workspace, &llm).await.unwrap();
        assert_eq!(execs.len(), 1, "one-shot should fire exactly once");
        assert!(execs[0].output.contains("one_shot_output"));

        // It must have self-deleted — gone from the live set.
        let tasks = scheduler.list_tasks().await;
        assert!(
            tasks.is_empty(),
            "one-shot must self-delete after firing, found: {:?}",
            tasks.iter().map(|t| &t.name).collect::<Vec<_>>()
        );

        // Second tick: nothing fires (no re-fire).
        let execs2 = scheduler.tick(&Config::default(), &workspace, &llm).await.unwrap();
        assert!(execs2.is_empty(), "one-shot must not re-fire on a later tick");
    }

    /// #174 P2: a `WakeMode::NextHeartbeat` task that is due is held back on a
    /// non-heartbeat tick and only fires on a heartbeat-aligned tick.
    #[tokio::test]
    async fn test_next_heartbeat_wake_mode_holds_until_heartbeat_tick() {
        let tmp = TempDir::new().unwrap();
        let workspace = Workspace::new(tmp.path());
        workspace.init().await.unwrap();
        let llm = LlmClient::new(zeus_core::Provider::Ollama, "dummy".to_string()).unwrap();

        // Due now, but wake_mode = NextHeartbeat: must not fire off-heartbeat.
        let fire_at = Utc::now() - chrono::Duration::seconds(5);
        let config = SchedulerConfig {
            enabled: true,
            tasks: vec![TaskConfig {
                name: "heartbeat-gated echo".to_string(),
                cron: String::new(),
                task_type: TaskType::Shell {
                    command: "echo heartbeat_gated".to_string(),
                },
                enabled: true,
                run_at: Some(fire_at),
                run_once: false,
                wake_mode: WakeMode::NextHeartbeat,
                delivery_mode: DeliveryMode::Channel,
            }],
            ..Default::default()
        };
        let scheduler = CronScheduler::new(config);

        // Non-heartbeat tick: the due NextHeartbeat task is held back.
        let off = scheduler
            .tick_inner(&Config::default(), &workspace, &llm, false)
            .await
            .unwrap();
        assert!(
            off.is_empty(),
            "NextHeartbeat task must not fire on a non-heartbeat tick"
        );
        // Still present (not fired, not self-deleted yet).
        assert_eq!(scheduler.list_tasks().await.len(), 1);

        // Heartbeat-aligned tick: now it fires.
        let on = scheduler
            .tick_inner(&Config::default(), &workspace, &llm, true)
            .await
            .unwrap();
        assert_eq!(on.len(), 1, "NextHeartbeat task must fire on a heartbeat tick");
        assert!(on[0].success);
    }

    #[tokio::test]
    async fn test_delivery_mode_silent_ledger_suppresses_send_but_records_history() {
        let tmp = TempDir::new().unwrap();
        let workspace = Workspace::new(tmp.path());
        workspace.init().await.unwrap();
        let llm = LlmClient::new(zeus_core::Provider::Ollama, "dummy".to_string()).unwrap();

        let fire_at = Utc::now() - chrono::Duration::seconds(5);

        // Two one-shot tasks both due now: one SilentLedger, one Channel.
        let config = SchedulerConfig {
            enabled: true,
            tasks: vec![
                TaskConfig {
                    name: "silent task".to_string(),
                    cron: String::new(),
                    task_type: TaskType::Shell {
                        command: "echo silent".to_string(),
                    },
                    enabled: true,
                    run_at: Some(fire_at),
                    run_once: true,
                    wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::SilentLedger,
                },
                TaskConfig {
                    name: "channel task".to_string(),
                    cron: String::new(),
                    task_type: TaskType::Shell {
                        command: "echo loud".to_string(),
                    },
                    enabled: true,
                    run_at: Some(fire_at),
                    run_once: true,
                    wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
                },
            ],
            ..Default::default()
        };
        let mut scheduler = CronScheduler::new(config);

        // Wire the gateway delivery channel.
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        scheduler.set_trigger_result_tx(tx);

        // Fire on a heartbeat tick so both due tasks execute.
        let fired = scheduler
            .tick_inner(&Config::default(), &workspace, &llm, true)
            .await
            .unwrap();

        // Both tasks executed (the returned executions are the ledger record —
        // SilentLedger still runs + returns, it just isn't delivered to the gateway).
        assert_eq!(
            fired.len(),
            2,
            "both due one-shots must execute and be recorded, regardless of delivery_mode"
        );

        // Exactly one message reached the gateway channel — the Channel task only.
        let mut delivered = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            delivered.push(msg);
        }
        assert_eq!(
            delivered.len(),
            1,
            "only the Channel task delivers to the gateway; SilentLedger is suppressed"
        );
        assert!(
            delivered[0].contains("channel task"),
            "the delivered message must be the Channel task, not the SilentLedger one"
        );
        assert!(
            !delivered[0].contains("silent task"),
            "the SilentLedger task must not be delivered to the gateway"
        );
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
            run_once: false,
            wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
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
            run_once: false,
            wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
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
                    run_at: None,
                    run_once: false,
                    wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
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

        let executions = scheduler.tick(&Config::default(), &workspace, &llm).await.unwrap();
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
            run_once: false,
            wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::new(tmp.path());
        workspace.init().await.unwrap();
        let llm = LlmClient::new(zeus_core::Provider::Ollama, "dummy".to_string()).unwrap();

        let execution = execute_task_cancellable(&Config::default(), &workspace, &llm, &task, &cancel).await;
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
            run_once: false,
            wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::new(tmp.path());
        workspace.init().await.unwrap();
        let llm = LlmClient::new(zeus_core::Provider::Ollama, "dummy".to_string()).unwrap();

        let execution = execute_task_cancellable(&Config::default(), &workspace, &llm, &task, &cancel).await;
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
                run_at: None,
                run_once: false,
                wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
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

        let results = sched.tick(&Config::default(), &workspace, &llm).await.unwrap();
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

    /// #173 e2e: a scheduled `LlmPrompt` task fires AND dispatches into the
    /// tool-capable Agent path (not the old toolless `llm.complete`, not an
    /// "unknown task type" drop).
    ///
    /// We can't run a real model in a unit test (no network), so we point the
    /// LLM at an unreachable Ollama and assert the deterministic failure mode
    /// proves the wiring: the task is recognized + dispatched, `execute_llm_prompt`
    /// builds an `Agent`, `Agent::run` is invoked, and it fails trying to reach
    /// the model — surfacing as `success: false` with an agent/LLM-path error
    /// (NOT false-green). If the row were dropped or mis-dispatched, `results`
    /// would be empty or the task would never be attempted.
    #[tokio::test]
    async fn test_scheduled_llm_prompt_dispatches_agent_turn() {
        let config = SchedulerConfig {
            enabled: true,
            tasks: vec![TaskConfig {
                name: "scheduled_agent_turn".to_string(),
                cron: "* * * * *".to_string(),
                task_type: TaskType::LlmPrompt {
                    prompt: "post a status update".to_string(),
                },
                enabled: true,
                run_at: None,
                run_once: false,
                wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
            }],
            ..Default::default()
        };

        let scheduler = CronScheduler::new(config);

        // Force next_run into the past so tick() picks it up.
        {
            let mut tasks = scheduler.tasks.write().await;
            tasks[0].next_run = Some(Utc::now() - chrono::Duration::seconds(60));
        }

        let tmp = TempDir::new().unwrap();
        let workspace = Workspace::new(tmp.path());
        workspace.init().await.unwrap();
        // Unreachable Ollama — the Agent will fail at the model call, which is
        // exactly the deterministic signal we assert on.
        let llm = LlmClient::new(zeus_core::Provider::Ollama, "dummy".to_string()).unwrap();

        let results = scheduler
            .tick(&Config::default(), &workspace, &llm)
            .await
            .unwrap();

        // The task WAS dispatched (not dropped) — exactly one execution.
        assert_eq!(
            results.len(),
            1,
            "scheduled LlmPrompt must be dispatched, got {} executions",
            results.len()
        );
        let exec = &results[0];
        assert_eq!(exec.task_name, "scheduled_agent_turn");
        // No false-green: an unreachable model must surface as failure.
        assert!(
            !exec.success,
            "unreachable model must fail (no false-green), got success with output: {}",
            exec.output
        );
        // The failure came from the Agent/LLM path — proving we routed into the
        // tool-capable Agent turn, not a silent no-op or unknown-task drop.
        let out = exec.output.to_lowercase();
        assert!(
            out.contains("agent run error")
                || out.contains("error")
                || out.contains("timed out"),
            "failure should come from the agent turn path, got: {}",
            exec.output
        );
    }

    /// Full-loop e2e: the real agent-facing `schedule_create` tool writes a
    /// `task_type="prompt"` row, and the prometheus `SchedulerDb` loads it back
    /// as a `TaskType::LlmPrompt`. This exercises the genuine
    /// agent-tool → SQLite → executor-load seam that the executor-only tests
    /// (e.g. `test_*_llm_prompt_*`) skip — they start from an in-memory
    /// `ScheduledTask` and never touch the talos writer or the SQL round-trip.
    ///
    /// Uses the `ZEUS_SCHEDULER_DB` env seam so the real
    /// `ScheduleCreateTool.execute()` writes a throwaway temp DB instead of the
    /// operator's live `~/.zeus/scheduler.db`.
    #[tokio::test]
    async fn test_schedule_create_prompt_roundtrip_e2e() {
        use zeus_talos::TalosTool;
        use zeus_talos::scheduler_tools::ScheduleCreateTool;

        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("scheduler.db");

        // Point the real talos writer at the throwaway DB.
        // SAFETY: single-threaded test scope; restored at end.
        unsafe {
            std::env::set_var("ZEUS_SCHEDULER_DB", &db_path);
        }

        let prompt_payload = "post the daily standup to #devs";

        // 1) Real agent-facing tool writes the row.
        let tool = ScheduleCreateTool;
        let result = tool
            .execute(serde_json::json!({
                "name": "daily-standup",
                "schedule": "daily at 9am",
                "task_type": "prompt",
                "payload": prompt_payload,
            }))
            .await
            .expect("schedule_create(task_type=prompt) must succeed");
        assert!(
            result.contains("daily-standup"),
            "tool should confirm the created task, got: {result}"
        );

        // 2) Prometheus store loads it back from the same DB.
        let store = SchedulerDb::new(&db_path);
        let tasks = store.load_tasks().expect("load_tasks must succeed");

        // 3) The agent-tool → DB → load seam produced exactly the
        //    LlmPrompt variant with the original payload.
        assert_eq!(tasks.len(), 1, "exactly one task should be persisted");
        let task = &tasks[0];
        assert_eq!(task.name, "daily-standup");
        match &task.task_type {
            TaskType::LlmPrompt { prompt } => {
                assert_eq!(
                    prompt, prompt_payload,
                    "round-tripped prompt must match what the agent scheduled"
                );
            }
            other => panic!(
                "expected TaskType::LlmPrompt from task_type=\"prompt\", got {other:?}"
            ),
        }

        // Restore env so we don't leak the override into other tests.
        unsafe {
            std::env::remove_var("ZEUS_SCHEDULER_DB");
        }
    }

    /// #187 e2e — the boundary the existing roundtrip misses. Write rows into a
    /// scheduler.db at the raw DB level (the talos-write side), then construct a
    /// FRESH `CronScheduler::with_db` on the SAME db (the prometheus-load side)
    /// and assert the backlog reconcile + that a fresh task actually FIRES.
    ///
    /// Four assertions:
    ///  (a) migrate+load: surviving rows are loaded into the in-memory vec.
    ///  (b) stale one-shot (overdue > STALE_GRACE) is DROPPED (not loaded, row deleted).
    ///  (c) overdue recurring is RE-ANCHORED to a FUTURE next_run (not replayed).
    ///  (d) a fresh due task FIRES through a live tick().
    #[tokio::test]
    async fn test_187_backlog_reconcile_and_fire_across_load_boundary() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("scheduler.db");

        // --- talos-write side: seed rows at the raw DB level ---
        let writer = SchedulerDb::new(&db_path);
        writer.init().unwrap();

        // (b) a stale one-shot, overdue well beyond the grace window → must drop.
        let stale_one_shot = ScheduledTask {
            id: "stale-oneshot".to_string(),
            name: "dead poll".to_string(),
            cron_expr: "* * * * *".to_string(),
            task_type: TaskType::Shell { command: "echo stale".to_string() },
            enabled: true,
            last_run: None,
            next_run: Some(Utc::now() - chrono::Duration::hours(48)),
            run_once: true,
            wake_mode: WakeMode::Now,
            delivery_mode: DeliveryMode::Channel,
        };

        // (c) an overdue recurring → must re-anchor to a future slot, not replay.
        let overdue_recurring = ScheduledTask {
            id: "overdue-recurring".to_string(),
            name: "daily-ish".to_string(),
            cron_expr: "*/5 * * * *".to_string(),
            task_type: TaskType::Shell { command: "echo recur".to_string() },
            enabled: true,
            last_run: None,
            next_run: Some(Utc::now() - chrono::Duration::hours(48)),
            run_once: false,
            wake_mode: WakeMode::Now,
            delivery_mode: DeliveryMode::Channel,
        };

        // (d) a fresh task → must survive load and FIRE when due. NOTE: any row
        // seeded overdue is (correctly) reconciled — one-shots drop, recurring
        // re-anchor to the future — so to prove "fires" we seed it NOT-overdue
        // (future next_run, survives load untouched) then force it due in-memory
        // AFTER load, exactly as the trigger-channel test does. This also proves
        // the reconcile loaded it intact.
        let fresh_due = ScheduledTask {
            id: "fresh-due".to_string(),
            name: "fires now".to_string(),
            cron_expr: "* * * * *".to_string(),
            task_type: TaskType::Shell { command: "echo fired_187".to_string() },
            enabled: true,
            last_run: None,
            next_run: Some(Utc::now() + chrono::Duration::hours(1)),
            run_once: false,
            wake_mode: WakeMode::Now,
            delivery_mode: DeliveryMode::Channel,
        };

        writer.save_task(&stale_one_shot).unwrap();
        writer.save_task(&overdue_recurring).unwrap();
        writer.save_task(&fresh_due).unwrap();
        assert_eq!(writer.load_tasks().unwrap().len(), 3, "3 rows seeded");

        // --- prometheus-load side: fresh scheduler on the SAME db ---
        let scheduler = CronScheduler::new(SchedulerConfig {
            enabled: true,
            tasks: vec![],
            ..Default::default()
        })
        .with_db(&db_path)
        .unwrap();

        let loaded = scheduler.list_tasks().await;

        // (b) stale one-shot dropped — not in vec, and deleted from the db.
        assert!(
            !loaded.iter().any(|t| t.id == "stale-oneshot"),
            "stale one-shot must be dropped on load, got: {:?}",
            loaded.iter().map(|t| &t.id).collect::<Vec<_>>()
        );
        let on_disk: Vec<String> =
            writer.load_tasks().unwrap().into_iter().map(|t| t.id).collect();
        assert!(
            !on_disk.contains(&"stale-oneshot".to_string()),
            "stale one-shot row must be deleted from db, on_disk={:?}",
            on_disk
        );

        // (a) surviving rows loaded.
        assert!(loaded.iter().any(|t| t.id == "overdue-recurring"), "(a) recurring loaded");
        assert!(loaded.iter().any(|t| t.id == "fresh-due"), "(a) fresh task loaded");

        // (c) overdue recurring re-anchored to a FUTURE next_run, not replayed.
        let recur = loaded.iter().find(|t| t.id == "overdue-recurring").unwrap();
        let nr = recur.next_run.expect("re-anchored task has a next_run");
        assert!(
            nr > Utc::now(),
            "(c) overdue recurring must re-anchor to a FUTURE slot, got {} (now {})",
            nr.to_rfc3339(),
            Utc::now().to_rfc3339()
        );

        // Force ONLY the fresh task due in-memory (post-load), leaving the
        // re-anchored recurring in the future. Proves (d) fire + that the
        // reconciled recurring does NOT replay.
        {
            let mut tasks = scheduler.tasks.write().await;
            for t in tasks.iter_mut() {
                if t.id == "fresh-due" {
                    t.next_run = Some(Utc::now() - chrono::Duration::seconds(60));
                }
            }
        }

        // (d) the fresh task fires through a live tick — and the re-anchored
        // recurring does NOT replay (its next_run is future, so it's not due).
        let workspace = Workspace::new(tmp.path().to_path_buf());
        workspace.init().await.unwrap();
        let llm = LlmClient::new(zeus_core::Provider::Ollama, "dummy".to_string()).unwrap();
        let execs = scheduler.tick(&Config::default(), &workspace, &llm).await.unwrap();

        assert_eq!(
            execs.len(),
            1,
            "(d) exactly the fresh task fires; re-anchored recurring must NOT replay. fired: {:?}",
            execs.iter().map(|e| &e.task_name).collect::<Vec<_>>()
        );
        assert!(execs[0].output.contains("fired_187"), "(d) fresh task output: {}", execs[0].output);
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
            run_at: None,
            run_once: false,
            wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
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
            run_at: None,
            run_once: false,
            wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
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
            run_at: None,
            run_once: false,
            wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
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
            run_at: None,
            run_once: false,
            wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
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
            run_at: None,
            run_once: false,
            wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
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
            run_at: None,
            run_once: false,
            wake_mode: WakeMode::Now,
                    delivery_mode: DeliveryMode::Channel,
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
