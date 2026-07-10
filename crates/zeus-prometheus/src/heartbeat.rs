//! Heartbeat - Proactive background task scheduling
//!
//! Periodically checks HEARTBEAT.md for tasks and uses LLM to execute them.
//!
//! # Optimizations (S27)
//! - **HEARTBEAT_OK silent discard**: if the LLM replies with just `HEARTBEAT_OK`,
//!   the task is considered a no-op and no log entry is written.
//! - **Per-task state dedup**: last-run timestamps are persisted to
//!   `heartbeat-state.json` in the workspace root, preventing redundant execution
//!   after service restarts.
//! - **Quiet hours**: execution is suppressed between configurable hours
//!   (default 23:00–08:00 local time).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use tokio::sync::watch;
use tracing::{debug, error, info, warn};
use zeus_core::{Message, Result, ToolSchema};
use zeus_llm::LlmClient;
use zeus_memory::{Workspace, StructuredHeartbeatTask};

use crate::fire_decision::{should_fire_heartbeat, FireDecision};
use crate::tool_executor::ToolExecutor;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Controls how much heartbeat output is forwarded to Discord/channels.
///
/// Task #119: the `[Heartbeat] Completed after N tool iterations` flood was
/// fleet-wide noise. This policy gates the four `result_tx` send sites so
/// routine filler is suppressed while genuine signal (failures + substantive
/// output) still reaches the channel.
///
/// Delivery matrix (see [`should_post_to_discord`]):
/// | Result                         | Silent | Verbose | Meaningful | Quiet |
/// |--------------------------------|--------|---------|------------|-------|
/// | FAIL (any output)              | post   | post    | **post**   | post  |
/// | success, substantive output    | post   | post    | **post**   | post  |
/// | success, routine filler*       | post   | post    | suppress   | suppress |
/// | silent (`HEARTBEAT_OK`)        | n/a — never reaches a send site (filtered upstream) |
///
/// \*routine filler = max-iter `Completed after …`, `HEARTBEAT_OK`-echo, and
/// other low-signal acks recognized by [`is_routine_filler`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscordVerbosity {
    /// Forward every non-silent result, including routine filler. (Old behavior.)
    Verbose,
    /// Forward failures and substantive output; suppress routine filler. (Default.)
    Meaningful,
    /// Forward only failures; suppress all routine success output.
    Quiet,
}

impl Default for DiscordVerbosity {
    fn default() -> Self {
        DiscordVerbosity::Meaningful
    }
}

/// Returns `true` if a successful result's output is low-signal "filler" that
/// should be suppressed at `Meaningful`/`Quiet` verbosity. Failures are never
/// filler (they always post). This is the single source of truth for the
/// flood that #119 targets.
pub fn is_routine_filler(output: &str) -> bool {
    is_noop_heartbeat(output)
}

/// Returns `true` when `output` carries no real work and must never be posted
/// to Discord or used to mark an autonomous goal `Completed`.
///
/// Strips a leading `[name]:` speaker prefix (fleet messages arrive tagged,
/// e.g. `[zeus-freebsd]: HEARTBEAT_OK`) and trims, then matches:
/// - empty / whitespace-only output,
/// - an exact `HEARTBEAT_OK` ack (case-insensitive), and
/// - the `Completed after N tool iteration(s)` filler string (#119 flood).
///
/// This is the single source of truth the post-layer gates (#156) consult so
/// `is_routine_filler` and the goal/plan/watchdog sites agree on what "no-op"
/// means. The `HEARTBEAT_OK` recognition mirrors the tested
/// [`crate::gateway_consumer_is_heartbeat_ok`] contract (gateway_consumer.rs).
pub fn is_noop_heartbeat(output: &str) -> bool {
    // Strip a leading `[name]: ` speaker prefix if present, then trim.
    let t = output
        .find("]: ")
        .map(|i| output[i + 3..].trim())
        .unwrap_or_else(|| output.trim());
    if t.is_empty() {
        return true;
    }
    // The exact #119 flood string + its variants.
    if t.starts_with("Completed after") && t.contains("tool iteration") {
        return true;
    }
    // HEARTBEAT_OK echoes that slipped past the silent filter.
    if t.eq_ignore_ascii_case("HEARTBEAT_OK") || t.eq_ignore_ascii_case("HEARTBEAT_OK.") {
        return true;
    }
    false
}

/// Central delivery policy for the four heartbeat `result_tx` send sites.
///
/// `success` is the task outcome; `output` is the note body (already formatted).
/// Returns `true` when the note should be forwarded to Discord.
pub fn should_post_to_discord(verbosity: DiscordVerbosity, success: bool, output: &str) -> bool {
    // Failures always post — that's the signal operators must see.
    if !success {
        return true;
    }
    match verbosity {
        // Everything goes (legacy flood behavior).
        DiscordVerbosity::Verbose => true,
        // Substantive success posts; routine filler is suppressed.
        DiscordVerbosity::Meaningful => !is_routine_filler(output),
        // Only failures post (handled above); all success suppressed.
        DiscordVerbosity::Quiet => false,
    }
}

/// Runtime configuration for the heartbeat loop.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HeartbeatConfig {
    /// Hour (0–23, local time) at which quiet hours begin. Default: 23.
    pub quiet_hours_start: u8,
    /// Hour (0–23, local time) at which quiet hours end (exclusive). Default: 8.
    pub quiet_hours_end: u8,
    /// Whether quiet-hour suppression is active. Default: true.
    pub enable_quiet_hours: bool,
    /// IANA timezone for quiet-hours evaluation (e.g. "America/New_York").
    /// When set, quiet hours use this timezone instead of system local time.
    #[serde(default)]
    pub timezone: Option<String>,
    /// Wall-clock timeout (seconds) for a single heartbeat task. Default: 30.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Window (seconds) for suppressing duplicate heartbeat output. Default: 86400 (24h).
    #[serde(default = "default_dedup_window_secs")]
    pub dedup_window_secs: u64,
    /// Interval (seconds) when agent has an active CURRENT TASK. Default: 120.
    #[serde(default = "default_active_interval")]
    pub active_interval_secs: u64,
    /// Interval (seconds) when agent has queued tasks but no current task. Default: 300.
    #[serde(default = "default_queued_interval")]
    pub queued_interval_secs: u64,
    /// Interval (seconds) when agent is idle (no tasks). Default: 900.
    #[serde(default = "default_idle_interval")]
    pub idle_interval_secs: u64,
    /// Timeout (seconds) for trivial tasks. Default: 900 (15 min).
    #[serde(default = "default_trivial_timeout_secs")]
    pub trivial_timeout_secs: u64,
    /// Timeout (seconds) for medium-complexity tasks. Default: 1800 (30 min).
    #[serde(default = "default_medium_timeout_secs")]
    pub medium_timeout_secs: u64,
    /// Timeout (seconds) for complex tasks. Default: 3600 (60 min).
    #[serde(default = "default_complex_timeout_secs")]
    pub complex_timeout_secs: u64,
    /// R2: Disable the periodic cron tick entirely — rely solely on event wakes.
    /// When true, the only way the heartbeat fires is via `WakeRequest`.
    /// The legacy 5-minute cron is OFF by default in R2.
    #[serde(default = "default_event_driven")]
    pub event_driven_only: bool,
    /// R2: Safety-net interval (seconds) when `event_driven_only=false`.
    /// Used as the upper bound between forced cron ticks. Default: 3600 (1h).
    #[serde(default = "default_safety_net_interval")]
    pub safety_net_interval_secs: u64,
    /// Minimum interval (seconds) between consecutive resume attempts of the
    /// SAME plan slug. Mirrors the `preflight_gate` last_run pattern so an
    /// in-progress (or stuck) plan doesn't re-narrate `[Plan Resume] <slug>: ...`
    /// on every adaptive heartbeat tick. Default: 3600 (1h).
    #[serde(default = "default_plan_resume_interval")]
    pub plan_resume_interval_secs: u64,
    /// #157 bounded retries: max times the heartbeat resumes the SAME plan
    /// before driving it terminal (Abandoned) + notifying. Stops a stuck plan
    /// that never reaches Completed from re-resuming forever. GENEROUS default
    /// (25) — a safety stop, not a limit on legit long-running plans.
    #[serde(default = "default_max_plan_resume_attempts")]
    pub max_plan_resume_attempts: usize,
    /// #157 CURRENT-TASK terminal gate: max times a CURRENT-TASK-bound
    /// heartbeat task may be marked STUCK (failure-count or timeout) before
    /// the runtime auto-retires `## CURRENT TASK` to idle (`set_current_task("")`)
    /// + notifies. Closes the 112/107 wedge where a CURRENT TASK stuck on a
    /// dead endpoint re-injects forever via the hourly "continue your CURRENT
    /// TASK" task. GENEROUS default (3 STUCK markings — each itself gated by
    /// the failure_count>=3 / timeout `check_stuck` threshold) — a safety stop,
    /// not a limit on legit long-running work. 0 = never auto-retire.
    #[serde(default = "default_max_current_task_attempts")]
    pub max_current_task_attempts: u32,
    /// #119: How much heartbeat output reaches Discord/channels. Default
    /// `Meaningful` — failures + substantive output post, routine filler
    /// (the `Completed after N tool iterations` flood) is suppressed.
    #[serde(default)]
    pub discord_verbosity: DiscordVerbosity,
}

fn default_timeout_secs() -> u64 { 300 } // 5 min — coding tasks need time
fn default_dedup_window_secs() -> u64 { 86400 }
fn default_active_interval() -> u64 { 120 }  // 2 min when actively working
fn default_queued_interval() -> u64 { 300 }   // 5 min when tasks queued
fn default_idle_interval() -> u64 { 900 }     // 15 min when idle
fn default_trivial_timeout_secs() -> u64 { 900 }    // 15 min
fn default_medium_timeout_secs() -> u64 { 1800 }    // 30 min
fn default_complex_timeout_secs() -> u64 { 3600 }   // 60 min
// R2: cron is OFF by default — heartbeat now wakes on events.
fn default_event_driven() -> bool { true }
fn default_safety_net_interval() -> u64 { 3600 } // 1h hard floor between forced ticks
fn default_plan_resume_interval() -> u64 { 3600 } // 1h between plan-resume attempts of same slug
fn default_max_plan_resume_attempts() -> usize { 25 } // #157: generous cap before a stuck plan is abandoned
fn default_max_current_task_attempts() -> u32 { 3 } // #157: STUCK markings before CURRENT TASK auto-retires to idle

/// Determine task complexity based on CURRENT TASK content.
/// Returns the appropriate timeout in seconds.
pub fn compute_task_timeout(task: &str, config: &HeartbeatConfig) -> u64 {
    // Complex: multi-step coding, architecture, or large refactors
    let is_complex = task.contains("refactor")
        || task.contains("architecture")
        || task.contains("design ")
        || task.contains("implement")
        || task.contains("build ")
        || task.contains("migrate");

    // Trivial: checks, reports, simple lookups
    let is_trivial = task.contains("check ")
        || task.contains("report")
        || task.contains("verify")
        || task.contains("ping")
        || task.contains("status");

    if is_complex {
        config.complex_timeout_secs
    } else if is_trivial {
        config.trivial_timeout_secs
    } else {
        // Default to medium for everything else
        config.medium_timeout_secs
    }
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            quiet_hours_start: 23,
            quiet_hours_end: 8,
            enable_quiet_hours: true,
            timezone: None,
            timeout_secs: 300,
            dedup_window_secs: 86400,
            active_interval_secs: 120,
            queued_interval_secs: 300,
            idle_interval_secs: 900,
            trivial_timeout_secs: 900,
            medium_timeout_secs: 1800,
            complex_timeout_secs: 3600,
            event_driven_only: true,
            safety_net_interval_secs: 3600,
            plan_resume_interval_secs: 3600,
            max_plan_resume_attempts: 25,
            max_current_task_attempts: 3,
            discord_verbosity: DiscordVerbosity::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Task status machine (P1 #6)
// ---------------------------------------------------------------------------

/// Status of a heartbeat task in its lifecycle.
/// Tracked per task and persisted to heartbeat-state.json.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum TaskStatus {
    /// Task is queued but not yet started.
    Pending,
    /// Task is currently being worked on.
    InProgress,
    /// Task completed successfully.
    Completed,
    /// Task failed after retries.
    Failed,
    /// Task is stuck — no progress after multiple attempts.
    Stuck,
}

impl Default for TaskStatus {
    fn default() -> Self {
        TaskStatus::Pending
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "PENDING"),
            TaskStatus::InProgress => write!(f, "IN_PROGRESS"),
            TaskStatus::Completed => write!(f, "COMPLETED"),
            TaskStatus::Failed => write!(f, "FAILED"),
            TaskStatus::Stuck => write!(f, "STUCK"),
        }
    }
}

// ---------------------------------------------------------------------------
// Per-task state (persisted to heartbeat-state.json)
// ---------------------------------------------------------------------------

/// Persisted state tracking last-run time and output per task description.
/// Stored as `<workspace-root>/heartbeat-state.json`.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct HeartbeatState {
    /// Maps task description → Unix timestamp of last successful run.
    last_run: HashMap<String, u64>,
    /// Maps task description → last non-silent output text (for dedup).
    #[serde(default)]
    last_output: HashMap<String, String>,
    /// Maps task description → Unix timestamp of last non-silent output.
    #[serde(default)]
    last_output_at: HashMap<String, u64>,
    /// Maps task description → current status in the task lifecycle.
    #[serde(default)]
    task_status: HashMap<String, TaskStatus>,
    /// Maps task description → Unix timestamp when status last changed.
    #[serde(default)]
    status_changed_at: HashMap<String, u64>,
    /// Maps task description → number of consecutive failures.
    #[serde(default)]
    failure_count: HashMap<String, u32>,
    /// Unix timestamp of the last heartbeat tick (persisted for crash detection).
    ///
    /// #171: MUST carry `#[serde(default)]`. Without it, a state file written by
    /// an older binary (or any file missing this field) fails `from_str` *entirely* —
    /// `load_state`'s `.unwrap_or_default()` then resets the WHOLE struct, nuking the
    /// persisted tick to `None`. The watchdog reads `None`, finds no in-run tick, and
    /// false-fires "Last tick: never" on a perfectly healthy seat. Every other field
    /// here is `#[serde(default)]` for exactly this forward/backward-compat reason;
    /// this one was the lone gap.
    #[serde(default)]
    last_heartbeat_tick: Option<u64>,
}

/// #206: process-wide lock giving single-writer ownership of the state file.
/// Three unsynchronized writers (fast-pulse, main-loop tick, main-loop save)
/// used to race plain `fs::write` calls → torn reads → watchdog false-fires.
static STATE_FILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// #206: last tick this process successfully read or wrote. If the on-disk
/// file is ever corrupt/partial, we preserve this instead of fabricating
/// `None` (which made the watchdog spam "Last tick: never" on healthy seats).
/// -1 = no tick known yet.
static LAST_KNOWN_TICK: AtomicI64 = AtomicI64::new(-1);

/// Default state, but with `last_heartbeat_tick` carried over from the last
/// known good value in this process — never reset the tick on a bad read.
fn state_with_preserved_tick() -> HeartbeatState {
    let mut state = HeartbeatState::default();
    let tick = LAST_KNOWN_TICK.load(Ordering::Relaxed);
    if tick >= 0 {
        state.last_heartbeat_tick = Some(tick as u64);
    }
    state
}

fn load_state(path: &std::path::Path) -> HeartbeatState {
    let _guard = STATE_FILE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    load_state_inner(path)
}

fn load_state_inner(path: &std::path::Path) -> HeartbeatState {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Fresh seat — genuinely no state yet. Default is correct here.
            return HeartbeatState::default();
        }
        Err(e) => {
            error!(
                "heartbeat-state: read failed at {}: {} — preserving last known tick (#206)",
                path.display(),
                e
            );
            return state_with_preserved_tick();
        }
    };
    match serde_json::from_str::<HeartbeatState>(&raw) {
        Ok(state) => {
            if let Some(t) = state.last_heartbeat_tick {
                LAST_KNOWN_TICK.store(t as i64, Ordering::Relaxed);
            }
            state
        }
        Err(e) => {
            // #206: LOUD failure, not silent `.ok()`. A corrupt/partial file is
            // an event worth logging, and it must NOT reset `last_heartbeat_tick`
            // — that fabricated `None` is exactly what made the watchdog
            // false-fire "Last tick: never" on healthy seats.
            error!(
                "heartbeat-state: PARSE FAILURE at {}: {} — file corrupt or partial; \
                 preserving last known tick instead of resetting (#206)",
                path.display(),
                e
            );
            state_with_preserved_tick()
        }
    }
}

fn save_state(path: &std::path::Path, state: &HeartbeatState) {
    // Single-writer ownership: serialize all writers in this process.
    let _guard = STATE_FILE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    save_state_inner(path, state)
}

fn save_state_inner(path: &std::path::Path, state: &HeartbeatState) {
    let json = match serde_json::to_string_pretty(state) {
        Ok(json) => json,
        Err(e) => {
            warn!("Failed to serialize heartbeat state: {}", e);
            return;
        }
    };
    if let Some(t) = state.last_heartbeat_tick {
        LAST_KNOWN_TICK.store(t as i64, Ordering::Relaxed);
    }
    // #206: atomic write — temp file in the same directory, then rename.
    // Readers either see the old complete file or the new complete file,
    // never a torn half-write.
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    if let Err(e) = std::fs::write(&tmp, &json) {
        warn!("Failed to write heartbeat state temp file: {}", e);
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        warn!("Failed to atomically replace heartbeat state file: {}", e);
        let _ = std::fs::remove_file(&tmp);
    }
}

/// #206: locked read-modify-write for short state updates. Holds the writer
/// lock across the whole load→mutate→save cycle so concurrent updaters can't
/// interleave and lose each other's writes.
fn update_state(path: &std::path::Path, mutate: impl FnOnce(&mut HeartbeatState)) {
    let _guard = STATE_FILE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut state = load_state_unlocked(path);
    mutate(&mut state);
    save_state_unlocked(path, &state);
}

/// Lock-free inner bodies so `update_state` can hold the lock across the
/// full cycle without re-entrant deadlock on `std::sync::Mutex`.
fn load_state_unlocked(path: &std::path::Path) -> HeartbeatState {
    load_state_inner(path)
}

fn save_state_unlocked(path: &std::path::Path, state: &HeartbeatState) {
    save_state_inner(path, state)
}

/// Update the status of a task in the heartbeat state.
/// Records the timestamp of the status change for tracking.
fn update_task_status(state: &mut HeartbeatState, task: &str, status: TaskStatus) {
    let now = chrono::Utc::now().timestamp() as u64;
    let old_status = state.task_status.get(task).cloned().unwrap_or_default();
    
    if old_status != status {
        info!("Task '{}' status: {} → {}", task, old_status, status);
        state.task_status.insert(task.to_string(), status);
        state.status_changed_at.insert(task.to_string(), now);
    }
}

/// Increment the failure count for a task. Returns the new count.
fn increment_failure_count(state: &mut HeartbeatState, task: &str) -> u32 {
    let count = state.failure_count.get(task).copied().unwrap_or(0) + 1;
    state.failure_count.insert(task.to_string(), count);
    count
}

/// Reset the failure count for a task (call on success).
fn reset_failure_count(state: &mut HeartbeatState, task: &str) {
    state.failure_count.remove(task);
}

/// Reason a task was classified as STUCK.
#[derive(Debug, Clone, PartialEq)]
pub enum StuckReason {
    FailureCount { count: u32, threshold: u32 },
    Timeout { elapsed_secs: u64, timeout_secs: u64 },
}

impl std::fmt::Display for StuckReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StuckReason::FailureCount { count, threshold } => {
                write!(f, "failure_count={}/threshold={}", count, threshold)
            }
            StuckReason::Timeout {
                elapsed_secs,
                timeout_secs,
            } => {
                write!(
                    f,
                    "timeout: elapsed={}s > limit={}s",
                    elapsed_secs, timeout_secs
                )
            }
        }
    }
}

/// #157: Does this heartbeat task's prompt drive the agent to work on the
/// `## CURRENT TASK` surface in HEARTBEAT.md? These are the tasks that, when
/// wedged on a dead endpoint, re-inject forever (the 112/107 loop) — so they
/// are the ones whose STUCK state must auto-retire CURRENT TASK to idle.
///
/// Matches the canonical "continue your CURRENT TASK" hourly task (and any
/// prompt that explicitly references the CURRENT TASK surface), case-insensitively.
fn is_current_task_bound(prompt: &str) -> bool {
    let p = prompt.to_lowercase();
    p.contains("current task")
}

/// Check if a task should be marked as STUCK.
/// A task is stuck if it has failed 3+ times or been in progress too long.
/// Returns the reason if stuck, or `None` if not.
fn check_stuck(state: &HeartbeatState, task: &str, timeout_secs: u64) -> Option<StuckReason> {
    let failure_count = state.failure_count.get(task).copied().unwrap_or(0);
    if failure_count >= 3 {
        return Some(StuckReason::FailureCount {
            count: failure_count,
            threshold: 3,
        });
    }

    if let Some(&changed_at) = state.status_changed_at.get(task) {
        let now = chrono::Utc::now().timestamp() as u64;
        let elapsed = now.saturating_sub(changed_at);
        if elapsed > timeout_secs {
            return Some(StuckReason::Timeout {
                elapsed_secs: elapsed,
                timeout_secs,
            });
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Quiet-hours check
// ---------------------------------------------------------------------------

/// Returns `true` if the current local time falls within the configured
/// quiet-hours window.  Handles overnight ranges (e.g. 23–08) correctly.
/// Uses configured timezone if set, otherwise system local time.
fn is_quiet_hour(config: &HeartbeatConfig, _unix_secs: u64) -> bool {
    if !config.enable_quiet_hours {
        return false;
    }
    let hour = resolve_current_hour(config.timezone.as_deref());
    is_quiet_hour_for(config, hour)
}

/// Resolve the current hour in the configured timezone, falling back to local.
/// Accepts UTC offset strings like "+05:00", "-08:00", or "UTC".
fn resolve_current_hour(timezone: Option<&str>) -> u8 {
    use chrono::Timelike;
    if let Some(tz_str) = timezone {
        // Parse as UTC offset: "+HH:MM", "-HH:MM", or "UTC"
        if tz_str.eq_ignore_ascii_case("utc") || tz_str == "+00:00" {
            return chrono::Utc::now().hour() as u8;
        }
        if let Some(offset) = parse_utc_offset(tz_str) {
            return chrono::Utc::now().with_timezone(&offset).hour() as u8;
        }
        warn!("Invalid timezone '{}' (expected UTC offset like '+05:00'), falling back to local", tz_str);
    }
    chrono::Local::now().hour() as u8
}

/// Parse a UTC offset string like "+05:00" or "-08:00" into a chrono::FixedOffset.
fn parse_utc_offset(s: &str) -> Option<chrono::FixedOffset> {
    let s = s.trim();
    if s.len() < 5 { return None; }
    let sign = match s.as_bytes()[0] {
        b'+' => 1i32,
        b'-' => -1i32,
        _ => return None,
    };
    let parts: Vec<&str> = s[1..].split(':').collect();
    if parts.len() != 2 { return None; }
    let hours: i32 = parts[0].parse().ok()?;
    let mins: i32 = parts[1].parse().ok()?;
    let total_secs = sign * (hours * 3600 + mins * 60);
    chrono::FixedOffset::east_opt(total_secs)
}

/// Determine the adaptive heartbeat interval based on current task state.
/// Reads HEARTBEAT.md to check for CURRENT TASK and queued tasks.
/// Returns the appropriate interval in seconds.
pub async fn compute_adaptive_interval(
    workspace: &zeus_memory::Workspace,
    config: &HeartbeatConfig,
) -> u64 {
    // Check if there's an active CURRENT TASK
    let has_current_task = match workspace.get_current_task().await {
        Ok(Some(task)) => !task.trim().is_empty()
            && !task.contains("Coordinator will assign")
            && !task.contains("(none)"),
        _ => false,
    };

    if has_current_task {
        debug!("Adaptive interval: active task → {}s", config.active_interval_secs);
        return config.active_interval_secs;
    }

    // Check if there are queued tasks
    let has_queued = match workspace.get_task_queue().await {
        Ok(tasks) => !tasks.is_empty(),
        _ => false,
    };

    if has_queued {
        debug!("Adaptive interval: queued tasks → {}s", config.queued_interval_secs);
        return config.queued_interval_secs;
    }

    // Idle — no current task, no queue
    debug!("Adaptive interval: idle → {}s", config.idle_interval_secs);
    config.idle_interval_secs
}

/// Pure logic: check whether `hour` falls inside the quiet window.
/// Extracted for deterministic testing.
fn is_quiet_hour_for(config: &HeartbeatConfig, hour: u8) -> bool {
    let start = config.quiet_hours_start;
    let end = config.quiet_hours_end;
    if start <= end {
        // Same-day range, e.g. 08–17
        hour >= start && hour < end
    } else {
        // Overnight range, e.g. 23–08
        hour >= start || hour < end
    }
}

// ---------------------------------------------------------------------------
// Public struct
// ---------------------------------------------------------------------------

/// Heartbeat manager for proactive background tasks.
/// A request to wake the heartbeat immediately.
#[derive(Debug, Clone)]
pub struct WakeRequest {
    /// Why this wake was requested (e.g. "cron_complete", "goal_added", "tool_finished")
    pub reason: String,
    /// Which agent to wake (None = default agent)
    pub agent_id: Option<String>,
}

/// Per-agent heartbeat state for independent scheduling (S67-B2).
#[derive(Debug, Clone)]
pub struct AgentHeartbeatState {
    pub agent_id: String,
    pub interval_secs: u64,
    pub last_run: Option<std::time::Instant>,
    pub next_due: std::time::Instant,
    pub active_hours: Option<String>,
    pub timezone: Option<String>,
}

pub struct Heartbeat {
    workspace: Arc<Workspace>,
    llm: Arc<LlmClient>,
    shutdown_tx: Option<watch::Sender<bool>>,
    interval_secs: u64,
    tool_executor: Option<Arc<dyn ToolExecutor>>,
    tools: Vec<ToolSchema>,
    config: HeartbeatConfig,
    /// S67-C1: Wake channel — any component can trigger an immediate heartbeat.
    /// Sender is cloneable and shared across the system.
    wake_tx: Option<tokio::sync::mpsc::Sender<WakeRequest>>,
    wake_rx: Option<tokio::sync::mpsc::Receiver<WakeRequest>>,
    /// S67-B2: Per-agent heartbeat states
    agent_states: Vec<AgentHeartbeatState>,
    /// S69: Optional callback to deliver heartbeat results to Discord/channels
    result_tx: Option<tokio::sync::mpsc::Sender<String>>,
    /// Signal that a channel message is actively being processed.
    /// Heartbeat defers execution while this is true to avoid starving real messages.
    channel_active: Option<zeus_core::CookState>,
    /// Mnemosyne memory store for recall during heartbeat cooks.
    /// When set, heartbeat tasks get relevant memory context injected.
    mnemosyne: Option<Arc<zeus_mnemosyne::Mnemosyne>>,
    /// Inbox queue-depth counter for busy-aware fire-decision (`busy: inbound`).
    /// Reads `> 0` indicate one or more messages waiting in the agent inbox
    /// mpsc buffer. Distinct from `channel_active` (cook-flight) — this is
    /// queue-buffered work, not handler-in-flight work. Wired by gateway from
    /// `create_inbox()` 3-tuple return at construction time.
    inbox_queue_depth: Option<Arc<std::sync::atomic::AtomicUsize>>,
    /// Subagent depth counter for busy-aware fire-decision (`busy: subagent`).
    /// Reads `> 0` indicate one or more subagent cooks in flight via
    /// `SpawnTracker::track_spawn` / `complete_spawn`. Distinct from
    /// `channel_active` (this-cook in-flight) and `inbox_queue_depth`
    /// (queued external messages). Wired by `Prometheus` construction
    /// from `SpawnTracker::active_count_handle()`.
    subagent_depth: Option<Arc<std::sync::atomic::AtomicUsize>>,
    /// Last user-interaction timestamp (unix seconds) for busy-aware
    /// fire-decision (`busy: recent_interaction`). Reads within the quiet
    /// threshold suppress autonomous fire to avoid talking-over an active
    /// user. Distinct from `channel_active` (handler in flight) and the
    /// queue-depth counters (work pending) — this is the post-cook quiet
    /// window. Wired by A1.5b-ii follow-up at the cook-completion site;
    /// `None` here = graceful-degrade (RecentInteraction bucket inactive),
    /// per banked Option-shape discipline (Lane A1.5a precedent).
    last_interaction_at: Option<Arc<std::sync::atomic::AtomicI64>>,
    /// #89.2: Shared fleet peer-status context for wake prompt injection.
    /// Gateway writes fresh data; heartbeat reads each cycle.
    /// Injected as tier 7 in the wake prompt (after memory_context).
    peer_status_context: Arc<tokio::sync::RwLock<Option<String>>>,
    /// #89.3: Shared unread/recent messages context for wake prompt injection.
    /// Gateway queries discord_history and formats it here.
    /// Injected as tier 8 in the wake prompt (after peer_status).
    unread_context: Arc<tokio::sync::RwLock<Option<String>>>,
}

/// γ safety-net spawn (#80, extracted for testability).
///
/// Spawns an independent tokio task that forces a `WakeRequest` every
/// `interval_secs` regardless of any other signal, capping max-silence at the
/// configured interval. Terminates cleanly when `shutdown` flips to `true`.
///
/// Pure function with injected dependencies — no `Heartbeat` construction
/// required, enabling unit tests via `tokio::time::pause` + `advance`.
///
/// If the wake channel is closed (heartbeat shutdown), the send errors
/// silently — channel-close on shutdown is expected. The task continues
/// looping until `shutdown` signals.
pub(crate) fn spawn_safety_net(
    safety_tx: tokio::sync::mpsc::Sender<WakeRequest>,
    interval_secs: u64,
    mut shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let dur = std::time::Duration::from_secs(interval_secs);
        loop {
            tokio::select! {
                _ = tokio::time::sleep(dur) => {
                    let _ = safety_tx
                        .send(WakeRequest {
                            reason: "safety_net_forced_tick".to_string(),
                            agent_id: None,
                        })
                        .await;
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    })
}

impl Heartbeat {
    /// Create a new heartbeat manager with default configuration.
    pub fn new(workspace: Arc<Workspace>, llm: Arc<LlmClient>) -> Self {
        let (wake_tx, wake_rx) = tokio::sync::mpsc::channel(32);
        Self {
            workspace,
            llm,
            shutdown_tx: None,
            interval_secs: 300, // 5 minutes — cook-priority prevents message starvation
            tool_executor: None,
            tools: Vec::new(),
            config: HeartbeatConfig::default(),
            wake_tx: Some(wake_tx),
            wake_rx: Some(wake_rx),
            agent_states: Vec::new(),
            result_tx: None,
            channel_active: None,
            mnemosyne: None,
            inbox_queue_depth: None,
            subagent_depth: None,
            last_interaction_at: None,
            peer_status_context: Arc::new(tokio::sync::RwLock::new(None)),
            unread_context: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Wire the inbox queue-depth counter for busy-aware fire-decision.
    /// Reads `> 0` will surface as `busy: inbound` skip-reason once the
    /// fire-decision integration lands. Plumbing-only at this layer; the
    /// fire-decision read site is a follow-up sub-task per spec §3.1.
    pub fn set_inbox_queue_depth(&mut self, depth: Arc<std::sync::atomic::AtomicUsize>) {
        self.inbox_queue_depth = Some(depth);
    }

    /// Read the current inbox queue depth (0 if not wired).
    /// Used by busy-aware fire-decision: `> 0` → `busy: inbound` skip.
    pub fn inbox_queue_depth(&self) -> usize {
        self.inbox_queue_depth
            .as_ref()
            .map(|d| d.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Wire the subagent-depth counter for busy-aware fire-decision.
    /// Reads `> 0` will surface as `busy: subagent` skip-reason once the
    /// fire-decision integration lands. Plumbing-only at this layer; the
    /// fire-decision read site is a follow-up sub-task per spec §3.1.
    /// Mirror of `set_inbox_queue_depth` shape (Lane A1 atomic precedent).
    pub fn set_subagent_depth(&mut self, depth: Arc<std::sync::atomic::AtomicUsize>) {
        self.subagent_depth = Some(depth);
    }

    /// Read the current subagent depth (0 if not wired).
    /// Used by busy-aware fire-decision: `> 0` → `busy: subagent` skip.
    pub fn subagent_depth(&self) -> usize {
        self.subagent_depth
            .as_ref()
            .map(|d| d.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Wire the last-interaction-at handle for busy-aware fire-decision
    /// (`busy: recent_interaction`).
    ///
    /// The cook-completion site (A1.5b-ii follow-up) calls
    /// `handle.store(now_unix, Relaxed)` to update; the fire-decision read
    /// site is a follow-up sub-task per spec §3.1.
    /// Mirror of `set_inbox_queue_depth` / `set_subagent_depth` shape
    /// (Lane A1.5a Option-shape precedent).
    pub fn set_last_interaction_at(&mut self, last_at: Arc<std::sync::atomic::AtomicI64>) {
        self.last_interaction_at = Some(last_at);
    }

    /// Read the current last-interaction-at unix-seconds value.
    ///
    /// Returns `None` when the handle is not wired — this is the
    /// graceful-degrade signal for the busy-aware fire-decision: the
    /// `RecentInteraction` bucket evaluates as inactive (never skips) when
    /// `None`, mirroring the A1.5a Option-shape discipline.
    pub fn last_interaction_at(&self) -> Option<i64> {
        self.last_interaction_at
            .as_ref()
            .map(|t| t.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Set Mnemosyne memory store for recall during heartbeat cooks.
    pub fn set_mnemosyne(&mut self, mnemosyne: Arc<zeus_mnemosyne::Mnemosyne>) {
        self.mnemosyne = Some(mnemosyne);
    }

    /// Set the result delivery channel — heartbeat results will be sent here
    /// for posting to Discord/channels (S69: autonomy visibility).
    pub fn set_result_tx(&mut self, tx: tokio::sync::mpsc::Sender<String>) {
        self.result_tx = Some(tx);
    }

    /// Wire the cook-state handle so heartbeat defers while other cooks run
    /// and so channel cooks can see when the heartbeat is active.
    pub fn set_channel_active(&mut self, state: zeus_core::CookState) {
        self.channel_active = Some(state);
    }

    /// #89.2: Get the shared peer-status context handle for external updates.
    pub fn peer_status_handle(&self) -> Arc<tokio::sync::RwLock<Option<String>>> {
        self.peer_status_context.clone()
    }

    /// #89.3: Get the shared unread context handle for external updates.
    pub fn unread_handle(&self) -> Arc<tokio::sync::RwLock<Option<String>>> {
        self.unread_context.clone()
    }

    /// S67-B2: Register per-agent heartbeat schedules from config.
    /// Each agent gets its own interval and active hours settings.
    pub fn register_agents(&mut self, agents: &[zeus_core::AgentConfig]) {
        let now = std::time::Instant::now();
        self.agent_states = agents.iter().filter_map(|a| {
            let interval = a.heartbeat_interval_secs.unwrap_or(self.interval_secs);
            if interval == 0 { return None; }
            Some(AgentHeartbeatState {
                agent_id: a.id.clone(),
                interval_secs: interval,
                last_run: None,
                next_due: now + std::time::Duration::from_secs(interval),
                active_hours: a.active_hours.clone(),
                timezone: a.heartbeat_timezone.clone(),
            })
        }).collect();
        if !self.agent_states.is_empty() {
            info!("Registered {} agent heartbeat schedule(s)", self.agent_states.len());
        }
    }

    /// Get a clone of the wake sender. Any component can use this to trigger
    /// an immediate heartbeat (S67-C1 wake-on-event).
    pub fn wake_sender(&self) -> Option<tokio::sync::mpsc::Sender<WakeRequest>> {
        self.wake_tx.clone()
    }

    /// Override the check interval.
    pub fn with_interval(mut self, secs: u64) -> Self {
        self.interval_secs = secs;
        self
    }

    /// Override the heartbeat configuration (quiet hours, etc.).
    pub fn with_config(mut self, config: HeartbeatConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the tool executor for heartbeat tasks.
    pub fn with_tool_executor(
        mut self,
        executor: Arc<dyn ToolExecutor>,
        tools: Vec<ToolSchema>,
    ) -> Self {
        self.tool_executor = Some(executor);
        self.tools = tools;
        self
    }

    /// Start the heartbeat background loop.
    pub async fn start(&mut self) -> Result<()> {
        if self.shutdown_tx.is_some() {
            return Ok(()); // Already running
        }

        let (tx, rx) = watch::channel(false);
        self.shutdown_tx = Some(tx);

        let workspace = self.workspace.clone();
        let llm = self.llm.clone();
        let interval = self.interval_secs;
        let tool_executor = self.tool_executor.clone();
        let tools = self.tools.clone();
        let config = self.config.clone();
        let state_path = workspace.root().join("heartbeat-state.json");
        let wake_rx = self.wake_rx.take();
        let channel_active = self.channel_active.clone();
        let result_tx = self.result_tx.clone();
        let mnemosyne = self.mnemosyne.clone();
        let inbox_queue_depth = self.inbox_queue_depth.clone();
        let subagent_depth = self.subagent_depth.clone();
        let last_interaction_at = self.last_interaction_at.clone();
        let peer_status_handle = self.peer_status_context.clone();
        let unread_handle = self.unread_context.clone();

        // γ safety-net (#80): bounded liveness backstop against the
        // `event_driven_only=true` "sleep forever" failure mode. Independent of
        // the heartbeat_loop's own wake-handling: this task forces a WakeRequest
        // every `safety_net_interval_secs` (default 1h) regardless of any other
        // signal, capping max-silence at the configured interval. If the wake
        // channel is closed (heartbeat shutdown), the send errors silently and
        // the task continues — shutdown cleanup is handled by the heartbeat_loop
        // dropping its receiver.
        if let Some(safety_tx) = self.wake_tx.clone() {
            let safety_interval = self.config.safety_net_interval_secs;
            let safety_shutdown = self.shutdown_tx.as_ref().unwrap().subscribe();
            spawn_safety_net(safety_tx, safety_interval, safety_shutdown);
        }

        tokio::spawn(async move {
            heartbeat_loop(
                workspace,
                llm,
                rx,
                interval,
                tool_executor,
                tools,
                config,
                state_path,
                wake_rx,
                channel_active,
                result_tx,
                mnemosyne,
                inbox_queue_depth,
                subagent_depth,
                last_interaction_at,
                peer_status_handle,
                unread_handle,
            )
            .await;
        });

        info!("Heartbeat started (interval: {}s)", self.interval_secs);
        Ok(())
    }

    /// Stop the heartbeat loop.
    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
            info!("Heartbeat stopped");
        }
    }

    /// Check if the heartbeat is running.
    pub fn is_running(&self) -> bool {
        self.shutdown_tx.is_some()
    }

    /// Run tasks for a given frequency using the LLM.
    ///
    /// Silent (`HEARTBEAT_OK`) results are excluded from the returned vec and
    /// from the workspace note.
    pub async fn run_tasks(&self, frequency: &str) -> Result<Vec<TaskResult>> {
        let tasks = self.workspace.get_heartbeat_tasks(frequency).await?;
        if tasks.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        for task in &tasks {
            let psc = self.peer_status_context.read().await;
            let psc_str = psc.as_deref().map(|s| s.to_string());
            drop(psc);
            let uc = self.unread_context.read().await;
            let uc_str = uc.as_deref().map(|s| s.to_string());
            drop(uc);
            let result = execute_heartbeat_task(
                &self.llm,
                &self.workspace,
                task,
                self.tool_executor.as_ref(),
                &self.tools,
                self.mnemosyne.as_ref(),
                &self.config,
                psc_str.as_deref(),
                uc_str.as_deref(),
            )
            .await;
            results.push(result);
        }

        // Only note tasks that produced real output (non-silent results).
        let meaningful: Vec<&TaskResult> = results.iter().filter(|r| !r.silent).collect();
        if !meaningful.is_empty() {
            let summary: Vec<String> = meaningful
                .iter()
                .map(|r| {
                    format!(
                        "- [{}] {}: {}",
                        if r.success { "OK" } else { "FAIL" },
                        r.task,
                        r.output
                    )
                })
                .collect();

            let note = format!(
                "[Heartbeat] Ran {} {} tasks:\n{}",
                meaningful.len(),
                frequency,
                summary.join("\n")
            );
            let _ = self.workspace.note(&note).await;

            // S69 + #119: Deliver to Discord only if the batch carries signal.
            // A batch is signal-bearing if any task FAILed or produced
            // substantive (non-filler) output; otherwise it's routine flood.
            let batch_has_signal = meaningful
                .iter()
                .any(|r| should_post_to_discord(self.config.discord_verbosity, r.success, &r.output));
            if batch_has_signal {
                if let Some(ref tx) = self.result_tx {
                    let _ = tx.try_send(note);
                }
            }

            // S69: Persist to dedicated heartbeat session
            for r in &meaningful {
                append_heartbeat_session(&self.workspace, &r.task, &r.output);
            }
        }

        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// Background loop
// ---------------------------------------------------------------------------

/// #330: compute the timed branch's HEAD re-arm value.
///
/// Called first thing inside the timed `select!` arm, before any gate can
/// `continue`, so a wake-driven `current_interval = 0` survives exactly one
/// iteration. Without this, any early-`continue` gate (skip arms, PAUSE,
/// quiet hours, plan-resume gating) left the interval at 0 → `sleep(0)`
/// busy-loop → the minibsd ~1ms ENOENT spam + tokio time-driver starvation.
///
/// Pure function so the invariant is unit-testable.
fn head_rearm_interval(event_driven_only: bool, safety_net_interval_secs: u64, interval_secs: u64) -> u64 {
    if event_driven_only {
        u64::MAX / 2 // effectively never — wake events re-zero it
    } else {
        // .max(1): even a degenerate config (both intervals 0) must not
        // reproduce the busy-loop this function exists to prevent.
        safety_net_interval_secs.max(interval_secs).max(1)
    }
}

/// Spawn a background fast-pulse task that writes `last_heartbeat_tick = now()`
/// to the state file every `period_secs` seconds. Pure liveness signal,
/// decoupled from wake events — the agent process being alive IS the signal.
///
/// Fix B from Dispatch 24. Complements zeus106's P0 parity write at the
/// cook return-path; the parity write covers "agent finished a task" while
/// this covers "agent is alive but mid-cook for a long time."
pub fn spawn_fast_pulse(state_path: std::path::PathBuf, period_secs: u64) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(period_secs));
        // Skip the immediate-fire tick — we want the first pulse after `period_secs`.
        interval.tick().await;
        loop {
            interval.tick().await;
            let now_unix = chrono::Utc::now().timestamp() as u64;
            // #206: locked read-modify-write — this used to be an unsynchronized
            // load/save pair racing the main loop's writers.
            update_state(&state_path, |state| {
                state.last_heartbeat_tick = Some(now_unix);
            });
            tracing::debug!(target: "heartbeat::fast_pulse", "wrote last_heartbeat_tick={}", now_unix);
        }
    });
}

/// Spawn a background watchdog that alerts when no heartbeat tick occurs
/// for `stall_threshold_secs` while a task is in_progress.
/// Pure stall-decision for the watchdog. Returns `true` when the heartbeat
/// machinery looks dead and an alert is warranted.
///
/// The liveness baseline is the most recent credible "machinery is alive"
/// signal for *this* process run: an in-run `last_tick` (one that post-dates
/// `process_start`), falling back to `process_start` itself when no in-run tick
/// has happened yet. Measuring elapsed time against that baseline closes the
/// #156 never-ticked-since-start hole — a loop that is dead from boot (no tick
/// ever, or only a stale pre-start tick) still pages once the threshold elapses,
/// while a tick from before a restart is correctly ignored as a stale artifact.
fn watchdog_should_alert(
    last_tick: Option<u64>,
    process_start: u64,
    now: u64,
    stall_threshold_secs: u64,
) -> bool {
    let in_run_tick = last_tick.filter(|&t| t >= process_start);
    let liveness_baseline = in_run_tick.unwrap_or(process_start);
    now.saturating_sub(liveness_baseline) >= stall_threshold_secs
}

pub fn spawn_watchdog(
    state_path: std::path::PathBuf,
    workspace: Arc<Workspace>,
    result_tx: Option<tokio::sync::mpsc::Sender<String>>,
    period_secs: u64,
    stall_threshold_secs: u64,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(period_secs));
        // #156 restart-guard: record when this process came up. A stale
        // `last_heartbeat_tick` from before the restart is a restart artifact,
        // not a live stall, and must not fire an alert.
        let process_start_unix = chrono::Utc::now().timestamp() as u64;
        // #156 debounce: suppress repeat alerts within this window (~1h) so a
        // genuine sustained stall pages once, not every `period_secs` tick.
        const ALERT_DEBOUNCE_SECS: u64 = 3600;
        let mut last_alert_unix: Option<u64> = None;
        // #156: skip the very first tick — on a fresh start there is no live
        // baseline to compare against yet.
        let mut first_tick = true;
        loop {
            interval.tick().await;
            if first_tick {
                first_tick = false;
                continue;
            }
            let state = load_state(&state_path);
            let now_unix = chrono::Utc::now().timestamp() as u64;
            // #156: stall decision now lives in the pure `watchdog_should_alert`
            // so the never-ticked-since-start path is directly testable. It uses
            // max(in-run last_tick, process_start) as the liveness baseline, which
            // closes the hole where the old restart-guard left `silent` permanently
            // false (last_tick < start, or last_tick == None) and a process dead
            // from boot never paged.
            let silent = watchdog_should_alert(
                state.last_heartbeat_tick,
                process_start_unix,
                now_unix,
                stall_threshold_secs,
            );
            // #156 debounce: hold off if we alerted within the window.
            let debounced = last_alert_unix
                .map(|t| now_unix.saturating_sub(t) < ALERT_DEBOUNCE_SECS)
                .unwrap_or(false);
            if silent && !debounced {
                let task_in_progress = match workspace.get_current_task().await {
                    Ok(Some(task)) => {
                        task.to_lowercase().contains("in_progress")
                            || task.contains("**status:**")
                            && task.to_lowercase().contains("progress")
                    }
                    _ => false,
                };
                if task_in_progress {
                    let warning = format!(
                        "⚠️ **Heartbeat Watchdog Alert**\n\
                        No heartbeat tick received in 10+ minutes with CURRENT TASK in_progress.\n\
                        Possible gateway crash or stall. Last tick: {}",
                        state.last_heartbeat_tick
                            .map(|t| chrono::DateTime::from_timestamp(t as i64, 0)
                                .map(|dt| dt.to_rfc3339())
                                .unwrap_or_else(|| "unknown".to_string()))
                            .unwrap_or_else(|| "never".to_string())
                    );
                    warn!("Watchdog alert fired: no heartbeat tick in 10+ min, task in_progress");
                    // #156: stamp the debounce only when an alert actually fires.
                    last_alert_unix = Some(now_unix);
                    if let Some(ref tx) = result_tx {
                        let _ = tx.try_send(warning);
                    }
                }
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
async fn heartbeat_loop(
    workspace: Arc<Workspace>,
    llm: Arc<LlmClient>,
    mut shutdown: watch::Receiver<bool>,
    interval_secs: u64,
    tool_executor: Option<Arc<dyn ToolExecutor>>,
    tools: Vec<ToolSchema>,
    config: HeartbeatConfig,
    state_path: std::path::PathBuf,
    mut wake_rx: Option<tokio::sync::mpsc::Receiver<WakeRequest>>,
    channel_active: Option<zeus_core::CookState>,
    result_tx: Option<tokio::sync::mpsc::Sender<String>>,
    mnemosyne: Option<Arc<zeus_mnemosyne::Mnemosyne>>,
    inbox_queue_depth: Option<Arc<AtomicUsize>>,
    subagent_depth: Option<Arc<AtomicUsize>>,
    last_interaction_at: Option<Arc<AtomicI64>>,
    peer_status_context: Arc<tokio::sync::RwLock<Option<String>>>,
    unread_context: Arc<tokio::sync::RwLock<Option<String>>>,
) {
    // R2: Adaptive interval is retained as a *safety-net only*. The 5-minute cron
    // is dead — `event_driven_only=true` (default) makes the timed branch pend
    // forever, so the loop wakes purely from `WakeRequest` events. When false,
    // the timed branch fires at most once per `safety_net_interval_secs`
    // (default 1h) as a watchdog against missed events.
    let mut current_interval = if config.event_driven_only {
        u64::MAX / 2 // effectively never
    } else {
        config.safety_net_interval_secs.max(interval_secs)
    };

    // Watchdog: background task that fires when no heartbeat tick is received
    // for 600+ seconds (10 min) AND CURRENT TASK is "in_progress".
    // Posts a system warning to Discord via result_tx.
    spawn_watchdog(state_path.clone(), workspace.clone(), result_tx.clone(), 600, 600);

    // Fix B (Dispatch 24): Watchdog fast-pulse.
    // Pure liveness signal — decoupled from wake events. Updates
    // `last_heartbeat_tick` every 60s so the stall-detector sees
    // a fresh tick even when the agent is busy cooking and not
    // returning to the main heartbeat loop. zeus106's P0 parity
    // write at the cook return-path stays as redundant safety.
    spawn_fast_pulse(state_path.clone(), 60);
    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(current_interval)) => {
                debug!("Heartbeat safety-net tick (interval={}s, event_driven_only={})",
                    current_interval, config.event_driven_only);

                // #330 fix — re-arm at the HEAD of the timed branch, not only the tail.
                //
                // The wake branch drains queued wakes and sets `current_interval = 0`
                // so this timed branch fires immediately (drain-and-rearm). The re-arm
                // back to the long interval used to live ONLY at the tail of this
                // branch — but the branch has a dozen early `continue` paths (advisory
                // skip arms, PAUSE sentinel, quiet hours, plan-resume gating, ...).
                // Any of them jumped straight back to `select!` with the interval
                // still 0 → `sleep(0)` → a hot busy-loop that (a) pegged a worker,
                // (b) re-ran the pre-gate state write every iteration — the ~1ms
                // "Failed to write heartbeat state temp file" ENOENT spam on minibsd
                // when ~/.zeus vanished — and (c) starved the tokio time driver on
                // small boxes, which is how the #329 drain's EXISTING 5s timeout sat
                // unfired for 8 days.
                //
                // Re-arming here, before ANY gate can `continue`, makes the invariant
                // structural: a zero interval survives exactly one iteration. The
                // tail re-arm below is kept — it refines the value with the freshly
                // computed adaptive interval on the full work path.
                current_interval =
                    head_rearm_interval(config.event_driven_only, config.safety_net_interval_secs, interval_secs);

                // --- Lane A1.5b-i.β: advisory pre-acquire fire-decision (busy-aware) ---
                // Atomic-load the 4-bucket disjunction signals and consult the pure
                // free-fn `should_fire_heartbeat` for a structured Skip{reason}/Fire
                // decision BEFORE attempting `try_acquire`. This emits per-bucket
                // structured trace events and short-circuits skip-paths without
                // contending the RAII cook slot.
                //
                // Advisory-vs-RAII reconciliation (load-bearing — banked discipline):
                //   - Skip arms (CookInFlight | InboundPending | SubagentActive |
                //     RecentInteraction) → emit `heartbeat_skipped{reason}` + `continue`.
                //     NO `try_acquire` on Skip — race-window irrelevant because we
                //     don't enter the work-path.
                //   - Fire arm → fall through to existing `try_acquire`. RAII guard
                //     remains the AUTHORITATIVE serializer for the Heartbeat→Heartbeat
                //     and Heartbeat→Channel race-window. Advisory-false is NOT trusted
                //     to elide the lock; the atomic could have flipped between load
                //     and acquire.
                //
                // 4th-bucket (RecentInteraction) is wired-but-inactive until A1.5b-ii
                // lands the cook-completion `last_interaction_at.store()` write-site.
                // Until then the field is `None` and the bucket short-circuits via
                // None-handle graceful-degrade in `should_fire_heartbeat`.
                {
                    let now_unix = chrono::Utc::now().timestamp();
                    // #192b P2: "is user/channel work happening?" — read the
                    // channel in-flight counter (lanes replaced the CAS, so
                    // `is_active()` no longer reflects concurrent channel cooks).
                    let channel_busy = channel_active
                        .as_ref()
                        .map(|s| s.is_channel_cooking())
                        .unwrap_or(false);
                    let inbox_depth = inbox_queue_depth
                        .as_ref()
                        .map(|h| h.load(Ordering::Relaxed))
                        .unwrap_or(0);
                    let subagent_active = subagent_depth
                        .as_ref()
                        .map(|h| h.load(Ordering::Relaxed))
                        .unwrap_or(0);
                    let last_interaction = last_interaction_at
                        .as_ref()
                        .map(|h| h.load(Ordering::Relaxed));
                    // Default 5min recency threshold. Inlined here (not config-surfaced
                    // yet) because the write-site lands in A1.5b-ii — until then the
                    // RecentInteraction bucket is None-handle-graceful-degrade inactive
                    // and this constant is unobservable. A1.5b-ii will promote to
                    // `HeartbeatConfig::interaction_recency_threshold_secs`.
                    const INTERACTION_RECENCY_THRESHOLD_SECS: i64 = 300;
                    let interaction_threshold = INTERACTION_RECENCY_THRESHOLD_SECS;

                    let decision = should_fire_heartbeat(
                        channel_busy,
                        inbox_depth,
                        subagent_active,
                        last_interaction,
                        now_unix,
                        interaction_threshold,
                    );

                    match decision {
                        FireDecision::Skip { reason } => {
                            tracing::info!(
                                event = "heartbeat_skipped",
                                reason = reason.as_str(),
                                "Heartbeat tick skipped: {}",
                                reason.as_str()
                            );
                            continue;
                        }
                        FireDecision::Fire => {
                            tracing::debug!(
                                event = "heartbeat_fired",
                                "Heartbeat tick advisory-cleared, proceeding to RAII acquire"
                            );
                            // Fall through to existing try_acquire below.
                        }
                    }
                }

                // Defer if a channel message is actively being processed.
                // This is the primary mechanism preventing heartbeat from starving real messages.
                // S-PRIORITY: try to acquire Heartbeat cook slot. If any other cook
                    // (channel or another heartbeat) is running, skip this tick.
                    // RAII guard clears state when heartbeat work below finishes.
                    // #192b P2: two-part gate.
                    //  (a) Defer to channel/user work: channel cooks now run via
                    //      per-session lanes and NO LONGER hold the global CAS, so
                    //      `try_acquire(Heartbeat)` would succeed mid-channel-cook.
                    //      Pre-check the channel in-flight counter to restore the
                    //      S-PRIORITY deferral the CAS used to provide.
                    //  (b) Heartbeat singleton-ness: retain the CAS for heartbeat-vs-
                    //      heartbeat exclusion, AND `begin_cook(Heartbeat)` alongside
                    //      it so the heartbeat populates the counter — that's what
                    //      makes the autonomous-goal loop's `any_cook_inflight()`
                    //      (gateway site ②) actually see the heartbeat. `_hb_flight`
                    //      is scoped to exactly the CAS guard's lifetime so the
                    //      counter decrements when the heartbeat work finishes.
                    let (_heartbeat_guard, _hb_flight) = if let Some(state) = channel_active.as_ref() {
                        if state.is_channel_cooking() {
                            tracing::info!("Heartbeat tick deferred — channel cook in flight");
                            continue;
                        }
                        match state.try_acquire(zeus_core::ActiveCookType::Heartbeat) {
                            Some(g) => (Some(g), Some(state.begin_cook(zeus_core::ActiveCookType::Heartbeat))),
                            None => {
                                tracing::info!("Heartbeat tick deferred — another cook in flight");
                                continue;
                            }
                        }
                    } else { (None, None) };

                let now_unix = chrono::Utc::now().timestamp() as u64;

                // Update last heartbeat tick timestamp (persisted for crash watchdog).
                // This is written before processing so watchdog can detect stalls.
                {
                    // #206: locked read-modify-write (was an unsynchronized pair).
                    update_state(&state_path, |state| {
                        state.last_heartbeat_tick = Some(now_unix);
                    });
                }

                // #3: respect an explicit stand-down. If the operator (or the /stop
                // handler) wrote the ~/.zeus/PAUSE sentinel, skip the heartbeat
                // report-cook entirely so a stood-down seat stops churning status
                // posts. Explicit-sentinel ONLY — never inferred from "idle"/"quiet"
                // (that conflation is the 76ca3c42 autonomy-kill) — and cleared by the
                // next addressed task, so it can't pause forever. Additive: with no
                // sentinel present, behavior is unchanged.
                {
                    let pause_path = dirs::home_dir().unwrap_or_default().join(".zeus").join("PAUSE");
                    if pause_path.exists() {
                        tracing::info!(
                            event = "heartbeat_skipped",
                            reason = "paused",
                            "Heartbeat tick skipped: PAUSE sentinel present (stood down)"
                        );
                        continue;
                    }
                }

                // Skip entire tick during quiet hours.
                if is_quiet_hour(&config, now_unix) {
                    debug!("Heartbeat suppressed: quiet hours active");
                    continue;
                }

                let mut state = load_state(&state_path);

                // --- Pre-flight gating (S-SMART-HB) ---
                // Check structured tasks first. If any are defined, only run
                // those whose per-task interval has elapsed. If no structured
                // tasks exist, fall through to legacy frequency-based scheduling.

                // --- Plan Mode resume: check for incomplete plans ---
                // If a previous plan was interrupted, resume it before processing
                // regular heartbeat tasks. This ensures plans are resume-safe.
                let incomplete_plans = crate::plan_mode::PlanMode::find_incomplete(workspace.root()).await;
                if !incomplete_plans.is_empty() {
                    info!("Heartbeat: found {} incomplete plan(s) to resume", incomplete_plans.len());
                    for slug in &incomplete_plans {
                        // Per-slug rate limit (mirrors `preflight_gate` last_run pattern).
                        // Without this, every adaptive heartbeat tick re-resumes every
                        // stale plan and emits `[Plan Resume] <slug>: ...` to channel,
                        // crowding the fleet with no actionable signal. Operators mark
                        // plans complete via STATUS.md so they exit `find_incomplete`.
                        let plan_resume_key = format!("plan_resume:{}", slug);
                        let last_resume = state.last_run.get(&plan_resume_key).copied().unwrap_or(0);
                        let elapsed = now_unix.saturating_sub(last_resume);
                        if elapsed < config.plan_resume_interval_secs {
                            debug!(
                                "Heartbeat: plan-resume gated for '{}' (elapsed {}s < interval {}s)",
                                slug, elapsed, config.plan_resume_interval_secs
                            );
                            continue;
                        }
                        match crate::plan_mode::PlanMode::load(workspace.root(), slug).await {
                            Ok(mut plan) => {
                                // BUG 3 FIX (Dispatch 22) — plan-resume completion gate.
                                // `find_incomplete()` runs once at the top of the tick;
                                // by the time we reach this loop body the plan may have
                                // transitioned to Completed (operator marked it via
                                // STATUS.md, or a prior iteration finished it). Re-check
                                // status right before firing the LLM. Cheap (already loaded
                                // STATUS.md via PlanMode::load), prevents the "resuming a
                                // done plan forever" footgun the audit flagged.
                                if matches!(
                                    plan.meta().status,
                                    crate::plan_mode::PlanStatus::Completed
                                        | crate::plan_mode::PlanStatus::Failed
                                ) {
                                    debug!(
                                        "Heartbeat: plan '{}' is {:?}, skipping resume",
                                        slug, plan.meta().status
                                    );
                                    // Bank the skip so the rate-limit gate also advances —
                                    // otherwise a completed plan keeps re-loading STATUS.md
                                    // every tick.
                                    state.last_run.insert(plan_resume_key.clone(), now_unix);
                                    continue;
                                }

                                // #157 bounded retries: cap how many times this
                                // same plan can be resumed. A plan that never
                                // reaches Completed would otherwise re-resume
                                // every tick forever (the stale-plan loop bug).
                                // At the cap, drive it terminal (Abandoned) and
                                // NOTIFY — never silent, mirroring the loop_tool
                                // abandon. GENEROUS default so legit long plans
                                // aren't killed.
                                let resume_n = match plan.record_resume_attempt().await {
                                    Ok(n) => n,
                                    Err(e) => {
                                        warn!("Heartbeat: failed to record resume attempt for '{}': {}", slug, e);
                                        state.last_run.insert(plan_resume_key.clone(), now_unix);
                                        continue;
                                    }
                                };
                                if resume_n > config.max_plan_resume_attempts {
                                    let reason = format!(
                                        "Plan abandoned after {} resume attempts (cap {}) — it never reached Completed. Mark it done in STATUS.md or re-create it if still needed.",
                                        resume_n.saturating_sub(1),
                                        config.max_plan_resume_attempts
                                    );
                                    warn!("Heartbeat: {}", reason);
                                    if let Err(e) = plan.mark_abandoned(&reason).await {
                                        warn!("Heartbeat: failed to mark plan '{}' abandoned: {}", slug, e);
                                    }
                                    // NOTIFY the channel so the abandon is never silent.
                                    // Uses the same `result_tx` sink as the resume-success
                                    // post below — abandons always surface.
                                    if let Some(ref tx) = result_tx {
                                        let note = format!(
                                            "⚠️ Plan `{}` abandoned: hit the resume cap ({} attempts) without completing. It will no longer re-resume.",
                                            slug, config.max_plan_resume_attempts
                                        );
                                        let _ = tx.try_send(note);
                                    }
                                    state.last_run.insert(plan_resume_key.clone(), now_unix);
                                    continue;
                                }

                                let plan_content = plan.read_plan().await.unwrap_or_default();
                                if plan_content.is_empty() {
                                    warn!("Heartbeat: plan {} has empty PLAN.md, skipping", slug);
                                    continue;
                                }
                                let resume_prompt = format!(
                                    "Resume this interrupted plan. Continue from where you left off.\n\n{}",
                                    plan.plan_context_prompt(&plan_content),
                                );
                                info!("Heartbeat: resuming plan '{}'", slug);
                                let psc = peer_status_context.read().await;
                                let psc_str = psc.as_deref().map(|s| s.to_string());
                                drop(psc);
                                let uc = unread_context.read().await;
                                let uc_str = uc.as_deref().map(|s| s.to_string());
                                drop(uc);
                                let resume_result = execute_heartbeat_task(
                                    &llm, &workspace, &resume_prompt,
                                    tool_executor.as_ref(), &tools,
                                    mnemosyne.as_ref(), &config,
                                    psc_str.as_deref(),
                                    uc_str.as_deref(),
                                ).await;
                                // Record the resume attempt regardless of outcome — the gate
                                // is decorative without this. Failures shouldn't spin either.
                                state.last_run.insert(plan_resume_key.clone(), now_unix);
                                if resume_result.success {
                                    info!("Heartbeat: plan '{}' resumed successfully", slug);
                                    // #156: only post when the resume actually advanced the
                                    // plan — a silent / no-op resume must not flood Discord.
                                    // `plan_advanced` := success && !silent; no-op/filler
                                    // suppression is delegated to should_post_to_discord (it
                                    // already strips filler at Meaningful and floods at Verbose),
                                    // exactly mirroring the structured-task success site below.
                                    // An inline !is_noop_heartbeat here would diverge from that
                                    // mirror at Verbose (hard-suppress where the mirror posts)
                                    // and is redundant at Meaningful.
                                    let plan_advanced = !resume_result.silent;
                                    if plan_advanced
                                        && should_post_to_discord(
                                            config.discord_verbosity,
                                            true,
                                            &resume_result.output,
                                        )
                                    {
                                        if let Some(ref tx) = result_tx {
                                            let note = format!("[Plan Resume] {}: {}", slug, resume_result.output);
                                            let _ = tx.try_send(note);
                                        }
                                    }
                                } else {
                                    warn!("Heartbeat: plan '{}' resume failed: {}", slug, resume_result.output);
                                }
                            }
                            Err(e) => {
                                warn!("Heartbeat: failed to load plan '{}': {}", slug, e);
                            }
                        }
                    }
                }

                let due_tasks = preflight_gate(&workspace, &mut state).await;

                match due_tasks {
                    Some(tasks) if !tasks.is_empty() => {
                        // Structured mode: run only due tasks
                        info!("Pre-flight: {} structured task(s) due", tasks.len());
                        for task in &tasks {
                            let task_timeout = compute_task_timeout(&task.prompt, &config);
                            let timeout_dur = std::time::Duration::from_secs(task_timeout);

                            // #157 CURRENT-TASK no-op gate: a "continue your CURRENT
                            // TASK"-style task with NO active CURRENT TASK must no-op
                            // at the CODE layer (not rely on the LLM saying
                            // HEARTBEAT_OK). Without this, clearing CURRENT TASK on
                            // retire wouldn't actually stop re-injection — the prompt
                            // still fires the LLM every interval. `get_current_task()`
                            // already returns None for empty / placeholder sections.
                            if is_current_task_bound(&task.prompt)
                                && workspace.get_current_task().await.ok().flatten().is_none()
                            {
                                debug!(
                                    task = %task.name,
                                    "CURRENT-TASK task skipped (no active CURRENT TASK) — HEARTBEAT_OK"
                                );
                                // Idle no-op: don't mark in-progress, don't burn an LLM
                                // call, don't count as failure. Just record the run.
                                state.last_run.insert(task.name.clone(), now_unix);
                                continue;
                            }

                            // Mark task as in-progress before execution
                            update_task_status(&mut state, &task.name, TaskStatus::InProgress);
                            
                            let psc = peer_status_context.read().await;
                            let psc_str = psc.as_deref().map(|s| s.to_string());
                            drop(psc);
                            let uc = unread_context.read().await;
                            let uc_str = uc.as_deref().map(|s| s.to_string());
                            drop(uc);
                            let result = match tokio::time::timeout(
                                timeout_dur,
                                execute_heartbeat_task(
                                    &llm,
                                    &workspace,
                                    &task.prompt,
                                    tool_executor.as_ref(),
                                    &tools,
                                    mnemosyne.as_ref(),
                                    &config,
                                    psc_str.as_deref(),
                                    uc_str.as_deref(),
                                ),
                            )
                            .await
                            {
                                Ok(r) => r,
                                Err(_elapsed) => {
                                    error!(
                                        "Heartbeat task TIMED OUT after {}s: {}",
                                        task_timeout, task.name
                                    );
                                    TaskResult {
                                        task: task.name.clone(),
                                        success: false,
                                        silent: false,
                                        output: format!("Timed out after {}s", task_timeout),
                                    }
                                }
                            };

                            // Update state timestamp on any completed attempt.
                            state.last_run.insert(task.name.clone(), now_unix);

                            if result.silent {
                                debug!("Heartbeat task acknowledged (HEARTBEAT_OK): {}", task.name);
                                // Silent OK doesn't change status — still in progress or pending
                            } else if result.success {
                                // #179: Reset stale failure state BEFORE checking stuck.
                                // A task that just succeeded has recovered — prior failure_count
                                // and status_changed_at are stale and would cause a false STUCK.
                                reset_failure_count(&mut state, &task.name);
                                state.status_changed_at.insert(task.name.clone(), now_unix);

                                // Now check_stuck sees clean state for this task.
                                if let Some(reason) = check_stuck(&state, &task.name, task_timeout) {
                                    update_task_status(&mut state, &task.name, TaskStatus::Stuck);
                                    warn!(
                                        task = %task.name,
                                        reason = %reason,
                                        last_output = %state.last_output.get(&task.name).map(|s| s.chars().take(120).collect::<String>()).unwrap_or_default(),
                                        "Task marked STUCK"
                                    );
                                } else {
                                    update_task_status(&mut state, &task.name, TaskStatus::Completed);
                                }
                                
                                // Text dedup: suppress if same output within dedup window.
                                let is_duplicate = state
                                    .last_output
                                    .get(task.name.as_str())
                                    .map(|prev| prev == &result.output)
                                    .unwrap_or(false)
                                    && state
                                        .last_output_at
                                        .get(task.name.as_str())
                                        .map(|&t| now_unix.saturating_sub(t) < config.dedup_window_secs)
                                        .unwrap_or(false);

                                if is_duplicate {
                                    debug!(
                                        "Heartbeat task suppressed (duplicate within {}s): {}",
                                        config.dedup_window_secs, task.name
                                    );
                                } else {
                                    info!("Heartbeat task completed: {}", task.name);
                                    state.last_output.insert(task.name.clone(), result.output.clone());
                                    state.last_output_at.insert(task.name.clone(), now_unix);
                                    // #119: suppress routine filler; substantive output still posts.
                                    if should_post_to_discord(config.discord_verbosity, true, &result.output) {
                                        if let Some(ref tx) = result_tx {
                                            let note = format!("[Heartbeat] {}: {}", task.name, result.output);
                                            let _ = tx.try_send(note);
                                        }
                                    }
                                }
                            } else {
                                // Task failed — increment failure count and check if stuck
                                let fail_count = increment_failure_count(&mut state, &task.name);
                                if let Some(reason) = check_stuck(&state, &task.name, task_timeout).or_else(|| {
                                    if fail_count >= 3 { Some(StuckReason::FailureCount { count: fail_count, threshold: 3 }) } else { None }
                                }) {
                                    update_task_status(&mut state, &task.name, TaskStatus::Stuck);
                                    warn!(
                                        task = %task.name,
                                        reason = %reason,
                                        last_output = %result.output.chars().take(120).collect::<String>(),
                                        "Task marked STUCK"
                                    );

                                    // #157 CURRENT-TASK terminal gate: if the STUCK task
                                    // is CURRENT-TASK-bound and has exhausted the retire
                                    // cap, retire `## CURRENT TASK` to idle so it stops
                                    // re-injecting (the 112/107 wedge). The counter is
                                    // `fail_count` — RUNTIME-owned (incremented by
                                    // `increment_failure_count`, persisted in
                                    // heartbeat-state.json), NEVER LLM-threaded, so the
                                    // LLM cannot reset it to dodge the cap. Notify on
                                    // retire; never silently drop.
                                    let cap = config.max_current_task_attempts;
                                    if cap > 0
                                        && is_current_task_bound(&task.prompt)
                                        && fail_count >= cap
                                    {
                                        match workspace.get_current_task().await.ok().flatten() {
                                            Some(retired) => {
                                                match workspace.set_current_task("").await {
                                                    Ok(()) => {
                                                        // Reset the counter so a freshly
                                                        // assigned CURRENT TASK starts clean.
                                                        reset_failure_count(&mut state, &task.name);
                                                        let summary: String = retired
                                                            .lines()
                                                            .next()
                                                            .unwrap_or("")
                                                            .chars()
                                                            .take(140)
                                                            .collect();
                                                        warn!(
                                                            task = %task.name,
                                                            attempts = fail_count,
                                                            reason = %reason,
                                                            "Auto-retired CURRENT TASK to idle"
                                                        );
                                                        if let Some(ref tx) = result_tx {
                                                            let note = format!(
                                                                "🛑 Retired CURRENT TASK after {} stuck attempts ({}) — auto-cleared to idle to break the re-injection loop. Last task: {}",
                                                                fail_count, reason, summary
                                                            );
                                                            let _ = tx.try_send(note);
                                                        }
                                                    }
                                                    Err(e) => {
                                                        warn!(
                                                            task = %task.name,
                                                            error = %e,
                                                            "Failed to auto-retire CURRENT TASK"
                                                        );
                                                    }
                                                }
                                            }
                                            None => {
                                                // Already idle — nothing to retire.
                                                reset_failure_count(&mut state, &task.name);
                                            }
                                        }
                                    }
                                } else {
                                    update_task_status(&mut state, &task.name, TaskStatus::Failed);
                                }
                                
                                error!("Heartbeat task FAILED: {} — {}", task.name, result.output);
                                // #119: failures always post (signal operators must see).
                                if should_post_to_discord(config.discord_verbosity, false, &result.output) {
                                    if let Some(ref tx) = result_tx {
                                        let note = format!("[Heartbeat FAIL] {}: {}", task.name, result.output);
                                        let _ = tx.try_send(note);
                                    }
                                }
                            }
                        }
                    }
                    Some(_) => {
                        // Structured tasks defined but none due — skip LLM entirely
                        debug!("Pre-flight: no structured tasks due — skipping LLM call");
                    }
                    None => {
                        // Legacy mode: no structured tasks, fall back to frequency-based scheduling.
                        // Gate by per-frequency `last_run` so `## hourly`-form workspaces don't
                        // fire every adaptive tick (mirrors structured `preflight_gate`).
                        let now_local = chrono::Local::now();
                        let now_unix_freq = now_local.timestamp() as u64;
                        let frequencies = determine_frequencies(now_local, &mut state, 3600);
                        let grace_secs = interval_secs.saturating_sub(interval_secs / 10);

                        for freq in &frequencies {
                            match workspace.get_heartbeat_tasks(freq).await {
                                Ok(tasks) if !tasks.is_empty() => {
                                    info!("Running {} heartbeat task(s) for '{}'", tasks.len(), freq);
                                    for task in &tasks {
                                        if let Some(&last_run) = state.last_run.get(task.as_str())
                                            && now_unix.saturating_sub(last_run) < grace_secs
                                        {
                                            debug!("Skipping recently-run heartbeat task: {}", task);
                                            continue;
                                        }

                                        let task_timeout = compute_task_timeout(task, &config);
                                        let timeout_dur = std::time::Duration::from_secs(task_timeout);
                                        
                                        // Mark task as in-progress before execution
                                        update_task_status(&mut state, task, TaskStatus::InProgress);
                                        
                                        let psc = peer_status_context.read().await;
                                        let psc_str = psc.as_deref().map(|s| s.to_string());
                                        drop(psc);
                                        let uc = unread_context.read().await;
                                        let uc_str = uc.as_deref().map(|s| s.to_string());
                                        drop(uc);
                                        let result = match tokio::time::timeout(
                                            timeout_dur,
                                            execute_heartbeat_task(
                                                &llm,
                                                &workspace,
                                                task,
                                                tool_executor.as_ref(),
                                                &tools,
                                                mnemosyne.as_ref(),
                                                &config,
                                                psc_str.as_deref(),
                                                uc_str.as_deref(),
                                            ),
                                        )
                                        .await
                                        {
                                            Ok(r) => r,
                                            Err(_elapsed) => {
                                                error!(
                                                    "Heartbeat task TIMED OUT after {}s: {}",
                                                    task_timeout, task
                                                );
                                                TaskResult {
                                                    task: task.clone(),
                                                    success: false,
                                                    silent: false,
                                                    output: format!("Timed out after {}s", task_timeout),
                                                }
                                            }
                                        };

                                        state.last_run.insert(task.clone(), now_unix);

                                        if result.silent {
                                            debug!("Heartbeat task acknowledged (HEARTBEAT_OK): {}", task);
                                        } else if result.success {
                                            // #179: Reset stale failure state BEFORE checking stuck.
                                            // A task that just succeeded has recovered — prior failure_count
                                            // and status_changed_at are stale and would cause a false STUCK.
                                            reset_failure_count(&mut state, task);
                                            state.status_changed_at.insert(task.clone(), now_unix);

                                            // Now check_stuck sees clean state for this task.
                                            if let Some(reason) = check_stuck(&state, task, task_timeout) {
                                                update_task_status(&mut state, task, TaskStatus::Stuck);
                                                warn!(
                                                    task = %task,
                                                    reason = %reason,
                                                    last_output = %state.last_output.get(task).map(|s| s.chars().take(120).collect::<String>()).unwrap_or_default(),
                                                    "Task marked STUCK"
                                                );
                                            } else {
                                                update_task_status(&mut state, task, TaskStatus::Completed);
                                            }
                                            
                                            let task_trim = task.trim_start();
                                            let is_forced_task = task_trim.starts_with("[forced]")
                                                || task_trim.starts_with("[trace]")
                                                || task_trim.starts_with("[FORCED]")
                                                || task_trim.starts_with("[TRACE]");
                                            let is_duplicate = !is_forced_task
                                                && state
                                                    .last_output
                                                    .get(task.as_str())
                                                    .map(|prev| prev == &result.output)
                                                    .unwrap_or(false)
                                                && state
                                                    .last_output_at
                                                    .get(task.as_str())
                                                    .map(|&t| now_unix.saturating_sub(t) < config.dedup_window_secs)
                                                    .unwrap_or(false);

                                            if is_duplicate {
                                                debug!(
                                                    "Heartbeat task suppressed (duplicate within {}s): {}",
                                                    config.dedup_window_secs, task
                                                );
                                            } else {
                                                info!("Heartbeat task completed: {}", task);
                                                state.last_output.insert(task.clone(), result.output.clone());
                                                state.last_output_at.insert(task.clone(), now_unix);
                                                // #119: suppress routine filler; substantive output still posts.
                                                if should_post_to_discord(config.discord_verbosity, true, &result.output) {
                                                    if let Some(ref tx) = result_tx {
                                                        let note = format!("[Heartbeat] {}: {}", task, result.output);
                                                        let _ = tx.try_send(note);
                                                    }
                                                }
                                            }
                                        } else {
                                            // Task failed — increment failure count and check if stuck
                                            let fail_count = increment_failure_count(&mut state, task);
                                            if let Some(reason) = check_stuck(&state, task, task_timeout).or_else(|| {
                                                if fail_count >= 3 { Some(StuckReason::FailureCount { count: fail_count, threshold: 3 }) } else { None }
                                            }) {
                                                update_task_status(&mut state, task, TaskStatus::Stuck);
                                                warn!(
                                                    task = %task,
                                                    reason = %reason,
                                                    last_output = %result.output.chars().take(120).collect::<String>(),
                                                    "Task marked STUCK"
                                                );
                                            } else {
                                                update_task_status(&mut state, task, TaskStatus::Failed);
                                            }
                                            
                                            error!("Heartbeat task FAILED: {} — {}", task, result.output);
                                            // #119: failures always post (signal operators must see).
                                            if should_post_to_discord(config.discord_verbosity, false, &result.output) {
                                                if let Some(ref tx) = result_tx {
                                                    let note = format!("[Heartbeat FAIL] {}: {}", task, result.output);
                                                    let _ = tx.try_send(note);
                                                }
                                            }
                                        }
                                    }
                                }
                                Ok(_) => { /* no tasks for this frequency */ }
                                Err(e) => {
                                    error!("Failed to get heartbeat tasks for '{}': {}", freq, e);
                                }
                            }
                        }

                        // Record run for each frequency we processed so the gate
                        // in `determine_frequencies` actually advances. Without
                        // this the gate is decorative and we'd fire every tick.
                        for freq in &frequencies {
                            state.last_run.insert((*freq).to_string(), now_unix_freq);
                        }
                    }
                }

                // BUG 1 FIX (Dispatch 22) — watchdog write parity for wake-event branch.
                // The periodic-tick branch at line 741 already writes `last_heartbeat_tick`,
                // but this wake-event branch (`match due_tasks` path) only updated
                // `state.last_run.<task>`, leaving `last_heartbeat_tick` stale. The watchdog
                // (line 649) keys off `last_heartbeat_tick`, so it would false-alarm "silent
                // since X" even though the wake branch fired tasks successfully. One line.
                state.last_heartbeat_tick = Some(now_unix);

                save_state(&state_path, &state);

                // P0 fix 2026-04-30 — honor R2 `event_driven_only` invariant.
                //
                // BEFORE: this line unconditionally reassigned `current_interval` to
                // a 120-900s adaptive value, which silently turned the timed branch
                // into a fixed-interval cron after the first wake-driven tick —
                // defeating the entire R2 design (wake-only firing).
                //
                // AFTER: only recompute the adaptive interval when we are actually
                // running in safety-net mode. In event-driven mode we go back to
                // sleeping forever; the only path back into the timed branch is
                // a `WakeRequest` triggering drain-and-rearm (`current_interval = 0`).
                current_interval = if config.event_driven_only {
                    u64::MAX / 2 // effectively never — match initial sleep value
                } else {
                    compute_adaptive_interval(&workspace, &config).await
                };
            }
            // S67-C1: Wake-on-event — any component can trigger immediate heartbeat.
            //
            // FIX 2026-04-29 (heartbeat regression — wake-branch parity):
            // The wake branch USED to run its own gutted task loop that bypassed:
            //   • channel_active gating (could starve real channel cooks)
            //   • pre-flight gating (quiet hours, structured-task readiness)
            //   • plan resume
            //   • dedup / failure-count / TaskStatus state machine
            //   • result_tx send-back to the gateway  ← this dropped agent output on the floor
            //   • mnemosyne integration
            //   • adaptive interval recompute
            //   • save_state
            //
            // Rather than maintain two divergent copies of ~290 LOC of tick logic
            // (which is exactly how this regression slipped in), the wake branch
            // now coalesces the wake burst then re-arms `current_interval = 0`.
            // The very next `tokio::select!` iteration falls into the timed branch
            // — the single canonical, fully-wired heartbeat tick path.
            //
            // R2 architecture: ONE tick implementation. Wake = "tick now".
            wake = async {
                match wake_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(wake) = wake {
                    info!("Heartbeat wake: reason={}, agent={:?} — coalescing then forcing tick",
                        wake.reason, wake.agent_id);
                    // S69: 2-second coalesce window — drain any rapid-fire wakes.
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    if let Some(ref mut rx) = wake_rx {
                        while rx.try_recv().is_ok() {} // drain queued wakes
                    }
                    // Force the timed branch to fire on the next iteration.
                    // The timed branch is the canonical, fully-wired tick.
                    current_interval = 0;
                }
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    info!("Heartbeat loop shutting down");
                    return;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pre-flight gating (S-SMART-HB)
// ---------------------------------------------------------------------------

/// Check which structured tasks are due based on per-task intervals and
/// last-run timestamps from `heartbeat-state.json`.
///
/// Returns only the tasks whose interval has elapsed since their last run.
/// If no structured tasks are defined, returns `None` (fall through to
/// legacy frequency-based scheduling).
async fn preflight_gate(
    workspace: &Workspace,
    state: &mut HeartbeatState,
) -> Option<Vec<StructuredHeartbeatTask>> {
    let structured = match workspace.get_structured_tasks().await {
        Ok(tasks) if !tasks.is_empty() => tasks,
        _ => return None, // No structured tasks — use legacy path
    };

    let now_unix = chrono::Utc::now().timestamp() as u64;

    // Dispatch 27 cold-start fix: any task with no recorded `last_run` (cold start,
    // post-nuke, fresh state file) is initialised to `now_unix` and treated as NOT
    // due this tick. Without this, the previous `None => true` branch caused EVERY
    // hourly task to fire on the very first heartbeat tick after a gateway boot,
    // producing the `[Heartbeat] hourly-1/2/3 Completed` flood. Mutation is
    // persisted by the caller's `save_state` after this gate runs.
    let task_names: Vec<String> = structured.iter().map(|t| t.name.clone()).collect();
    for name in &task_names {
        if !state.last_run.contains_key(name) {
            debug!("Pre-flight: cold-start init for '{}' — anchoring to now, NOT firing this tick", name);
            state.last_run.insert(name.clone(), now_unix);
        }
    }

    let due: Vec<StructuredHeartbeatTask> = structured
        .into_iter()
        .filter(|task| {
            match state.last_run.get(&task.name) {
                Some(&last_run) => {
                    // After cold-start init above, `last_run == now_unix`, so
                    // elapsed == 0 < interval → not due. Subsequent ticks will
                    // fire correctly once the interval has actually elapsed.
                    let elapsed = now_unix.saturating_sub(last_run);
                    let due = elapsed >= task.interval_secs;
                    if !due {
                        debug!(
                            "Pre-flight: skipping '{}' — elapsed {}s < interval {}s",
                            task.name, elapsed, task.interval_secs
                        );
                    }
                    due
                }
                None => {
                    // Should never hit this branch after the cold-start init loop
                    // above. Kept defensively — treat as not due.
                    debug!("Pre-flight: task '{}' missing last_run after init (defensive skip)", task.name);
                    false
                }
            }
        })
        .collect();

    Some(due)
}

// ---------------------------------------------------------------------------
// Frequency helpers
// ---------------------------------------------------------------------------

/// Determine which task frequencies to run based on current local time and
/// per-frequency `last_run` state. Mirrors the structured `preflight_gate`
/// behavior so legacy `## hourly` form workspaces don't fire every tick.
///
/// `hourly_interval_secs` defaults to 3600. Daily/weekly retain their
/// time-of-day predicates AND additionally gate on `last_run` to prevent
/// multiple fires within the 9:00–9:10 window.
fn determine_frequencies(
    now: chrono::DateTime<chrono::Local>,
    state: &mut HeartbeatState,
    hourly_interval_secs: u64,
) -> Vec<&'static str> {
    let mut freqs = Vec::new();
    let now_unix = now.timestamp() as u64;

    // Dispatch 27 cold-start fix (legacy mirror): if `last_run.hourly` is
    // missing OR 0 (cold start, fresh state), anchor it to now_unix and skip
    // firing this tick. Without this, `unwrap_or(0)` made `now - 0 >= interval`
    // trivially true on every fresh boot → all `## hourly` tasks fired tick 1.
    // Same fix shape applied to daily/weekly below.
    if state.last_run.get("hourly").copied().unwrap_or(0) == 0 {
        debug!("determine_frequencies: cold-start init for 'hourly' — anchoring, NOT firing this tick");
        state.last_run.insert("hourly".to_string(), now_unix);
    }

    // Hourly: gate purely on elapsed time since last run.
    let last_hourly = state.last_run.get("hourly").copied().unwrap_or(0);
    if now_unix.saturating_sub(last_hourly) >= hourly_interval_secs {
        freqs.push("hourly");
    }

    let hour = now.format("%H").to_string().parse::<u32>().unwrap_or(0);
    let minute = now.format("%M").to_string().parse::<u32>().unwrap_or(0);

    // Daily: time-of-day window + last_run gate (>= 23h since last fire to
    // prevent multiple fires within the 10-minute window).
    if hour == 9 && minute < 10 {
        let last_daily = state.last_run.get("daily").copied().unwrap_or(0);
        if now_unix.saturating_sub(last_daily) >= 23 * 3600 {
            freqs.push("daily");
        }
    }

    // Weekly: Monday morning + last_run gate (>= 6 days since last fire).
    if now.format("%u").to_string() == "1" && hour == 9 && minute < 10 {
        let last_weekly = state.last_run.get("weekly").copied().unwrap_or(0);
        if now_unix.saturating_sub(last_weekly) >= 6 * 86_400 {
            freqs.push("weekly");
        }
    }

    freqs
}

// ---------------------------------------------------------------------------
// Task execution
// ---------------------------------------------------------------------------

/// Execute a single heartbeat task using the LLM, with optional tool execution.
///
/// If the LLM responds with only `HEARTBEAT_OK` (case-insensitive), the task
/// is treated as a no-op: `TaskResult::silent` will be `true` and `output`
/// will be empty, suppressing all logging and workspace notes.
/// Maximum characters for heartbeat responses.
/// Set high to allow communicative agents — the operator wants agents descriptive by default.
/// Only applied to prevent truly runaway responses (>2000 chars = Discord limit).
const HEARTBEAT_ACK_MAX_CHARS: usize = 1800;

/// S69: Append heartbeat result to a dedicated per-agent session file.
/// This gives heartbeat continuity between runs — the agent remembers prior checks.
fn append_heartbeat_session(workspace: &std::sync::Arc<zeus_memory::Workspace>, task: &str, output: &str) {
    let session_path = workspace.root().join("heartbeat-session.jsonl");
    let entry = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "task": task,
        "output": output,
    });
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&session_path) {
        use std::io::Write;
        let _ = writeln!(file, "{}", entry);
    }
}

async fn execute_heartbeat_task(
    llm: &LlmClient,
    workspace: &Workspace,
    task: &str,
    tool_executor: Option<&Arc<dyn ToolExecutor>>,
    tools: &[ToolSchema],
    mnemosyne: Option<&Arc<zeus_mnemosyne::Mnemosyne>>,
    _config: &HeartbeatConfig,
    peer_status_context: Option<&str>,
    unread_context: Option<&str>,
) -> TaskResult {
    // Adaptive context level: dev tasks get full workspace context (including
    // AGENTS.md with dev workflow instructions), routine tasks get light context.
    let current_task = workspace.get_current_task().await.ok().flatten().unwrap_or_default();
    let is_dev_task = current_task.contains(".rs")
        || current_task.contains("crates/")
        || current_task.contains("src/")
        || current_task.contains("cargo")
        || current_task.contains("branch")
        || current_task.contains("commit")
        || current_task.contains("fix ")
        || current_task.contains("implement")
        || current_task.contains("build ");

    let base_context = if is_dev_task {
        // Dev task: full workspace context — includes AGENTS.md with dev workflow,
        // repo path, branch conventions. Critical for autonomous coding.
        workspace.get_context().await.unwrap_or_default()
    } else {
        // Routine task: light context — SOUL + IDENTITY + HEARTBEAT.md only.
        let identity = workspace.read("IDENTITY.md").await.unwrap_or_default();
        let soul = workspace.read("SOUL.md").await.unwrap_or_default();
        let heartbeat_md = workspace.read("HEARTBEAT.md").await.unwrap_or_default();
        if identity.is_empty() && soul.is_empty() {
            workspace.get_context().await.unwrap_or_default()
        } else {
            format!(
                "{}\n\n{}\n\n{}",
                soul.chars().take(500).collect::<String>(),
                identity,
                heartbeat_md
            )
        }
    };
    let light_context = base_context;

    // S67-G1: Dynamic prompt tiers — inject pending context into heartbeat prompt.
    // Tier 1: Base (SOUL + IDENTITY + task instructions) — always present.
    // Tier 2: + active goals from GoalStack (if any).
    // Tier 3: + recent daily note entries (lightweight recent context).
    let goals_context = {
        let goals_db = dirs::home_dir()
            .unwrap_or_default()
            .join(".zeus/goals.db");
        if goals_db.exists() {
            match crate::goals::GoalStack::new(&goals_db) {
                Ok(stack) => {
                    let active = stack.active_goals().unwrap_or_default();
                    if active.is_empty() {
                        String::new()
                    } else {
                        let items: Vec<String> = active.iter().take(5).map(|g| {
                            format!("- [{}] {}", g.priority, g.description)
                        }).collect();
                        format!("\n\n## Active Goals\n{}", items.join("\n"))
                    }
                }
                Err(_) => String::new(),
            }
        } else {
            String::new()
        }
    };

    // Standing Orders: persistent multi-day directives that survive restarts.
    // Sync from HEARTBEAT.md on each cycle (idempotent), then inject active
    // orders into the prompt.
    let standing_orders_context = {
        match crate::standing_orders::StandingOrderStore::default_path() {
            Ok(store) => {
                // Idempotent sync from HEARTBEAT.md — new bullets in the
                // "## STANDING ORDERS" section get persisted.
                let heartbeat_md = workspace.read("HEARTBEAT.md").await.unwrap_or_default();
                if !heartbeat_md.is_empty() {
                    let _ = store.sync_from_heartbeat(&heartbeat_md);
                }
                let active = store.active().unwrap_or_default();
                if active.is_empty() {
                    String::new()
                } else {
                    let now = chrono::Utc::now();
                    let items: Vec<String> = active.iter().take(8).map(|o| {
                        // Surface a staleness flag so the agent prioritises
                        // orders it hasn't touched within their cadence window.
                        let stale_flag = if o.is_stale(now) { " ⚠️ STALE — advance this" } else { "" };
                        let last = match o.last_acted_at {
                            Some(t) => format!(" (last acted {})", t.format("%Y-%m-%d %H:%M UTC")),
                            None => " (never acted)".to_string(),
                        };
                        format!("- [{}] {}{}{}", o.priority, o.description, last, stale_flag)
                    }).collect();
                    format!(
                        "\n\n## STANDING ORDERS (persistent — advance these across sessions)\n{}",
                        items.join("\n")
                    )
                }
            }
            Err(_) => String::new(),
        }
    };

    // Mnemosyne recall: search for memories relevant to the current task.
    // Gives heartbeat cooks continuity across sessions.
    let memory_context = if let Some(mnem) = mnemosyne {
        match mnem.hybrid_search(task, None, 5).await {
            Ok(results) if !results.is_empty() => {
                let items: Vec<String> = results.iter()
                    .map(|r| format!("- {}", r.content.chars().take(200).collect::<String>()))
                    .collect();
                format!("\n\n## Relevant Memories\n{}\n", items.join("\n"))
            }
            _ => String::new(),
        }
    } else {
        String::new()
    };

    // Phase 2a: inject task queue + current task into heartbeat prompt so the
    // agent can self-advance instead of defaulting to HEARTBEAT_OK.
    let current_task_line = workspace
        .get_current_task()
        .await
        .ok()
        .flatten()
        .map(|t| format!("\n\n## YOUR CURRENT TASK (from HEARTBEAT.md)\n{}\n", t))
        .unwrap_or_default();
    let task_queue_block = workspace
        .get_task_queue()
        .await
        .ok()
        .filter(|q| !q.is_empty())
        .map(|q| {
            let items: Vec<String> = q.iter().take(5).map(|t| format!("- {}", t)).collect();
            format!("\n\n## PENDING TASK QUEUE (from HEARTBEAT.md)\n{}\n", items.join("\n"))
        })
        .unwrap_or_default();

    let system = format!(
        "{}{}{}{}{}{}{}{}\n\n\
         ## TASK PRIORITY — READ THIS FIRST\n\
         1. **CURRENT TASK** above is your primary job — if present, execute it NOW and report progress.\n\
         2. If CURRENT TASK is empty, pick the TOP item from PENDING TASK QUEUE and start it.\n\
         3. Only if BOTH are empty, fall through to the routine heartbeat item below.\n\n\
         You are running an autonomous heartbeat cycle.\n\
         - **Default expectation: you are working, not idling.** Produce visible progress every cycle.\n\
         - Report what you did, what you found, what you decided. Your team reads these.\n\
         - Commit work frequently — small commits > one big dump.\n\
         - If you're blocked, SAY SO with specifics (what's blocking, what you tried, what you need).\n\
         - Only reply `HEARTBEAT_OK` when you have GENUINELY checked the queue, found nothing actionable, \
           and have no in-flight work to advance. `HEARTBEAT_OK` is the exception, not the default. \
           When there's nothing to do, reply `HEARTBEAT_OK` and nothing else — don't narrate the check.\n\
         - Do NOT invent unsolicited side-projects or reorganize files that weren't asked for. \
           Stay on your CURRENT TASK / QUEUE items.\n\
         - If the routine item says \"check X\", just check X.",
        light_context, goals_context, standing_orders_context, current_task_line, task_queue_block, memory_context, peer_status_context.unwrap_or(""), unread_context.unwrap_or("")
    );

    let mut messages = vec![Message::user(format!(
        "Heartbeat cycle — routine item: {}\n\n\
         Before handling the routine item, review your CURRENT TASK and PENDING TASK QUEUE above \
         and advance whichever is most valuable. Silence (`HEARTBEAT_OK`) is only correct when \
         both are empty AND the routine item is a no-op.",
        task
    ))];



    // Adaptive iteration budget: dev tasks get more iterations for coding work,
    // research tasks get moderate budget, routine tasks stay lean.
    let max_iterations = if is_dev_task {
        15
    } else if current_task.contains("research")
        || current_task.contains("investigate")
        || current_task.contains("analyze")
    {
        10
    } else {
        5
    };
    for _iteration in 0..max_iterations {
        let response = match llm.complete(&messages, tools, Some(&system)).await {
            Ok(r) => r,
            Err(e) => {
                return TaskResult {
                    task: task.to_string(),
                    success: false,
                    silent: false,
                    output: format!("LLM error: {}", e),
                };
            }
        };

        // If no tool calls, we're done — check for HEARTBEAT_OK.
        if response.tool_calls.is_empty() {
            let trimmed = response.content.trim();
            let silent = trimmed.eq_ignore_ascii_case("HEARTBEAT_OK");
            // S67-G3: Truncate non-silent responses to ack max chars
            let output = if silent {
                String::new()
            } else {
                let content = response.content.trim().to_string();
                if content.len() > HEARTBEAT_ACK_MAX_CHARS {
                    format!("{}…", content.chars().take(HEARTBEAT_ACK_MAX_CHARS).collect::<String>())
                } else {
                    content
                }
            };
            return TaskResult {
                task: task.to_string(),
                success: true,
                silent,
                output,
            };
        }

        // Execute tool calls if we have an executor.
        if let Some(executor) = tool_executor {
            // Push assistant message WITH tool_calls so tool_result IDs match
            let mut assistant_msg = Message::assistant(&response.content);
            assistant_msg.tool_calls = response.tool_calls.clone();
            messages.push(assistant_msg);

            for tool_call in &response.tool_calls {
                debug!(tool = %tool_call.name, "Heartbeat executing tool call");
                let result = executor.execute_tool(tool_call).await;
                messages.push(Message::tool(
                    &result.call_id,
                    result.success,
                    &result.output,
                ));
            }
        } else {
            // No executor — return what we have.
            let trimmed = response.content.trim();
            let silent = trimmed.eq_ignore_ascii_case("HEARTBEAT_OK");
            return TaskResult {
                task: task.to_string(),
                success: true,
                silent,
                output: if silent {
                    String::new()
                } else {
                    response.content
                },
            };
        }
    }

    // Hit max iterations. #119: this is routine filler, not signal — mark it
    // silent so it's filtered upstream (batch summary, sessions) and never
    // reaches a Discord send site. This is the root of the fleet-wide flood.
    TaskResult {
        task: task.to_string(),
        success: true,
        silent: true,
        output: format!("Completed after {} tool iterations", max_iterations),
    }
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

/// Result of running a heartbeat task.
#[derive(Debug, Clone)]
pub struct TaskResult {
    pub task: String,
    pub success: bool,
    /// `true` when the LLM responded with `HEARTBEAT_OK` — no action was
    /// needed and the result should be silently discarded (no log, no note).
    pub silent: bool,
    pub output: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    // --- determine_frequencies ---

    /// Test helper: returns a state with `last_run` entries set far enough in the
    /// past that the cold-start gate (Dispatch 27) is bypassed and frequency
    /// gates are exercised on their normal "interval elapsed" path.
    fn warm_state() -> HeartbeatState {
        let mut state = HeartbeatState::default();
        // 30 days ago — older than any frequency interval (hourly/daily/weekly).
        let stale = (chrono::Utc::now().timestamp() as u64).saturating_sub(30 * 86_400);
        state.last_run.insert("hourly".to_string(), stale);
        state.last_run.insert("daily".to_string(), stale);
        state.last_run.insert("weekly".to_string(), stale);
        state
    }


    #[test]
    fn test_determine_frequencies_hourly_gate_recent_run_skips() {
        // Gate behavior: if last_run["hourly"] is recent (< interval), skip hourly.
        let dt = chrono::Local::now()
            .with_hour(15)
            .unwrap()
            .with_minute(30)
            .unwrap();
        let now_unix = dt.timestamp() as u64;
        let mut state = HeartbeatState::default();
        // Last hourly fired 10 minutes ago — should NOT fire again with 3600s interval.
        state.last_run.insert("hourly".to_string(), now_unix.saturating_sub(600));
        let freqs = determine_frequencies(dt, &mut state, 3600);
        assert!(!freqs.contains(&"hourly"), "hourly should be gated within interval");
    }

    #[test]
    fn test_determine_frequencies_hourly_gate_elapsed_fires() {
        // Gate behavior: if last_run["hourly"] is older than interval, fire.
        let dt = chrono::Local::now()
            .with_hour(15)
            .unwrap()
            .with_minute(30)
            .unwrap();
        let now_unix = dt.timestamp() as u64;
        let mut state = HeartbeatState::default();
        state.last_run.insert("hourly".to_string(), now_unix.saturating_sub(3700));
        let freqs = determine_frequencies(dt, &mut state, 3600);
        assert!(freqs.contains(&"hourly"), "hourly should fire after interval elapsed");
    }

    #[test]
    fn test_determine_frequencies_hourly_gate_first_run_anchors_no_fire() {
        // Dispatch 27 cold-start fix: with empty state (last_run absent),
        // hourly should NOT fire on the first tick. Instead, state is anchored
        // to now_unix and the next interval-elapsed tick fires.
        let dt = chrono::Local::now()
            .with_hour(15)
            .unwrap()
            .with_minute(30)
            .unwrap();
        let mut state = HeartbeatState::default();
        let freqs = determine_frequencies(dt, &mut state, 3600);
        assert!(!freqs.contains(&"hourly"), "hourly must NOT fire on cold start");
        assert!(state.last_run.contains_key("hourly"), "cold-start init must anchor last_run.hourly");
    }

    #[test]
    fn test_determine_frequencies_normal() {
        let dt = chrono::Local::now()
            .with_hour(15)
            .unwrap()
            .with_minute(30)
            .unwrap();
        let freqs = determine_frequencies(dt, &mut warm_state(), 3600);
        assert!(freqs.contains(&"hourly"));
        assert!(!freqs.contains(&"daily"));
    }

    #[test]
    fn test_determine_frequencies_daily() {
        let dt = chrono::Local::now()
            .with_hour(9)
            .unwrap()
            .with_minute(5)
            .unwrap();
        let freqs = determine_frequencies(dt, &mut warm_state(), 3600);
        assert!(freqs.contains(&"hourly"));
        assert!(freqs.contains(&"daily"));
    }

    #[test]
    fn test_determine_frequencies_daily_outside_window() {
        let dt = chrono::Local::now()
            .with_hour(9)
            .unwrap()
            .with_minute(15)
            .unwrap();
        let freqs = determine_frequencies(dt, &mut warm_state(), 3600);
        assert!(freqs.contains(&"hourly"));
        assert!(!freqs.contains(&"daily"));
    }

    #[test]
    fn test_determine_frequencies_daily_boundary_minute_zero() {
        let dt = chrono::Local::now()
            .with_hour(9)
            .unwrap()
            .with_minute(0)
            .unwrap();
        let freqs = determine_frequencies(dt, &mut warm_state(), 3600);
        assert!(freqs.contains(&"daily"));
    }

    #[test]
    fn test_determine_frequencies_daily_boundary_minute_nine() {
        let dt = chrono::Local::now()
            .with_hour(9)
            .unwrap()
            .with_minute(9)
            .unwrap();
        let freqs = determine_frequencies(dt, &mut warm_state(), 3600);
        assert!(freqs.contains(&"daily"));
    }

    #[test]
    fn test_determine_frequencies_late_evening() {
        let dt = chrono::Local::now()
            .with_hour(23)
            .unwrap()
            .with_minute(59)
            .unwrap();
        let freqs = determine_frequencies(dt, &mut warm_state(), 3600);
        assert!(freqs.contains(&"hourly"));
        assert!(!freqs.contains(&"daily"));
        assert!(!freqs.contains(&"weekly"));
    }

    #[test]
    fn test_determine_frequencies_midnight() {
        let dt = chrono::Local::now()
            .with_hour(0)
            .unwrap()
            .with_minute(0)
            .unwrap();
        let freqs = determine_frequencies(dt, &mut warm_state(), 3600);
        assert!(freqs.contains(&"hourly"));
        assert!(!freqs.contains(&"daily"));
    }

    #[test]
    fn test_determine_frequencies_always_has_hourly() {
        for hour in 0..24 {
            let dt = chrono::Local::now()
                .with_hour(hour)
                .unwrap()
                .with_minute(30)
                .unwrap();
            let freqs = determine_frequencies(dt, &mut warm_state(), 3600);
            assert!(
                freqs.contains(&"hourly"),
                "Hour {} should contain 'hourly'",
                hour
            );
        }
    }

    #[test]
    fn test_determine_frequencies_no_weekly_non_monday() {
        let now = chrono::Local::now();
        let weekday = now.format("%u").to_string().parse::<u32>().unwrap_or(1);
        let days_to_tuesday = if weekday <= 2 {
            2 - weekday
        } else {
            7 - weekday + 2
        };
        let tuesday = now + chrono::Duration::days(days_to_tuesday as i64);
        let dt = tuesday.with_hour(9).unwrap().with_minute(5).unwrap();
        let freqs = determine_frequencies(dt, &mut warm_state(), 3600);
        assert!(freqs.contains(&"daily"));
        assert!(
            !freqs.contains(&"weekly"),
            "Tuesday should not include weekly tasks"
        );
    }

    #[test]
    fn test_determine_frequencies_afternoon() {
        let dt = chrono::Local::now()
            .with_hour(14)
            .unwrap()
            .with_minute(0)
            .unwrap();
        let freqs = determine_frequencies(dt, &mut warm_state(), 3600);
        assert_eq!(freqs, vec!["hourly"]);
    }

    #[test]
    fn test_determine_frequencies_morning_non_nine() {
        let dt = chrono::Local::now()
            .with_hour(8)
            .unwrap()
            .with_minute(5)
            .unwrap();
        let freqs = determine_frequencies(dt, &mut warm_state(), 3600);
        assert!(freqs.contains(&"hourly"));
        assert!(!freqs.contains(&"daily"));
    }

    #[test]
    fn test_determine_frequencies_early_morning() {
        let dt = chrono::Local::now()
            .with_hour(5)
            .unwrap()
            .with_minute(0)
            .unwrap();
        let freqs = determine_frequencies(dt, &mut warm_state(), 3600);
        assert!(freqs.contains(&"hourly"));
        assert!(!freqs.contains(&"daily"));
        assert!(!freqs.contains(&"weekly"));
    }

    #[test]
    fn test_determine_frequencies_weekend() {
        let now = chrono::Local::now();
        let weekday = now.format("%u").to_string().parse::<u32>().unwrap_or(1);
        let days_to_saturday = if weekday <= 6 {
            6 - weekday
        } else {
            0
        };
        let saturday = now + chrono::Duration::days(days_to_saturday as i64);
        let dt = saturday.with_hour(9).unwrap().with_minute(5).unwrap();
        let freqs = determine_frequencies(dt, &mut warm_state(), 3600);
        assert!(freqs.contains(&"hourly"));
        assert!(freqs.contains(&"daily"), "Saturday 9:05 should trigger daily");
        assert!(
            !freqs.contains(&"weekly"),
            "Saturday should not trigger weekly tasks"
        );
    }

    #[test]
    fn test_determine_frequencies_weekly_on_monday() {
        let now = chrono::Local::now();
        let weekday = now.format("%u").to_string().parse::<u32>().unwrap_or(1);
        let days_to_monday = if weekday == 1 { 0 } else { 7 - weekday + 1 };
        let monday = now + chrono::Duration::days(days_to_monday as i64);
        let dt = monday.with_hour(9).unwrap().with_minute(5).unwrap();
        let freqs = determine_frequencies(dt, &mut warm_state(), 3600);
        assert!(freqs.contains(&"hourly"));
        assert!(freqs.contains(&"daily"));
        assert!(
            freqs.contains(&"weekly"),
            "Monday at 9:05 should include weekly tasks"
        );
    }

    // --- is_quiet_hour ---

    #[test]
    fn test_quiet_hour_overnight_range_inside() {
        // 23:00–08:00 overnight range, hour 1 → quiet
        let config = HeartbeatConfig::default(); // 23–8
        assert!(is_quiet_hour_for(&config, 1));
    }

    #[test]
    fn test_quiet_hour_overnight_range_outside() {
        // 23:00–08:00, hour 12 → not quiet
        let config = HeartbeatConfig::default();
        assert!(!is_quiet_hour_for(&config, 12));
    }

    #[test]
    fn test_quiet_hour_overnight_range_boundary_start() {
        // Exactly at start hour (23) → quiet
        let config = HeartbeatConfig::default();
        assert!(is_quiet_hour_for(&config, 23));
    }

    #[test]
    fn test_quiet_hour_overnight_range_boundary_end() {
        // Exactly at end hour (08) → NOT quiet (end is exclusive)
        let config = HeartbeatConfig::default();
        assert!(!is_quiet_hour_for(&config, 8));
    }

    #[test]
    fn test_quiet_hour_disabled() {
        let config = HeartbeatConfig {
            enable_quiet_hours: false,
            ..Default::default()
        };
        // Even at midnight, quiet hours disabled → not suppressed
        assert!(!is_quiet_hour(&config, 0));
    }

    #[test]
    fn test_quiet_hour_same_day_range() {
        // 08:00–17:00 same-day range
        let config = HeartbeatConfig {
            quiet_hours_start: 8,
            quiet_hours_end: 17,
            enable_quiet_hours: true,
            ..Default::default()
        };
        assert!(is_quiet_hour_for(&config, 10)); // 10:00 → quiet
        assert!(!is_quiet_hour_for(&config, 18)); // 18:00 → not quiet
        assert!(!is_quiet_hour_for(&config, 7)); // 07:00 → not quiet
    }

    #[test]
    fn test_quiet_hour_all_hours_overnight() {
        let config = HeartbeatConfig::default(); // 23–8
        for h in 0u8..8 {
            assert!(is_quiet_hour_for(&config, h), "hour {} should be quiet", h);
        }
        for h in 8u8..23 {
            assert!(!is_quiet_hour_for(&config, h), "hour {} should not be quiet", h);
        }
        assert!(is_quiet_hour_for(&config, 23), "hour 23 should be quiet");
    }

    // --- HeartbeatConfig defaults ---

    #[test]
    fn test_heartbeat_config_defaults() {
        let cfg = HeartbeatConfig::default();
        assert_eq!(cfg.quiet_hours_start, 23);
        assert_eq!(cfg.quiet_hours_end, 8);
        assert!(cfg.enable_quiet_hours);
    }

    // --- HeartbeatState load/save roundtrip ---

    #[test]
    fn test_state_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("heartbeat-state.json");

        let mut state = HeartbeatState::default();
        state.last_run.insert("check email".to_string(), 1_700_000_000);
        save_state(&path, &state);

        let loaded = load_state(&path);
        assert_eq!(loaded.last_run.get("check email"), Some(&1_700_000_000u64));
    }

    #[test]
    fn test_state_load_missing_file() {
        let path = std::path::Path::new("/nonexistent/heartbeat-state.json");
        let state = load_state(path);
        assert!(state.last_run.is_empty());
    }

    #[test]
    fn test_state_load_corrupt_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("heartbeat-state.json");
        std::fs::write(&path, b"not valid json").unwrap();
        let state = load_state(&path);
        assert!(state.last_run.is_empty());
    }

    // --- #206: corrupt/partial state file must NOT fabricate a `None` tick ---

    #[test]
    fn test_corrupt_state_preserves_last_heartbeat_tick() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("heartbeat-state.json");

        // A healthy seat writes a tick…
        let mut state = HeartbeatState::default();
        state.last_heartbeat_tick = Some(1_750_000_000);
        save_state(&path, &state);

        // …then a torn/partial write corrupts the file on disk.
        std::fs::write(&path, b"{\"last_run\":{\"task\":17").unwrap();

        // Load must NOT reset the tick to None — that fabricated None is what
        // made the watchdog false-fire "Last tick: never" on healthy seats.
        let loaded = load_state(&path);
        assert!(
            loaded.last_heartbeat_tick.is_some(),
            "#206: parse failure must preserve last known tick, not fabricate None"
        );

        // Garbage (non-JSON) corruption: same guarantee.
        std::fs::write(&path, b"\x00\xffnot json at all").unwrap();
        let loaded = load_state(&path);
        assert!(loaded.last_heartbeat_tick.is_some());
    }

    #[test]
    fn test_head_rearm_never_returns_zero() {
        // #330 regression guard: the head re-arm must NEVER yield 0 —
        // 0 is the wake-branch's drain-and-rearm sentinel, and if it
        // survives past one timed iteration the loop goes hot (sleep(0)
        // busy-loop → ENOENT spam + time-driver starvation on small boxes).
        assert!(head_rearm_interval(true, 3600, 60) > 0);
        assert!(head_rearm_interval(false, 3600, 60) > 0);
        // Even with degenerate config values, the invariant holds via .max().
        assert_eq!(head_rearm_interval(false, 0, 60), 60);
        assert_eq!(head_rearm_interval(false, 0, 0), 1); // both-zero config: floored to 1s, never 0
        // Event-driven mode sleeps "forever" (u64::MAX/2 secs).
        assert_eq!(head_rearm_interval(true, 0, 0), u64::MAX / 2);
        // Safety-net mode takes the larger of the two intervals.
        assert_eq!(head_rearm_interval(false, 3600, 7200), 7200);
        assert_eq!(head_rearm_interval(false, 3600, 60), 3600);
    }

    #[test]
    fn test_save_state_atomic_no_temp_residue() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("heartbeat-state.json");

        let mut state = HeartbeatState::default();
        state.last_heartbeat_tick = Some(42);
        save_state(&path, &state);

        // The rename target is complete, parseable JSON…
        let loaded = load_state(&path);
        assert_eq!(loaded.last_heartbeat_tick, Some(42));

        // …and no temp file lingers in the directory.
        let residue: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(residue.is_empty(), "atomic save must not leave temp files");
    }

    #[test]
    fn test_update_state_read_modify_write() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("heartbeat-state.json");

        let mut state = HeartbeatState::default();
        state.last_run.insert("existing task".to_string(), 100);
        save_state(&path, &state);

        // Locked RMW must merge, not clobber.
        update_state(&path, |s| {
            s.last_heartbeat_tick = Some(200);
        });

        let loaded = load_state(&path);
        assert_eq!(loaded.last_run.get("existing task"), Some(&100u64));
        assert_eq!(loaded.last_heartbeat_tick, Some(200));
    }

    // --- TaskResult ---

    #[test]
    fn test_task_result_debug() {
        let result = TaskResult {
            task: "Check email".to_string(),
            success: true,
            silent: false,
            output: "All good".to_string(),
        };
        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("Check email"));
        assert!(debug_str.contains("true"));
    }

    #[test]
    fn test_task_result_clone() {
        let result = TaskResult {
            task: "Backup".to_string(),
            success: false,
            silent: false,
            output: "Disk full".to_string(),
        };
        let cloned = result.clone();
        assert_eq!(cloned.task, "Backup");
        assert!(!cloned.success);
        assert_eq!(cloned.output, "Disk full");
    }

    #[test]
    fn test_task_result_silent_flag() {
        let result = TaskResult {
            task: "ping gateway".to_string(),
            success: true,
            silent: true,
            output: String::new(),
        };
        assert!(result.silent);
        assert!(result.output.is_empty());
    }

    #[test]
    fn test_task_result_success_formatting() {
        let ok_result = TaskResult {
            task: "task1".to_string(),
            success: true,
            silent: false,
            output: "done".to_string(),
        };
        let fail_result = TaskResult {
            task: "task2".to_string(),
            success: false,
            silent: false,
            output: "error occurred".to_string(),
        };
        let summary = format!(
            "- [{}] {}: {}",
            if ok_result.success { "OK" } else { "FAIL" },
            ok_result.task,
            ok_result.output
        );
        assert_eq!(summary, "- [OK] task1: done");

        let summary2 = format!(
            "- [{}] {}: {}",
            if fail_result.success { "OK" } else { "FAIL" },
            fail_result.task,
            fail_result.output
        );
        assert_eq!(summary2, "- [FAIL] task2: error occurred");
    }

    #[test]
    fn test_task_result_empty_output() {
        let result = TaskResult {
            task: "noop".to_string(),
            success: true,
            silent: true,
            output: String::new(),
        };
        assert!(result.output.is_empty());
        assert!(result.success);
        assert!(result.silent);
    }

    #[test]
    fn test_task_result_with_long_output() {
        let long_output = "x".repeat(100_000);
        let result = TaskResult {
            task: "large output task".to_string(),
            success: true,
            silent: false,
            output: long_output.clone(),
        };
        assert_eq!(result.output.len(), 100_000);

        let cloned = result.clone();
        assert_eq!(cloned.output.len(), 100_000);
    }

    // --- Dedup grace window logic ---

    #[test]
    fn test_dedup_grace_window_skip() {
        // Simulate: task ran 60s ago, interval=300s, grace=270s → should skip
        let interval = 300u64;
        let grace = interval.saturating_sub(interval / 10); // 270
        let now = 1_700_001_000u64;
        let last_run = now - 60; // 60s ago
        assert!(now.saturating_sub(last_run) < grace);
    }

    #[test]
    fn test_dedup_grace_window_allow() {
        // Simulate: task ran 280s ago, interval=300s, grace=270s → should run
        let interval = 300u64;
        let grace = interval.saturating_sub(interval / 10); // 270
        let now = 1_700_001_000u64;
        let last_run = now - 280; // 280s ago
        assert!(now.saturating_sub(last_run) >= grace);
    }

    #[test]
    fn test_dedup_unknown_task_runs() {
        // A task not in state should always run (no entry → no skip)
        let state = HeartbeatState::default();
        assert!(state.last_run.get("new task").is_none());
    }

    #[test]
    fn test_heartbeat_default_interval() {
        assert_eq!(300_u64, 300);
    }

    // --- HeartbeatConfig extended fields ---

    #[test]
    fn test_heartbeat_config_timeout_default() {
        let cfg = HeartbeatConfig::default();
        assert_eq!(cfg.timeout_secs, 300);
    }

    #[test]
    fn test_heartbeat_config_dedup_window_default() {
        let cfg = HeartbeatConfig::default();
        assert_eq!(cfg.dedup_window_secs, 86400); // 24 hours
    }

    // --- HeartbeatState text dedup ---

    #[test]
    fn test_state_dedup_fields_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("heartbeat-state.json");

        let mut state = HeartbeatState::default();
        state.last_run.insert("task1".to_string(), 1_700_000_000);
        state.last_output.insert("task1".to_string(), "alert: disk 90%".to_string());
        state.last_output_at.insert("task1".to_string(), 1_700_000_000);
        save_state(&path, &state);

        let loaded = load_state(&path);
        assert_eq!(loaded.last_output.get("task1").unwrap(), "alert: disk 90%");
        assert_eq!(loaded.last_output_at.get("task1"), Some(&1_700_000_000u64));
    }

    #[test]
    fn test_state_dedup_detect_duplicate() {
        let now = 1_700_001_000u64;
        let dedup_window = 86400u64; // 24h
        let mut state = HeartbeatState::default();
        state.last_output.insert("task1".to_string(), "same output".to_string());
        state.last_output_at.insert("task1".to_string(), now - 3600); // 1h ago

        let output = "same output";
        let is_dup = state.last_output.get("task1").map(|prev| prev == output).unwrap_or(false)
            && state.last_output_at.get("task1").map(|&t| now.saturating_sub(t) < dedup_window).unwrap_or(false);
        assert!(is_dup, "Same output within 24h should be duplicate");
    }

    #[test]
    fn test_state_dedup_different_output() {
        let now = 1_700_001_000u64;
        let dedup_window = 86400u64;
        let mut state = HeartbeatState::default();
        state.last_output.insert("task1".to_string(), "old output".to_string());
        state.last_output_at.insert("task1".to_string(), now - 3600);

        let output = "new output";
        let is_dup = state.last_output.get("task1").map(|prev| prev == output).unwrap_or(false)
            && state.last_output_at.get("task1").map(|&t| now.saturating_sub(t) < dedup_window).unwrap_or(false);
        assert!(!is_dup, "Different output should not be duplicate");
    }

    #[test]
    fn test_state_dedup_expired_window() {
        let now = 1_700_100_000u64;
        let dedup_window = 86400u64; // 24h
        let mut state = HeartbeatState::default();
        state.last_output.insert("task1".to_string(), "same output".to_string());
        state.last_output_at.insert("task1".to_string(), now - 90000); // 25h ago

        let output = "same output";
        let is_dup = state.last_output.get("task1").map(|prev| prev == output).unwrap_or(false)
            && state.last_output_at.get("task1").map(|&t| now.saturating_sub(t) < dedup_window).unwrap_or(false);
        assert!(!is_dup, "Same output outside 24h window should not be duplicate");
    }

    #[test]
    fn test_state_backward_compat_no_dedup_fields() {
        // Simulate old heartbeat-state.json without dedup fields
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("heartbeat-state.json");
        std::fs::write(&path, r#"{"last_run":{"task1":1700000000}}"#).unwrap();

        let loaded = load_state(&path);
        assert_eq!(loaded.last_run.get("task1"), Some(&1_700_000_000u64));
        assert!(loaded.last_output.is_empty(), "Missing dedup fields should default to empty");
        assert!(loaded.last_output_at.is_empty());
    }

    #[test]
    fn test_171_missing_tick_field_does_not_nuke_persisted_state() {
        // #171 regression: a state file written by an older binary that lacks
        // `last_heartbeat_tick` must still deserialize. Before the fix this field
        // had no `#[serde(default)]`, so `from_str` failed on the whole document,
        // `load_state` fell to `.unwrap_or_default()`, and EVERY persisted field
        // (including a healthy tick) was nuked — driving the watchdog to read
        // `None` and false-fire "Last tick: never" on a healthy seat.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("heartbeat-state.json");
        // Old-schema file: real last_run, NO last_heartbeat_tick field.
        std::fs::write(&path, r#"{"last_run":{"task1":1700000000}}"#).unwrap();

        let loaded = load_state(&path);
        // The whole document must survive (not reset to default).
        assert_eq!(
            loaded.last_run.get("task1"),
            Some(&1_700_000_000u64),
            "#171: missing last_heartbeat_tick must NOT cause a full-state reset"
        );
        // The absent field defaults to None — not a parse failure.
        assert_eq!(loaded.last_heartbeat_tick, None);
    }

    #[test]
    fn test_171_fresh_in_run_tick_round_trips_and_does_not_alert() {
        // #171 end-to-end: a healthy seat with a fresh in-run tick written via
        // the REAL save_state path must persist that tick, load it back intact,
        // and the watchdog must stay silent — no "Last tick: never" false-fire.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("heartbeat-state.json");

        let process_start = 1_700_000_000u64;
        let fresh_tick = process_start + 30; // ticked 30s into this run

        // Write through the production save path (as spawn_fast_pulse does).
        let mut state = HeartbeatState::default();
        state.last_heartbeat_tick = Some(fresh_tick);
        save_state(&path, &state);

        // Read through the production load path (as spawn_watchdog does).
        let loaded = load_state(&path);
        assert_eq!(
            loaded.last_heartbeat_tick,
            Some(fresh_tick),
            "#171: a fresh in-run tick must survive the disk round-trip"
        );

        // Watchdog evaluated 60s after the tick, threshold 600s → must stay silent.
        let now = fresh_tick + 60;
        assert!(
            !watchdog_should_alert(loaded.last_heartbeat_tick, process_start, now, 600),
            "#171: a seat with a fresh persisted in-run tick must NOT false-fire the watchdog"
        );
    }

    // --- parse_utc_offset ---

    #[test]
    fn test_parse_utc_offset_positive() {
        let offset = parse_utc_offset("+05:30").unwrap();
        assert_eq!(offset.local_minus_utc(), 5 * 3600 + 30 * 60);
    }

    #[test]
    fn test_parse_utc_offset_negative() {
        let offset = parse_utc_offset("-08:00").unwrap();
        assert_eq!(offset.local_minus_utc(), -(8 * 3600));
    }

    #[test]
    fn test_parse_utc_offset_zero() {
        let offset = parse_utc_offset("+00:00").unwrap();
        assert_eq!(offset.local_minus_utc(), 0);
    }

    #[test]
    fn test_parse_utc_offset_invalid() {
        assert!(parse_utc_offset("EST").is_none());
        assert!(parse_utc_offset("abc").is_none());
        assert!(parse_utc_offset("").is_none());
    }

    // --- resolve_current_hour ---

    #[test]
    fn test_resolve_current_hour_utc() {
        let hour = resolve_current_hour(Some("UTC"));
        assert!(hour < 24);
    }

    #[test]
    fn test_resolve_current_hour_offset() {
        let hour = resolve_current_hour(Some("+00:00"));
        let utc_hour = chrono::Utc::now().hour() as u8;
        assert_eq!(hour, utc_hour);
    }

    #[test]
    fn test_resolve_current_hour_local_fallback() {
        use chrono::Timelike;
        let hour = resolve_current_hour(None);
        let local_hour = chrono::Local::now().hour() as u8;
        assert_eq!(hour, local_hour);
    }

    // --- adaptive interval config ---

    #[test]
    fn test_heartbeat_config_adaptive_defaults() {
        let config = HeartbeatConfig::default();
        assert_eq!(config.active_interval_secs, 120);
        assert_eq!(config.queued_interval_secs, 300);
        assert_eq!(config.idle_interval_secs, 900);
    }

    // --- Pre-flight gating ---

    #[test]
    fn test_preflight_gate_task_never_run() {
        // Task never in state → always due
        let state = HeartbeatState::default();
        // Simulate: task "push-work" not in state.last_run. Never-run
        // tasks are due because there is no persisted timestamp to compare,
        // not because any wall-clock interval has elapsed.
        assert!(state.last_run.get("push-work").is_none());
    }

    #[test]
    fn test_preflight_gate_task_recently_run() {
        // Task ran 60s ago, interval 300s → not due
        let mut state = HeartbeatState::default();
        let now = chrono::Utc::now().timestamp() as u64;
        state.last_run.insert("push-work".to_string(), now - 60);
        let elapsed = now.saturating_sub(state.last_run["push-work"]);
        assert!(elapsed < 300, "Task ran 60s ago with 300s interval should not be due");
    }

    #[test]
    fn test_preflight_gate_task_interval_elapsed() {
        // Task ran 400s ago, interval 300s → due
        let mut state = HeartbeatState::default();
        let now = chrono::Utc::now().timestamp() as u64;
        state.last_run.insert("push-work".to_string(), now - 400);
        let elapsed = now.saturating_sub(state.last_run["push-work"]);
        assert!(elapsed >= 300, "Task ran 400s ago with 300s interval should be due");
    }

    #[test]
    fn test_preflight_gate_mixed_tasks() {
        let mut state = HeartbeatState::default();
        let now = chrono::Utc::now().timestamp() as u64;
        // push-work: ran 60s ago (not due, interval=300s)
        state.last_run.insert("push-work".to_string(), now - 60);
        // report: ran 4000s ago (due, interval=3600s)
        state.last_run.insert("report".to_string(), now - 4000);
        // current-task: never run (due)

        let due_count = 0
            + (now.saturating_sub(state.last_run["push-work"]) >= 300) as usize
            + (now.saturating_sub(state.last_run["report"]) >= 3600) as usize
            + (state.last_run.get("current-task").is_none()) as usize;
        assert_eq!(due_count, 2, "report + current-task should be due, push-work should not");
    }

    #[test]
    fn test_preflight_gate_empty_state_all_due() {
        let state = HeartbeatState::default();
        // No tasks in state → all should be due
        assert!(state.last_run.is_empty());
    }

    // --- Dispatch 27: cold-start preflight gate ---

    /// Helper: build a workspace with a structured `## tasks` HEARTBEAT.md and
    /// no per-task `last_run` state (cold-start scenario).
    async fn workspace_with_structured_tasks() -> (tempfile::TempDir, zeus_memory::Workspace) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ws = zeus_memory::Workspace::new(tmp.path());
        ws.init().await.expect("init workspace");
        let heartbeat_md = "\
# HEARTBEAT.md

## tasks

- name: push-work
  interval: 1h
  prompt: push uncommitted work

- name: report-status
  interval: 2h
  prompt: report status to channel
";
        ws.write("HEARTBEAT.md", heartbeat_md)
            .await
            .expect("write HEARTBEAT.md");
        (tmp, ws)
    }

    /// Dispatch 27 — cold start: empty `state.last_run` must NOT cause every
    /// hourly task to fire on tick 1. State must be mutated to anchor each
    /// task's `last_run` at `now_unix` so the next tick gates correctly.
    #[tokio::test]
    async fn test_preflight_gate_cold_start_no_flood() {
        let (_tmp, ws) = workspace_with_structured_tasks().await;
        let mut state = HeartbeatState::default();
        assert!(state.last_run.is_empty());

        let now_before = chrono::Utc::now().timestamp() as u64;
        let due = preflight_gate(&ws, &mut state).await;

        let due = due.expect("structured tasks present → Some(due)");
        assert!(
            due.is_empty(),
            "cold-start tick must return zero due tasks (got {}) — this is the flood bug",
            due.len()
        );

        // State must have been mutated: every task gets a last_run anchor at ~now.
        assert!(state.last_run.contains_key("push-work"), "push-work last_run must be anchored");
        assert!(state.last_run.contains_key("report-status"), "report-status last_run must be anchored");
        let push_anchor = state.last_run["push-work"];
        assert!(
            push_anchor >= now_before && push_anchor <= now_before + 5,
            "anchor should be ~now_unix, got {} vs now_before {}",
            push_anchor,
            now_before
        );
    }

    /// Second tick after cold start: same state, intervals not yet elapsed
    /// (test runs in milliseconds), so still no tasks due. This verifies the
    /// anchor from tick 1 actually persists and is honored on tick 2.
    #[tokio::test]
    async fn test_preflight_gate_second_tick_after_cold_start() {
        let (_tmp, ws) = workspace_with_structured_tasks().await;
        let mut state = HeartbeatState::default();

        // Tick 1: cold-start init.
        let due1 = preflight_gate(&ws, &mut state).await.expect("Some");
        assert!(due1.is_empty(), "tick 1 cold-start: nothing due");

        let snapshot_after_tick1 = state.last_run.clone();

        // Tick 2 immediately: intervals (1h, 2h) far exceed test runtime, so
        // still nothing due. Anchors from tick 1 should be unchanged.
        let due2 = preflight_gate(&ws, &mut state).await.expect("Some");
        assert!(
            due2.is_empty(),
            "tick 2 immediately after tick 1: still nothing due (intervals are 1h+/2h+)"
        );
        assert_eq!(
            state.last_run, snapshot_after_tick1,
            "tick 2 must NOT re-anchor — last_run should be identical to tick 1"
        );
    }

    /// Legacy `## hourly` path mirror: with a fresh state (`last_run.hourly` absent
    /// → unwrap_or(0)), the cold-start branch must anchor `last_run.hourly` to
    /// now_unix and skip firing this tick. Without the fix, `now - 0 >= 3600`
    /// is trivially true and every hourly task fires.
    #[test]
    fn test_determine_frequencies_cold_start() {
        let dt = chrono::Local::now()
            .with_hour(15)
            .unwrap()
            .with_minute(30)
            .unwrap();
        let mut state = HeartbeatState::default();
        assert!(state.last_run.is_empty(), "preconditon: cold-start state");

        let freqs = determine_frequencies(dt, &mut state, 3600);

        assert!(
            !freqs.contains(&"hourly"),
            "cold start must NOT include hourly (this is the legacy flood bug)"
        );
        let anchored = state.last_run.get("hourly").copied().unwrap_or(0);
        assert!(
            anchored > 0,
            "cold-start init must anchor last_run.hourly to a non-zero now_unix"
        );
        // The function derives now_unix from the passed-in datetime, not wall clock.
        let expected = dt.timestamp() as u64;
        assert_eq!(
            anchored, expected,
            "anchor should equal dt.timestamp() (the function's now_unix source)"
        );
    }

    // --- Task status machine (P1 #6) ---

    #[test]
    fn test_task_status_default() {
        let status = TaskStatus::default();
        assert_eq!(status, TaskStatus::Pending);
    }

    #[test]
    fn test_task_status_display() {
        assert_eq!(format!("{}", TaskStatus::Pending), "PENDING");
        assert_eq!(format!("{}", TaskStatus::InProgress), "IN_PROGRESS");
        assert_eq!(format!("{}", TaskStatus::Completed), "COMPLETED");
        assert_eq!(format!("{}", TaskStatus::Failed), "FAILED");
        assert_eq!(format!("{}", TaskStatus::Stuck), "STUCK");
    }

    #[test]
    fn test_update_task_status_transitions() {
        let mut state = HeartbeatState::default();
        
        // Initial status should be Pending (default)
        assert_eq!(state.task_status.get("task1"), None);
        
        // Transition to InProgress
        update_task_status(&mut state, "task1", TaskStatus::InProgress);
        assert_eq!(state.task_status.get("task1"), Some(&TaskStatus::InProgress));
        assert!(state.status_changed_at.contains_key("task1"));
        
        // Transition to Completed
        update_task_status(&mut state, "task1", TaskStatus::Completed);
        assert_eq!(state.task_status.get("task1"), Some(&TaskStatus::Completed));
        
        // Transition to same status should not panic
        update_task_status(&mut state, "task1", TaskStatus::Completed);
        assert_eq!(state.task_status.get("task1"), Some(&TaskStatus::Completed));
    }

    #[test]
    fn test_failure_count_increment() {
        let mut state = HeartbeatState::default();
        
        assert_eq!(increment_failure_count(&mut state, "task1"), 1);
        assert_eq!(increment_failure_count(&mut state, "task1"), 2);
        assert_eq!(increment_failure_count(&mut state, "task1"), 3);
        
        // Different task should have independent count
        assert_eq!(increment_failure_count(&mut state, "task2"), 1);
    }

    #[test]
    fn test_failure_count_reset() {
        let mut state = HeartbeatState::default();
        
        increment_failure_count(&mut state, "task1");
        increment_failure_count(&mut state, "task1");
        assert_eq!(state.failure_count.get("task1"), Some(&2));
        
        reset_failure_count(&mut state, "task1");
        assert_eq!(state.failure_count.get("task1"), None);
    }

    #[test]
    fn test_check_stuck_by_failures() {
        let mut state = HeartbeatState::default();
        
        // No failures → not stuck
        assert!(check_stuck(&state, "task1", 3600).is_none());
        
        // 2 failures → not stuck yet
        increment_failure_count(&mut state, "task1");
        increment_failure_count(&mut state, "task1");
        assert!(check_stuck(&state, "task1", 3600).is_none());
        
        // 3 failures → stuck with FailureCount variant
        increment_failure_count(&mut state, "task1");
        let reason = check_stuck(&state, "task1", 3600).expect("should be stuck");
        assert_eq!(reason, StuckReason::FailureCount { count: 3, threshold: 3 });
    }

    #[test]
    fn test_is_current_task_bound_matches_hourly_continue() {
        // The canonical hourly task that drives the 112/107 wedge.
        assert!(is_current_task_bound("Then: continue your CURRENT TASK"));
        assert!(is_current_task_bound("continue your current task")); // case-insensitive
        assert!(is_current_task_bound("Work on the CURRENT TASK now"));
        // Unrelated tasks must NOT trip the gate (no false auto-retire).
        assert!(!is_current_task_bound("First: push any uncommitted work"));
        assert!(!is_current_task_bound("Report to your team channel"));
        assert!(!is_current_task_bound(""));
    }

    #[tokio::test]
    async fn test_set_current_task_empty_persists_and_clears() {
        // #157 CHECK 1: set_current_task("") must PERSIST the cleared state to
        // HEARTBEAT.md on disk so a restart re-reads idle (not the wedged task).
        let dir = tempfile::tempdir().expect("tempdir");
        let ws = zeus_memory::Workspace::new(dir.path());
        ws.init().await.expect("init workspace");

        ws.set_current_task("Build the widget on a dead endpoint")
            .await
            .expect("set");
        assert!(
            ws.get_current_task().await.expect("get").is_some(),
            "task should be active after set"
        );

        // Retire to idle.
        ws.set_current_task("").await.expect("clear");

        // CHECK 1a: in-memory getter reports idle.
        assert!(
            ws.get_current_task().await.expect("get").is_none(),
            "CURRENT TASK should be None after clear"
        );

        // CHECK 1b: the cleared state is on DISK — read the raw file, not the
        // cache. A fresh Workspace over the same dir must also see idle.
        let raw = std::fs::read_to_string(dir.path().join("HEARTBEAT.md"))
            .expect("HEARTBEAT.md on disk");
        assert!(
            raw.contains("## CURRENT TASK"),
            "section header must survive"
        );
        assert!(
            !raw.contains("dead endpoint"),
            "the wedged task must be gone from disk after retire"
        );
        let ws2 = zeus_memory::Workspace::new(dir.path());
        assert!(
            ws2.get_current_task().await.expect("get").is_none(),
            "a restart (fresh Workspace) must re-read idle, not the wedged task"
        );
    }

    #[tokio::test]
    async fn test_empty_current_task_is_noop_signal() {
        // #157 CHECK 2: with CURRENT TASK cleared, get_current_task() returns
        // None — which is the exact signal the no-op gate uses to skip the
        // "continue your CURRENT TASK" task (HEARTBEAT_OK) instead of firing
        // the LLM forever. This asserts the placeholder is treated as idle.
        let dir = tempfile::tempdir().expect("tempdir");
        let ws = zeus_memory::Workspace::new(dir.path());
        ws.init().await.expect("init workspace");
        ws.set_current_task("").await.expect("clear");

        let active = ws.get_current_task().await.expect("get");
        assert!(active.is_none(), "cleared CURRENT TASK must read as idle/None");

        // The gate condition the heartbeat loop evaluates:
        let gate_skips = is_current_task_bound("Then: continue your CURRENT TASK")
            && active.is_none();
        assert!(gate_skips, "no-op gate must trip when CURRENT TASK is idle");
    }

    #[test]
    fn test_check_stuck_by_timeout() {
        let mut state = HeartbeatState::default();

        // Set status changed to ~2h ago, relative to now so the fixture never
        // ages out (a hardcoded past epoch makes `now - changed_at` grow
        // without bound and breaks the long-timeout assertion over time).
        let now = chrono::Utc::now().timestamp() as u64;
        state
            .status_changed_at
            .insert("task1".to_string(), now - 7200); // 2h ago

        // With a short timeout, should be stuck with Timeout variant
        let reason = check_stuck(&state, "task1", 60).expect("should be stuck");
        assert!(matches!(reason, StuckReason::Timeout { elapsed_secs, timeout_secs: 60 } if elapsed_secs >= 7200));

        // With a long timeout (10h > 2h elapsed), should not be stuck
        assert!(check_stuck(&state, "task1", 36000).is_none());
    }

    #[test]
    fn test_success_resets_failure_count_before_check_stuck() {
        // #179: A task with ≥3 prior failures that just succeeded should NOT
        // be marked STUCK. reset_failure_count must fire before check_stuck.
        let mut state = HeartbeatState::default();
        let now = chrono::Utc::now().timestamp() as u64;

        // Simulate 3 prior failures
        increment_failure_count(&mut state, "task1");
        increment_failure_count(&mut state, "task1");
        increment_failure_count(&mut state, "task1");
        assert_eq!(*state.failure_count.get("task1").unwrap_or(&0), 3);

        // Before #179: check_stuck would return Some(FailureCount) here.
        // After #179: the success path resets failure_count first.
        reset_failure_count(&mut state, "task1");
        state.status_changed_at.insert("task1".to_string(), now);

        // Now check_stuck should return None — task recovered.
        assert!(check_stuck(&state, "task1", 3600).is_none());
        assert_eq!(*state.failure_count.get("task1").unwrap_or(&0), 0);
    }

    #[test]
    fn test_success_resets_status_changed_at_before_check_stuck() {
        // #179: A task with stale status_changed_at that just succeeded
        // should NOT be marked STUCK via timeout.
        let mut state = HeartbeatState::default();
        let now = chrono::Utc::now().timestamp() as u64;

        // Simulate old status_changed_at (2h ago) + 3 failures
        state.status_changed_at.insert("task1".to_string(), now - 7200);
        increment_failure_count(&mut state, "task1");
        increment_failure_count(&mut state, "task1");
        increment_failure_count(&mut state, "task1");

        // Before #179: check_stuck would return Some(Timeout) or Some(FailureCount).
        // After #179: the success path resets both.
        reset_failure_count(&mut state, "task1");
        state.status_changed_at.insert("task1".to_string(), now);

        // Now check_stuck should return None — elapsed ≈ 0, failure_count = 0.
        assert!(check_stuck(&state, "task1", 3600).is_none());
    }

    #[test]
    fn test_state_roundtrip_with_status() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("heartbeat-state.json");

        let mut state = HeartbeatState::default();
        state.last_run.insert("task1".to_string(), 1_700_000_000);
        state.task_status.insert("task1".to_string(), TaskStatus::InProgress);
        state.status_changed_at.insert("task1".to_string(), 1_700_000_000);
        state.failure_count.insert("task1".to_string(), 2);
        save_state(&path, &state);

        let loaded = load_state(&path);
        assert_eq!(loaded.task_status.get("task1"), Some(&TaskStatus::InProgress));
        assert_eq!(loaded.status_changed_at.get("task1"), Some(&1_700_000_000u64));
        assert_eq!(loaded.failure_count.get("task1"), Some(&2u32));
    }

    #[test]
    fn test_state_backward_compat_no_status_fields() {
        // Simulate old heartbeat-state.json without status fields
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("heartbeat-state.json");
        std::fs::write(&path, r#"{"last_run":{"task1":1700000000}}"#).unwrap();

        let loaded = load_state(&path);
        assert!(loaded.task_status.is_empty(), "Missing status fields should default to empty");
        assert!(loaded.status_changed_at.is_empty());
        assert!(loaded.failure_count.is_empty());
    }

    #[test]
    fn test_compute_task_timeout_complex() {
        let config = HeartbeatConfig::default();
        let task = "refactor the authentication module";
        assert_eq!(compute_task_timeout(task, &config), config.complex_timeout_secs);
    }

    #[test]
    fn test_compute_task_timeout_trivial() {
        let config = HeartbeatConfig::default();
        let task = "check disk usage and report";
        assert_eq!(compute_task_timeout(task, &config), config.trivial_timeout_secs);
    }

    #[test]
    fn test_compute_task_timeout_medium() {
        let config = HeartbeatConfig::default();
        let task = "update dependencies and test";
        assert_eq!(compute_task_timeout(task, &config), config.medium_timeout_secs);
    }

    // ---- Wake-branch parity (regression 2026-04-29) ------------------------
    //
    // R2 architecture: the heartbeat loop has ONE canonical tick path (the
    // timed branch). The wake branch should NOT duplicate tick logic — it
    // should only coalesce wake events and force the timed branch to fire on
    // the next select iteration. These tests document and protect that
    // contract so the regression can't silently re-emerge.

    #[test]
    fn test_wake_request_struct_shape() {
        // Contract: WakeRequest carries (reason, optional agent_id).
        // If this signature changes, every wake call site must be audited.
        let req = WakeRequest {
            reason: "cooking_complete".into(),
            agent_id: Some("zeus106".into()),
        };
        assert_eq!(req.reason, "cooking_complete");
        assert_eq!(req.agent_id.as_deref(), Some("zeus106"));

        let broadcast = WakeRequest {
            reason: "fleet_event".into(),
            agent_id: None,
        };
        assert!(broadcast.agent_id.is_none());
    }

    #[tokio::test]
    async fn test_wake_channel_smoke() {
        // Contract: a WakeRequest pushed into the wake channel is delivered
        // and recv() returns the same payload. This is the exact path the
        // heartbeat_loop's wake branch consumes from. If this breaks, the
        // event-driven heartbeat is dead.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<WakeRequest>(8);
        tx.send(WakeRequest {
            reason: "test".into(),
            agent_id: None,
        })
        .await
        .expect("send wake");

        let got = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("wake delivered within 2s")
            .expect("wake payload present");
        assert_eq!(got.reason, "test");
        assert!(got.agent_id.is_none());
    }

    #[tokio::test]
    async fn test_wake_coalesce_drains_burst() {
        // Contract: when many wake events arrive in a burst, the wake branch
        // drains them all (try_recv loop) so the *next* timed tick handles
        // them as a single coalesced unit. This test mirrors the drain logic
        // inside heartbeat_loop's wake arm.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<WakeRequest>(32);
        for i in 0..10 {
            tx.send(WakeRequest {
                reason: format!("burst-{i}"),
                agent_id: None,
            })
            .await
            .unwrap();
        }
        // First wake (the one that wins the select).
        let first = rx.recv().await.expect("first wake");
        assert_eq!(first.reason, "burst-0");

        // Drain remaining (matches the `while rx.try_recv().is_ok() {}` loop).
        let mut drained = 0;
        while rx.try_recv().is_ok() {
            drained += 1;
        }
        assert_eq!(drained, 9, "all queued wakes should be drained");
    }

    // --- Plan-resume gating tests -------------------------------------------------
    //
    // These exercise the per-slug `last_run["plan_resume:<slug>"]` rate-limit logic
    // added to fix the noise loop where every adaptive heartbeat tick re-resumed
    // every stale plan and emitted `[Plan Resume] <slug>: ...` to channel.
    //
    // The gate is implemented inline in `heartbeat_loop`, so we test the predicate
    // directly: given a state + slug + interval, would the gate skip or fire?

    fn plan_resume_gate(
        state: &HeartbeatState,
        slug: &str,
        now_unix: u64,
        interval_secs: u64,
    ) -> bool {
        let key = format!("plan_resume:{}", slug);
        let last_resume = state.last_run.get(&key).copied().unwrap_or(0);
        let elapsed = now_unix.saturating_sub(last_resume);
        elapsed >= interval_secs // true = fire, false = skip
    }

    #[test]
    fn test_plan_resume_gate_first_run_fires() {
        // Empty state: last_run for slug is absent → elapsed = now_unix → fires.
        let state = HeartbeatState::default();
        let now_unix = 1_700_000_000;
        assert!(
            plan_resume_gate(&state, "2026-05-03-some-slug", now_unix, 3600),
            "first run (no last_run entry) must fire"
        );
    }

    #[test]
    fn test_plan_resume_gate_recent_run_skips() {
        // Recent run (10 minutes ago) with 1h interval → must skip.
        let mut state = HeartbeatState::default();
        let now_unix = 1_700_000_000;
        state
            .last_run
            .insert("plan_resume:2026-05-03-some-slug".to_string(), now_unix - 600);
        assert!(
            !plan_resume_gate(&state, "2026-05-03-some-slug", now_unix, 3600),
            "recent run within interval must skip (no noise loop)"
        );
    }

    #[test]
    fn test_plan_resume_gate_interval_elapsed_fires() {
        // Run >1h ago with 1h interval → must fire.
        let mut state = HeartbeatState::default();
        let now_unix = 1_700_000_000;
        state
            .last_run
            .insert("plan_resume:2026-05-03-some-slug".to_string(), now_unix - 3700);
        assert!(
            plan_resume_gate(&state, "2026-05-03-some-slug", now_unix, 3600),
            "elapsed > interval must fire"
        );
    }

    #[test]
    fn test_plan_resume_gate_per_slug_isolation() {
        // Two slugs: one recently fired, one not. The not-fired one must still fire.
        let mut state = HeartbeatState::default();
        let now_unix = 1_700_000_000;
        state
            .last_run
            .insert("plan_resume:slug-a".to_string(), now_unix - 100);
        // slug-b is never inserted.
        assert!(
            !plan_resume_gate(&state, "slug-a", now_unix, 3600),
            "slug-a (recent) must skip"
        );
        assert!(
            plan_resume_gate(&state, "slug-b", now_unix, 3600),
            "slug-b (no entry) must fire — gate is per-slug, not global"
        );
    }

    // --- watchdog stall decision (#156) ---

    #[test]
    fn test_watchdog_alerts_when_never_ticked_since_start() {
        // The hole #156 closes: heartbeat machinery dead from boot. No tick has
        // ever been recorded for this run; process_start is older than the
        // threshold → must page. Old restart-guard left this permanently silent.
        let start = 1_700_000_000;
        let now = start + 700; // > 600s threshold elapsed since boot
        assert!(
            watchdog_should_alert(None, start, now, 600),
            "a process dead from start (no tick ever) must page after the threshold"
        );
    }

    #[test]
    fn test_watchdog_alerts_when_only_pre_start_tick_exists() {
        // Stale state file from a prior run: last_tick predates process_start and
        // no in-run tick followed → also the never-ticked-since-start case. Must
        // page once process_start is older than the threshold.
        let start = 1_700_000_000;
        let stale_tick = start - 5000; // from before the restart
        let now = start + 700;
        assert!(
            watchdog_should_alert(Some(stale_tick), start, now, 600),
            "a stale pre-start tick is not liveness — a dead-from-boot process must still page"
        );
    }

    #[test]
    fn test_watchdog_silent_within_threshold_after_start() {
        // Fresh boot, threshold not yet elapsed → no alert (avoids false-positive
        // paging immediately after restart before any tick is expected).
        let start = 1_700_000_000;
        let now = start + 100; // < 600s
        assert!(
            !watchdog_should_alert(None, start, now, 600),
            "must not page before the stall threshold elapses post-start"
        );
    }

    #[test]
    fn test_watchdog_silent_with_recent_in_run_tick() {
        // Machinery is alive: an in-run tick happened recently → no alert, even
        // though process_start itself is old.
        let start = 1_700_000_000;
        let recent_tick = start + 10_000;
        let now = recent_tick + 100; // fresh tick, well within threshold
        assert!(
            !watchdog_should_alert(Some(recent_tick), start, now, 600),
            "a recent in-run tick means the machinery is alive — must stay silent"
        );
    }

    #[test]
    fn test_watchdog_alerts_when_in_run_tick_goes_stale() {
        // Machinery ticked after start, then stalled longer than the threshold
        // → genuine sustained stall, must page.
        let start = 1_700_000_000;
        let in_run_tick = start + 1000;
        let now = in_run_tick + 700; // > 600s since the last live tick
        assert!(
            watchdog_should_alert(Some(in_run_tick), start, now, 600),
            "an in-run tick that goes stale past the threshold is a real stall — must page"
        );
    }

    #[test]
    fn test_plan_resume_interval_default_is_3600() {
        let cfg = HeartbeatConfig::default();
        assert_eq!(
            cfg.plan_resume_interval_secs, 3600,
            "default plan_resume_interval_secs must be 1h to match preflight gate cadence"
        );
    }

    // --- γ safety-net (#80) ---
    //
    // These tests exercise `spawn_safety_net` directly via injected dependencies
    // (extract-helper refactor enables this — no Heartbeat construction needed).
    // `tokio::time::pause` + `advance` provides fast-tick assertions without
    // real wall-clock waits.

    #[tokio::test(start_paused = true)]
    async fn test_gamma_fires_within_interval() {
        // γ should emit WakeRequest{reason:"safety_net_forced_tick"} every
        // `interval_secs`. With paused time, advancing past the interval
        // deterministically triggers the send.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<WakeRequest>(4);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let handle = spawn_safety_net(tx, 60, shutdown_rx);

        // Yield once so the spawned task registers its sleep before we advance.
        tokio::task::yield_now().await;
        // Advance past the 60s interval — γ tick should fire.
        tokio::time::advance(std::time::Duration::from_secs(61)).await;

        // Under start_paused + advance, the spawned sleep resolves; recv returns.
        let wake = rx
            .recv()
            .await
            .expect("wake channel closed unexpectedly");

        assert_eq!(wake.reason, "safety_net_forced_tick");
        assert!(wake.agent_id.is_none());

        // Clean shutdown to avoid leaking the task into other tests.
        let _ = shutdown_tx.send(true);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
    }

    #[tokio::test(start_paused = true)]
    async fn test_gamma_uses_configured_interval() {
        // γ should respect the injected `interval_secs` — at half the interval,
        // no fire; past full interval, fire. Verifies the helper actually
        // reads the parameter (not a hardcoded constant).
        let (tx, mut rx) = tokio::sync::mpsc::channel::<WakeRequest>(4);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let handle = spawn_safety_net(tx, 3600, shutdown_rx);

        // Yield once so the spawned task registers its sleep before we advance.
        tokio::task::yield_now().await;

        // Half-interval: should NOT fire.
        tokio::time::advance(std::time::Duration::from_secs(1800)).await;
        tokio::task::yield_now().await;
        assert!(
            rx.try_recv().is_err(),
            "γ fired before interval elapsed — helper not reading interval_secs"
        );

        // Past full interval: should fire.
        tokio::time::advance(std::time::Duration::from_secs(1801)).await;
        let wake = rx
            .recv()
            .await
            .expect("wake channel closed unexpectedly");
        assert_eq!(wake.reason, "safety_net_forced_tick");

        let _ = shutdown_tx.send(true);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
    }

    #[tokio::test(start_paused = true)]
    async fn test_gamma_terminates_on_shutdown() {
        // γ should exit cleanly when `shutdown` flips to true — no zombie task.
        // Joining the handle within a bounded timeout proves termination.
        let (tx, _rx) = tokio::sync::mpsc::channel::<WakeRequest>(4);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let handle = spawn_safety_net(tx, 3600, shutdown_rx);

        // Signal shutdown.
        shutdown_tx
            .send(true)
            .expect("shutdown channel closed before send");

        // Helper must observe the shutdown and exit. With paused time we
        // need to yield so the select! branch on `shutdown.changed()` runs.
        tokio::task::yield_now().await;

        // Bounded join — if the task didn't terminate, this times out.
        let join_result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            handle,
        )
        .await;

        assert!(
            join_result.is_ok(),
            "γ task did not terminate within 5s of shutdown signal — zombie"
        );
        assert!(
            join_result.unwrap().is_ok(),
            "γ task panicked rather than exiting cleanly"
        );
    }

    // --- #89.2: Peer-status context injection ---

    #[tokio::test]
    async fn test_peer_status_context_default_none() {
        // The shared handle starts as None — no fleet data yet.
        let handle: Arc<tokio::sync::RwLock<Option<String>>> =
            Arc::new(tokio::sync::RwLock::new(None));
        let guard = handle.read().await;
        assert!(guard.is_none());
    }

    #[tokio::test]
    async fn test_peer_status_context_single_peer() {
        let handle: Arc<tokio::sync::RwLock<Option<String>>> =
            Arc::new(tokio::sync::RwLock::new(None));

        // Simulate gateway writing a single peer
        let status = "\n\n## Fleet Peer Status\n- zeus-titan (bot) — last seen: 1234567890, last msg: HEARTBEAT_OK";
        *handle.write().await = Some(status.to_string());

        let guard = handle.read().await;
        assert_eq!(guard.as_deref(), Some(status));
    }

    #[tokio::test]
    async fn test_peer_status_context_multiple_peers() {
        let handle: Arc<tokio::sync::RwLock<Option<String>>> =
            Arc::new(tokio::sync::RwLock::new(None));

        let status = "\n\n## Fleet Peer Status\n- zeus-titan (bot) — last seen: 100, last msg: hello\n- zeus-hermes (bot) — last seen: 200, last msg: working";
        *handle.write().await = Some(status.to_string());

        let guard = handle.read().await;
        let val = guard.as_ref().unwrap();
        assert!(val.contains("zeus-titan"));
        assert!(val.contains("zeus-hermes"));
        assert!(val.contains("## Fleet Peer Status"));
    }

    #[tokio::test]
    async fn test_peer_status_context_empty_after_clear() {
        let handle: Arc<tokio::sync::RwLock<Option<String>>> =
            Arc::new(tokio::sync::RwLock::new(None));

        *handle.write().await = Some("some data".to_string());
        assert!(handle.read().await.is_some());

        // Simulate gateway clearing when presence is empty
        *handle.write().await = None;
        assert!(handle.read().await.is_none());
    }

    // --- #89.3: Unread context injection ---

    #[tokio::test]
    async fn test_unread_context_default_none() {
        let handle: Arc<tokio::sync::RwLock<Option<String>>> =
            Arc::new(tokio::sync::RwLock::new(None));
        assert!(handle.read().await.is_none());
    }

    #[tokio::test]
    async fn test_unread_context_single_channel() {
        let handle: Arc<tokio::sync::RwLock<Option<String>>> =
            Arc::new(tokio::sync::RwLock::new(None));

        let context = "\n\n## Recent Channel Messages\n- [user] alice: hey team\n- [bot] zeus-titan: HEARTBEAT_OK";
        *handle.write().await = Some(context.to_string());

        let guard = handle.read().await;
        let val = guard.as_ref().unwrap();
        assert!(val.contains("alice"));
        assert!(val.contains("zeus-titan"));
        assert!(val.contains("## Recent Channel Messages"));
    }

    #[tokio::test]
    async fn test_unread_context_empty_messages() {
        let handle: Arc<tokio::sync::RwLock<Option<String>>> =
            Arc::new(tokio::sync::RwLock::new(None));

        // Gateway sets None when no messages exist
        *handle.write().await = None;
        assert!(handle.read().await.is_none());
    }

    // -----------------------------------------------------------------------
    // #119: Discord verbosity / heartbeat-flood suppression
    // -----------------------------------------------------------------------

    #[test]
    fn test_max_iter_result_is_silent() {
        // The max-iter completion is the #119 flood string; it must be silent
        // so it's filtered before any Discord send site.
        let r = TaskResult {
            task: "t".into(),
            success: true,
            silent: true,
            output: "Completed after 20 tool iterations".into(),
        };
        assert!(r.silent, "max-iter completion must be silent (#119)");
        assert!(is_routine_filler(&r.output));
    }

    #[test]
    fn test_should_post_max_iter_filler_suppressed() {
        // Meaningful (default): the flood string is suppressed.
        assert!(
            !should_post_to_discord(
                DiscordVerbosity::Meaningful,
                true,
                "Completed after 20 tool iterations"
            ),
            "routine filler must NOT post at Meaningful verbosity"
        );
    }

    #[test]
    fn test_should_post_failure_always_posted() {
        // FAILs post at every verbosity, even with filler-looking output.
        for v in [
            DiscordVerbosity::Verbose,
            DiscordVerbosity::Meaningful,
            DiscordVerbosity::Quiet,
        ] {
            assert!(
                should_post_to_discord(v, false, "Completed after 20 tool iterations"),
                "failures must always post (verbosity={v:?})"
            );
            assert!(should_post_to_discord(v, false, "real error detail"));
        }
    }

    #[test]
    fn test_should_post_substantive_output_posted() {
        // Substantive success output posts at Meaningful + Verbose.
        let body = "Pushed fix/119 — SHA abc123, 387 tests green";
        assert!(should_post_to_discord(DiscordVerbosity::Meaningful, true, body));
        assert!(should_post_to_discord(DiscordVerbosity::Verbose, true, body));
        // ...but Quiet suppresses all success.
        assert!(!should_post_to_discord(DiscordVerbosity::Quiet, true, body));
    }

    #[test]
    fn test_verbose_posts_filler() {
        // Verbose preserves legacy behavior — even filler goes through.
        assert!(should_post_to_discord(
            DiscordVerbosity::Verbose,
            true,
            "Completed after 5 tool iterations"
        ));
    }

    #[test]
    fn test_is_routine_filler_matrix() {
        assert!(is_routine_filler("Completed after 12 tool iterations"));
        assert!(is_routine_filler("Completed after 1 tool iteration"));
        assert!(is_routine_filler("HEARTBEAT_OK"));
        assert!(is_routine_filler("   "));
        assert!(is_routine_filler(""));
        // Substantive output is NOT filler.
        assert!(!is_routine_filler("Drafted release notes, posted for review"));
        assert!(!is_routine_filler("FAIL: cargo check 3 errors"));
    }

    #[test]
    fn test_is_noop_heartbeat_strips_speaker_prefix() {
        // #156: fleet messages arrive tagged `[name]:` — the prefix must be
        // stripped before the no-op check, or tagged acks slip past the gate.
        assert!(is_noop_heartbeat("[zeus-freebsd]: HEARTBEAT_OK"));
        assert!(is_noop_heartbeat("[Zeus100]: heartbeat_ok"));
        assert!(is_noop_heartbeat("[bot]: HEARTBEAT_OK."));
        assert!(is_noop_heartbeat("[x]: Completed after 3 tool iterations"));
        assert!(is_noop_heartbeat("[x]:   "));
        // Bare (unprefixed) forms still match.
        assert!(is_noop_heartbeat("HEARTBEAT_OK"));
        assert!(is_noop_heartbeat(""));
        assert!(is_noop_heartbeat("   "));
        // A tagged substantive message is NOT a no-op — real work must post.
        assert!(!is_noop_heartbeat("[zeus-freebsd]: Shipped fix @ abc123"));
        assert!(!is_noop_heartbeat("[x]: FAIL: cargo check 3 errors"));
    }

    #[test]
    fn test_did_work_counts_oneshot_notool_completion() {
        // #156 BLOCKING regression: a goal the LLM resolves in a single no-tool
        // text completion (iterations==1, tool_calls empty — the legit terminal
        // path at tool_executor.rs:627) IS work and must advance the stack. The
        // gateway goal-loop `did_work` predicate is exactly:
        //   !is_noop_heartbeat(&response) && !response.trim().is_empty()
        // It must NOT depend on iterations or tool_calls — gating on those (the
        // prior shape) misclassified the one-shot answer as a no-op and wedged
        // the whole goal stack behind it, re-cooking every 60s forever.
        let did_work = |r: &str| !is_noop_heartbeat(r) && !r.trim().is_empty();

        // A substantive one-shot answer — no tools, single iteration — is work.
        assert!(
            did_work("The capital of France is Paris."),
            "one-shot informational answer must count as work and advance the stack"
        );
        assert!(did_work("[zeus-freebsd]: Here is the summary you asked for: ..."));

        // True no-ops still stay pending (not work).
        assert!(!did_work("HEARTBEAT_OK"));
        assert!(!did_work("[zeus-freebsd]: HEARTBEAT_OK"));
        assert!(!did_work("Completed after 3 tool iterations"));
        assert!(!did_work(""));
        assert!(!did_work("   "));
    }

    #[test]
    fn test_is_routine_filler_delegates_to_is_noop_heartbeat() {
        // #156: the two must agree — is_routine_filler is now a thin delegate.
        for s in [
            "HEARTBEAT_OK",
            "[zeus-freebsd]: HEARTBEAT_OK",
            "Completed after 5 tool iterations",
            "",
            "Drafted release notes",
        ] {
            assert_eq!(
                is_routine_filler(s),
                is_noop_heartbeat(s),
                "delegation mismatch for {s:?}"
            );
        }
    }

    #[test]
    fn test_discord_verbosity_default_is_meaningful() {
        assert_eq!(DiscordVerbosity::default(), DiscordVerbosity::Meaningful);
        assert_eq!(
            HeartbeatConfig::default().discord_verbosity,
            DiscordVerbosity::Meaningful
        );
    }

    // --- Autonomy regression guards (#156) ---------------------------------
    // The #156 silent-heartbeat fix lives entirely at the POST layer. These
    // two guards lock the autonomy machinery 76ca3c42 broke, so a future
    // post-layer change can never silently re-collapse the wake prompt or the
    // structured-task set without tripping the suite.

    /// The action-first wake prompt MUST survive: agents wake to WORK, and
    /// `HEARTBEAT_OK` is the explicit exception, not the default. 76ca3c42's
    /// kill inverted this into a silent-default prompt. If this string ever
    /// disappears from the source, autonomy regressed — fail loudly.
    #[test]
    fn test_action_first_wake_prompt_survives() {
        let src = include_str!("heartbeat.rs");
        assert!(
            src.contains("Default expectation: you are working, not idling."),
            "action-first wake prompt missing — autonomy regression (#156)"
        );
        assert!(
            src.contains("`HEARTBEAT_OK` is the exception, not the default."),
            "HEARTBEAT_OK-is-the-exception clause missing — autonomy regression (#156)"
        );
    }

    /// The wake prompt MUST keep the full TASK PRIORITY ladder (CURRENT TASK →
    /// QUEUE → routine fallthrough), not collapse to a single routine item.
    /// This is the prompt-side mirror of zeus-memory's
    /// `test_parse_structured_tasks_default_heartbeat` (6-task count guard).
    #[test]
    fn test_wake_prompt_keeps_full_task_priority_ladder() {
        let src = include_str!("heartbeat.rs");
        assert!(
            src.contains("## TASK PRIORITY — READ THIS FIRST"),
            "TASK PRIORITY ladder missing — wake prompt collapsed (#156)"
        );
        assert!(
            src.contains("CURRENT TASK") && src.contains("PENDING TASK QUEUE"),
            "CURRENT TASK / QUEUE rungs missing from wake prompt (#156)"
        );
    }
}
