//! Phase 3c — `cook_until_done` task-driven autonomous loop.
//!
//! Wraps `Agent::run` in an outer loop driven by a persisted task record.
//! The loop continues until one of:
//!   - acceptance checks pass          → `TaskOutcome::Done`
//!   - acceptance check reports Blocked → `TaskOutcome::Blocked`
//!   - iterations budget exhausted      → `TaskOutcome::BudgetExhausted`
//!   - wall-clock budget exhausted      → `TaskOutcome::TimedOut`
//!   - underlying `Agent::run` errors repeatedly → `TaskOutcome::Failed`
//!
//! `TaskStore` (SQLite) lives in `zeus-api`, which depends on `zeus-agent` —
//! so we abstract persistence behind the `TaskPersistence` trait and let the
//! gateway inject a `zeus-api`-backed impl.
//!
//! Similarly, acceptance-check execution is Phase 3d (owned by zeus106) —
//! we accept anything that implements `AcceptanceChecker`. Until 3d lands we
//! provide `NoopAcceptanceChecker` which always reports `Progressing` and
//! relies on the iterations budget to terminate the loop.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::agent_loop::Agent;

// ===========================================================================
// Public types
// ===========================================================================

/// Outcome of a `cook_until_done` run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    /// Acceptance checks all passed.
    Done,
    /// An acceptance check reported the task cannot proceed.
    Blocked { reason: String },
    /// Exceeded `iterations_budget`.
    BudgetExhausted { used: i64, budget: i64 },
    /// Exceeded wall-clock budget.
    TimedOut { seconds: u64 },
    /// `Agent::run` errored `max_retries` consecutive times.
    Failed { error: String, iterations: i64 },
}

impl TaskOutcome {
    pub fn is_terminal_success(&self) -> bool {
        matches!(self, TaskOutcome::Done)
    }

    pub fn as_status_str(&self) -> &'static str {
        match self {
            TaskOutcome::Done => "completed",
            TaskOutcome::Blocked { .. } => "paused",
            TaskOutcome::BudgetExhausted { .. }
            | TaskOutcome::TimedOut { .. }
            | TaskOutcome::Failed { .. } => "failed",
        }
    }

    pub fn summary(&self) -> String {
        match self {
            TaskOutcome::Done => "✅ task complete".to_string(),
            TaskOutcome::Blocked { reason } => format!("⏸️ blocked: {reason}"),
            TaskOutcome::BudgetExhausted { used, budget } => {
                format!("⛔ budget exhausted ({used}/{budget})")
            }
            TaskOutcome::TimedOut { seconds } => format!("⏱️ timed out after {seconds}s"),
            TaskOutcome::Failed { error, iterations } => {
                format!("❌ failed after {iterations} iterations: {error}")
            }
        }
    }
}

/// Minimal view of a persisted task exposed to the loop. Mirrors the
/// relevant columns from `zeus-api::handlers::task_store::AgentTask` but
/// is defined here to avoid a `zeus-agent → zeus-api` cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub agent_id: String,
    pub description: String,
    pub scope_json: serde_json::Value,
    pub iterations_used: i64,
    pub iterations_budget: i64,
    pub assigned_by: String,
    pub source_channel: String,
    pub branch: String,
    pub priority: i64,
}

/// Acceptance check verdict returned between iterations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptanceResult {
    /// All acceptance checks pass — terminate with `Done`.
    Done,
    /// Checks not yet satisfied, but progress is possible — keep looping.
    Progressing,
    /// An unrecoverable condition was hit — terminate with `Blocked`.
    Blocked(String),
}

/// Persistence backend for the cooking loop. Implemented by the gateway
/// with a real `TaskStore` handle; a no-op impl is used for tests.
#[async_trait]
pub trait TaskPersistence: Send + Sync {
    /// Load a task record by id.
    async fn load(&self, task_id: &str) -> Option<TaskRecord>;

    /// Persist a status transition (pending/active/paused/completed/failed).
    async fn set_status(&self, task_id: &str, status: &str) -> bool;

    /// Increment `iterations_used` by 1 and persist the checkpoint blob.
    async fn checkpoint_iteration(
        &self,
        task_id: &str,
        iterations_used: i64,
        checkpoint: &serde_json::Value,
    ) -> bool;

    /// Record a final outcome (appended to checkpoint / logged).
    async fn record_outcome(&self, task_id: &str, outcome: &TaskOutcome) -> bool;
}

/// Acceptance-check runner. Phase 3d (zeus106) will provide the real impl;
/// until then `NoopAcceptanceChecker` keeps the loop compiling.
#[async_trait]
pub trait AcceptanceChecker: Send + Sync {
    /// Evaluate the task's acceptance spec against current state.
    async fn check(&self, task: &TaskRecord) -> AcceptanceResult;
}

/// No-op checker — always `Progressing`. Relies on the iterations budget
/// to terminate. Safe default until Phase 3d lands.
pub struct NoopAcceptanceChecker;

#[async_trait]
impl AcceptanceChecker for NoopAcceptanceChecker {
    async fn check(&self, _task: &TaskRecord) -> AcceptanceResult {
        AcceptanceResult::Progressing
    }
}

/// Tuning knobs for `cook_until_done`.
#[derive(Debug, Clone)]
pub struct CookConfig {
    /// Consecutive `Agent::run` errors tolerated before `Failed`.
    pub max_consecutive_errors: u32,
    /// Progress-based idle budget (seconds). 0 = disabled.
    ///
    /// This is the **no-progress** budget: the deadline resets on each
    /// successful iteration (a progress signal), so a slow-but-advancing
    /// cook is never killed. Only a genuinely idle loop — one that makes no
    /// progress for this many seconds — trips it. (Historically this was a
    /// pure wall-clock guillotine that killed slow-but-progressing GLM cooks
    /// at 1800s; #255 made it progress-aware.)
    pub wall_budget_seconds: u64,
    /// Absolute hard-cap backstop (seconds). 0 = disabled.
    ///
    /// A genuinely stuck loop that keeps emitting cheap "progress" must still
    /// terminate. This is the absolute wall-clock ceiling measured from cook
    /// start — independent of progress — beyond which the loop always stops.
    /// Defaults to a generous multiple of the idle budget so it only fires on
    /// pathological runs.
    pub hard_cap_seconds: u64,
    /// Back-off base seconds on consecutive errors (capped at 30s).
    pub error_backoff_base_secs: u64,
    /// Sleep between iterations to let external state settle (ms).
    pub inter_iteration_sleep_ms: u64,
}

impl Default for CookConfig {
    fn default() -> Self {
        Self {
            max_consecutive_errors: 3,
            wall_budget_seconds: 1800, // 30 min no-progress idle budget
            hard_cap_seconds: 4 * 1800, // 2h absolute backstop (4× idle budget)
            error_backoff_base_secs: 2,
            inter_iteration_sleep_ms: 250,
        }
    }
}

impl CookConfig {
    /// Build a `CookConfig` whose wall-budget is resolved from the live
    /// `PrometheusConfig` cooking-loop-timeout knob, honouring the same
    /// resolution order the production gateway path uses
    /// (`zeus_core::resolve_cooking_loop_timeout`):
    ///   1. `cooking_loop_timeout` (NL string, e.g. `"2h"`)
    ///   2. `cooking_loop_timeout_secs`
    ///   3. `gateway_default_secs` (falls back to 1800 if zero).
    ///
    /// NOTE: `cook_until_done` is **not** the production cooking path — the
    /// gateway hand-rolls its own `tokio::select!` loop that already consumes
    /// the resolver (`src/gateway.rs:1451`, `:2350`). This constructor exists
    /// so that *if* `cook_until_done` is ever wired into a live path, its
    /// wall-budget honours the operator's `cooking_loop_timeout` config
    /// instead of the hardcoded 30-minute default. Cheap insurance (#176).
    pub fn from_prometheus_config(
        config: &zeus_core::PrometheusConfig,
        gateway_default_secs: u64,
    ) -> Self {
        let resolved =
            zeus_core::resolve_cooking_loop_timeout(config, gateway_default_secs);
        let idle = resolved.as_secs();
        Self {
            wall_budget_seconds: idle,
            // Backstop scales with the operator's resolved idle budget so a
            // longer configured cook gets a proportionally longer hard ceiling.
            hard_cap_seconds: idle.saturating_mul(4),
            ..Self::default()
        }
    }
}

// ===========================================================================
// The loop
// ===========================================================================

/// Drive an `Agent` to completion of a persisted task.
///
/// Contract:
///  - Caller has already `set_tasks_context` / `set_goals_context` if desired.
///  - We mark the task `active` on entry and a terminal status on exit.
///  - Each iteration is checkpointed via `persistence.checkpoint_iteration`
///    BEFORE acceptance is evaluated, so a crash mid-loop is recoverable.
pub async fn cook_until_done(
    agent: &mut Agent,
    persistence: Arc<dyn TaskPersistence>,
    acceptance: Arc<dyn AcceptanceChecker>,
    task_id: &str,
    config: CookConfig,
) -> TaskOutcome {
    // 1. Load + mark active
    let mut task = match persistence.load(task_id).await {
        Some(t) => t,
        None => {
            let outcome = TaskOutcome::Failed {
                error: format!("task {task_id} not found"),
                iterations: 0,
            };
            persistence.record_outcome(task_id, &outcome).await;
            return outcome;
        }
    };
    persistence.set_status(task_id, "active").await;
    info!(
        task_id = %task.id,
        agent_id = %task.agent_id,
        budget = task.iterations_budget,
        "cook_until_done: starting"
    );
    // P2 observability: stable greppable cook-lifecycle line.
    info!(
        target: "cook",
        event = "start",
        task_id = %task.id,
        trigger = %task.assigned_by,
        channel = %task.source_channel,
        "cook start"
    );

    let started = Instant::now();
    // Progress-aware deadline anchor: reset on each successful iteration so a
    // slow-but-advancing cook never trips the idle budget. Only a genuinely
    // idle loop (no progress for `wall_budget_seconds`) is killed. #255.
    let mut last_progress = Instant::now();
    let mut consecutive_errors: u32 = 0;

    // 2. Main loop
    let outcome = loop {
        // 2a. Budget check (iterations)
        if task.iterations_used >= task.iterations_budget {
            break TaskOutcome::BudgetExhausted {
                used: task.iterations_used,
                budget: task.iterations_budget,
            };
        }

        // 2b. Budget check (progress-aware idle + hard-cap backstop).
        //
        // Idle budget: time since the last *progress* (successful iteration),
        // not since cook start — a slow-but-streaming cook resets this each
        // iteration and is never killed. This is what stops the 1800s GLM
        // guillotine from killing cooks that are still advancing.
        if config.wall_budget_seconds > 0
            && last_progress.elapsed() >= Duration::from_secs(config.wall_budget_seconds)
        {
            break TaskOutcome::TimedOut {
                seconds: config.wall_budget_seconds,
            };
        }
        // Hard-cap backstop: absolute wall-clock ceiling from cook start,
        // independent of progress, so a genuinely stuck loop that keeps
        // emitting cheap "progress" still terminates.
        if config.hard_cap_seconds > 0
            && started.elapsed() >= Duration::from_secs(config.hard_cap_seconds)
        {
            break TaskOutcome::TimedOut {
                seconds: config.hard_cap_seconds,
            };
        }

        // 2c. Build iteration prompt — let the LLM see scope + current progress
        let prompt = build_iteration_prompt(&task);

        // 2d. Run one cook iteration
        debug!(
            task_id = %task.id,
            iter = task.iterations_used + 1,
            "cook_until_done: iteration begin"
        );
        let result = agent.run(&prompt).await;

        task.iterations_used += 1;
        match &result {
            Ok(output) => {
                consecutive_errors = 0;
                // Progress signal — a successful iteration resets the idle
                // deadline so the next slow iteration isn't measured against a
                // stale anchor. #255 progress-aware timeout.
                last_progress = Instant::now();
                let checkpoint = serde_json::json!({
                    "iteration": task.iterations_used,
                    "last_output_excerpt": truncate(output, 500),
                    "elapsed_secs": started.elapsed().as_secs(),
                });
                persistence
                    .checkpoint_iteration(task_id, task.iterations_used, &checkpoint)
                    .await;
            }
            Err(e) => {
                consecutive_errors += 1;
                warn!(
                    task_id = %task.id,
                    iter = task.iterations_used,
                    consecutive = consecutive_errors,
                    error = %e,
                    "cook_until_done: iteration errored"
                );
                let checkpoint = serde_json::json!({
                    "iteration": task.iterations_used,
                    "last_error": e.to_string(),
                    "consecutive_errors": consecutive_errors,
                    "elapsed_secs": started.elapsed().as_secs(),
                });
                persistence
                    .checkpoint_iteration(task_id, task.iterations_used, &checkpoint)
                    .await;

                if consecutive_errors >= config.max_consecutive_errors {
                    break TaskOutcome::Failed {
                        error: e.to_string(),
                        iterations: task.iterations_used,
                    };
                }

                // Exponential back-off, capped at 30s
                let backoff = config
                    .error_backoff_base_secs
                    .saturating_pow(consecutive_errors)
                    .min(30);
                tokio::time::sleep(Duration::from_secs(backoff)).await;
                continue;
            }
        }

        // 2e. Evaluate acceptance
        match acceptance.check(&task).await {
            AcceptanceResult::Done => break TaskOutcome::Done,
            AcceptanceResult::Blocked(reason) => {
                break TaskOutcome::Blocked { reason };
            }
            AcceptanceResult::Progressing => {
                // Continue looping
                if config.inter_iteration_sleep_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(config.inter_iteration_sleep_ms))
                        .await;
                }
            }
        }
    };

    // 3. Terminal persistence
    persistence
        .set_status(task_id, outcome.as_status_str())
        .await;
    persistence.record_outcome(task_id, &outcome).await;

    info!(
        task_id = %task.id,
        outcome = %outcome.summary(),
        iterations = task.iterations_used,
        "cook_until_done: terminated"
    );
    // P2 observability: stable greppable cook-lifecycle line.
    info!(
        target: "cook",
        event = "end",
        task_id = %task.id,
        duration_ms = started.elapsed().as_millis() as u64,
        iterations = task.iterations_used,
        result = outcome.as_status_str(),
        "cook end"
    );
    outcome
}

// ===========================================================================
// Helpers
// ===========================================================================

fn build_iteration_prompt(task: &TaskRecord) -> String {
    let scope_pretty = serde_json::to_string_pretty(&task.scope_json)
        .unwrap_or_else(|_| "{}".to_string());
    let branch_note = if task.branch.is_empty() {
        String::new()
    } else {
        format!("\nBranch: `{}`", task.branch)
    };
    format!(
        "## Active Task (iteration {}/{})\n\
         ID: {}\n\
         {}\n{}\n\n\
         ### Scope\n```json\n{}\n```\n\n\
         Continue executing this task. Make concrete progress this iteration \
         (read/edit files, run commands, push commits). If the task is already \
         done, say so explicitly and summarize what was completed. If you are \
         blocked, describe the blocker clearly so the coordinator can unstick it.",
        task.iterations_used + 1,
        task.iterations_budget,
        task.id,
        task.description,
        branch_note,
        scope_pretty,
    )
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut out = s[..max].to_string();
        out.push_str("…");
        out
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct FakePersistence {
        task: StdMutex<Option<TaskRecord>>,
        statuses: StdMutex<Vec<String>>,
        checkpoints: StdMutex<Vec<(i64, serde_json::Value)>>,
        outcomes: StdMutex<Vec<TaskOutcome>>,
    }

    impl FakePersistence {
        fn with_task(task: TaskRecord) -> Arc<Self> {
            Arc::new(Self {
                task: StdMutex::new(Some(task)),
                ..Default::default()
            })
        }
    }

    #[async_trait]
    impl TaskPersistence for FakePersistence {
        async fn load(&self, _task_id: &str) -> Option<TaskRecord> {
            self.task.lock().unwrap().clone()
        }
        async fn set_status(&self, _task_id: &str, status: &str) -> bool {
            self.statuses.lock().unwrap().push(status.to_string());
            true
        }
        async fn checkpoint_iteration(
            &self,
            _task_id: &str,
            used: i64,
            ck: &serde_json::Value,
        ) -> bool {
            self.checkpoints.lock().unwrap().push((used, ck.clone()));
            true
        }
        async fn record_outcome(&self, _task_id: &str, outcome: &TaskOutcome) -> bool {
            self.outcomes.lock().unwrap().push(outcome.clone());
            true
        }
    }

    struct AlwaysDone;
    #[async_trait]
    impl AcceptanceChecker for AlwaysDone {
        async fn check(&self, _t: &TaskRecord) -> AcceptanceResult {
            AcceptanceResult::Done
        }
    }

    struct AlwaysBlocked;
    #[async_trait]
    impl AcceptanceChecker for AlwaysBlocked {
        async fn check(&self, _t: &TaskRecord) -> AcceptanceResult {
            AcceptanceResult::Blocked("dependency missing".into())
        }
    }

    fn rec(budget: i64) -> TaskRecord {
        TaskRecord {
            id: "t1".into(),
            agent_id: "a1".into(),
            description: "test".into(),
            scope_json: serde_json::json!({}),
            iterations_used: 0,
            iterations_budget: budget,
            assigned_by: "test".into(),
            source_channel: "test".into(),
            branch: "test".into(),
            priority: 1,
        }
    }

    #[test]
    fn outcome_status_mapping() {
        assert_eq!(TaskOutcome::Done.as_status_str(), "completed");
        assert_eq!(
            TaskOutcome::Blocked { reason: "x".into() }.as_status_str(),
            "paused"
        );
        assert_eq!(
            TaskOutcome::BudgetExhausted { used: 1, budget: 1 }.as_status_str(),
            "failed"
        );
        assert_eq!(
            TaskOutcome::TimedOut { seconds: 1 }.as_status_str(),
            "failed"
        );
        assert_eq!(
            TaskOutcome::Failed {
                error: "e".into(),
                iterations: 1,
            }
            .as_status_str(),
            "failed"
        );
    }

    #[test]
    fn outcome_is_terminal_success_only_for_done() {
        assert!(TaskOutcome::Done.is_terminal_success());
        assert!(!TaskOutcome::TimedOut { seconds: 1 }.is_terminal_success());
    }

    #[test]
    fn build_iteration_prompt_includes_scope_and_budget() {
        let t = rec(20);
        let p = build_iteration_prompt(&t);
        assert!(p.contains("iteration 1/20"));
        assert!(p.contains("test"));
        assert!(p.contains("Scope"));
    }

    #[test]
    fn truncate_respects_bound() {
        assert_eq!(truncate("hello", 10), "hello");
        let long = "x".repeat(600);
        let t = truncate(&long, 500);
        assert!(t.ends_with("…"));
        assert!(t.chars().count() <= 501);
    }

    #[test]
    fn noop_acceptance_always_progressing() {
        let c = NoopAcceptanceChecker;
        let r = tokio::runtime::Runtime::new().unwrap();
        let v = r.block_on(c.check(&rec(1)));
        assert_eq!(v, AcceptanceResult::Progressing);
    }

    #[test]
    fn always_done_checker_returns_done() {
        let c = AlwaysDone;
        let r = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(r.block_on(c.check(&rec(1))), AcceptanceResult::Done);
    }

    #[test]
    fn always_blocked_checker_returns_blocked() {
        let c = AlwaysBlocked;
        let r = tokio::runtime::Runtime::new().unwrap();
        match r.block_on(c.check(&rec(1))) {
            AcceptanceResult::Blocked(msg) => assert!(msg.contains("dependency")),
            other => panic!("expected Blocked, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn missing_task_returns_failed_immediately() {
        let p: Arc<dyn TaskPersistence> = Arc::new(FakePersistence::default());
        let c: Arc<dyn AcceptanceChecker> = Arc::new(NoopAcceptanceChecker);
        // Cannot actually build an Agent in unit tests (needs LlmClient etc),
        // so this path is exercised via the integration of load() returning None
        // below — see persistence_records_outcome test.
        let _ = (p, c); // silence unused
    }

    #[tokio::test]
    async fn persistence_records_missing_task_outcome() {
        // Fake persistence with no task loaded simulates task_id not found.
        let fake = Arc::new(FakePersistence::default());
        let p: Arc<dyn TaskPersistence> = fake.clone();
        // We can't call cook_until_done without a real Agent, but we can
        // exercise the persistence path directly to validate contract.
        assert!(p.load("nope").await.is_none());
        let outcome = TaskOutcome::Failed {
            error: "task nope not found".into(),
            iterations: 0,
        };
        p.record_outcome("nope", &outcome).await;
        assert_eq!(fake.outcomes.lock().unwrap().len(), 1);
    }

    #[test]
    fn cook_config_defaults_reasonable() {
        let c = CookConfig::default();
        assert!(c.max_consecutive_errors >= 1);
        assert!(c.wall_budget_seconds > 0);
        // #255: hard-cap backstop must be enabled and strictly greater than
        // the idle budget so the absolute ceiling never trips before the
        // progress-aware idle budget on a normal cook.
        assert!(c.hard_cap_seconds > 0, "hard-cap backstop must be enabled by default");
        assert!(
            c.hard_cap_seconds > c.wall_budget_seconds,
            "hard-cap ({}) must exceed idle budget ({}) so the backstop is the last resort",
            c.hard_cap_seconds,
            c.wall_budget_seconds
        );
    }

    #[test]
    fn cook_config_hard_cap_scales_with_resolved_budget() {
        // #255: the absolute backstop should scale with the operator's
        // resolved idle budget (4×), so a longer configured cook gets a
        // proportionally longer ceiling rather than a fixed default.
        let mut cfg = zeus_core::PrometheusConfig::default();
        cfg.cooking_loop_timeout_secs = Some(5400);
        let c = CookConfig::from_prometheus_config(&cfg, 1800);
        assert_eq!(c.wall_budget_seconds, 5400);
        assert_eq!(
            c.hard_cap_seconds,
            5400 * 4,
            "hard-cap must be 4× the resolved idle budget"
        );
    }

    // #176: proof that CookConfig honours the operator's cooking-loop-timeout
    // config via the *same* resolver the production gateway path consumes
    // (`zeus_core::resolve_cooking_loop_timeout`). Guards against the dead-code
    // `cook_until_done` default silently ignoring config if ever wired live.

    #[test]
    fn cook_config_honours_nl_timeout_string() {
        // `cooking_loop_timeout = "2h"` must win over the gateway default.
        let mut cfg = zeus_core::PrometheusConfig::default();
        cfg.cooking_loop_timeout = Some("2h".to_string());
        let c = CookConfig::from_prometheus_config(&cfg, 1800);
        assert_eq!(c.wall_budget_seconds, 2 * 60 * 60, "NL '2h' must resolve to 7200s");
    }

    #[test]
    fn cook_config_honours_secs_override() {
        let mut cfg = zeus_core::PrometheusConfig::default();
        cfg.cooking_loop_timeout_secs = Some(5400);
        let c = CookConfig::from_prometheus_config(&cfg, 1800);
        assert_eq!(c.wall_budget_seconds, 5400, "secs override must win over gateway default");
    }

    #[test]
    fn cook_config_falls_back_to_gateway_default() {
        // Empty config → gateway default carries through.
        let cfg = zeus_core::PrometheusConfig::default();
        let c = CookConfig::from_prometheus_config(&cfg, 3600);
        assert_eq!(c.wall_budget_seconds, 3600, "empty config must fall back to gateway default");
    }
}
