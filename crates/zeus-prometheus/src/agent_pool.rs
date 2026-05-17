//! Agent Pool — parallel sub-agent execution with concurrency control
//!
//! Manages concurrent task execution bounded by a Tokio semaphore, with:
//! - Configurable max concurrency (prevents resource exhaustion)
//! - Per-task timeout (prevents hung agents from blocking the pool)
//! - Rate limiting (minimum delay between task starts)
//! - Partial failure handling (continue-on-failure vs fail-fast)
//! - Result aggregation with success/failure/skipped counts
//!
//! Used by MissionDriver to execute independent plan steps in parallel.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::join_all;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};

use crate::executor::{Executor, StepResult};
use crate::planner::{Plan, PlanStatus, Step, StepStatus};
use crate::tool_executor::ToolExecutor;
use zeus_core::ToolSchema;
use zeus_llm::LlmClient;

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the agent pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPoolConfig {
    /// Maximum number of concurrent agent workers (default: 4)
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,

    /// Per-task timeout in seconds (default: 300 = 5 minutes)
    #[serde(default = "default_task_timeout_secs")]
    pub task_timeout_secs: u64,

    /// Minimum delay between task starts in milliseconds (default: 100)
    #[serde(default = "default_rate_limit_ms")]
    pub rate_limit_ms: u64,

    /// Whether to continue executing other tasks when one fails (default: true)
    #[serde(default = "default_continue_on_failure")]
    pub continue_on_failure: bool,
}

fn default_max_concurrent() -> usize {
    4
}
fn default_task_timeout_secs() -> u64 {
    300
}
fn default_rate_limit_ms() -> u64 {
    100
}
fn default_continue_on_failure() -> bool {
    true
}

impl Default for AgentPoolConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_concurrent(),
            task_timeout_secs: default_task_timeout_secs(),
            rate_limit_ms: default_rate_limit_ms(),
            continue_on_failure: default_continue_on_failure(),
        }
    }
}

// ============================================================================
// Pool Result
// ============================================================================

/// Aggregated result from a pool execution batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolResult {
    /// Individual step results in order of completion
    pub step_results: Vec<StepResult>,
    /// Total wall-clock time for the batch
    pub total_time_ms: u64,
    /// Number of successfully completed tasks
    pub tasks_succeeded: usize,
    /// Number of failed tasks
    pub tasks_failed: usize,
    /// Number of skipped tasks (dependency failures)
    pub tasks_skipped: usize,
}

impl PoolResult {
    /// Whether all tasks succeeded.
    pub fn all_succeeded(&self) -> bool {
        self.tasks_failed == 0 && self.tasks_skipped == 0
    }

    /// Whether at least one task succeeded.
    pub fn any_succeeded(&self) -> bool {
        self.tasks_succeeded > 0
    }
}

// ============================================================================
// Agent Pool
// ============================================================================

/// Parallel sub-agent execution pool with concurrency control.
///
/// Wraps around the existing `Executor` to add semaphore-bounded parallelism,
/// per-task timeouts, and rate limiting.
pub struct AgentPool {
    config: AgentPoolConfig,
    semaphore: Arc<Semaphore>,
}

impl AgentPool {
    /// Create a new agent pool with the given configuration.
    pub fn new(config: AgentPoolConfig) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent));
        info!(
            max_concurrent = config.max_concurrent,
            timeout_secs = config.task_timeout_secs,
            rate_limit_ms = config.rate_limit_ms,
            "AgentPool initialized"
        );
        Self { config, semaphore }
    }

    /// Create a pool with default configuration.
    pub fn default_pool() -> Self {
        Self::new(AgentPoolConfig::default())
    }

    /// Current number of available permits (idle workers).
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Maximum concurrent workers.
    pub fn max_concurrent(&self) -> usize {
        self.config.max_concurrent
    }

    /// Execute a full plan using pool-managed parallelism.
    ///
    /// Respects step dependencies: only steps whose dependencies are all
    /// completed are eligible for dispatch. Independent steps run in parallel
    /// up to `max_concurrent`.
    pub async fn execute_plan(
        &self,
        plan: &Plan,
        executor: &Executor,
        llm: &LlmClient,
        tools: &[ToolSchema],
        tool_executor: Option<&dyn ToolExecutor>,
    ) -> PoolResult {
        let start = Instant::now();
        let mut plan = plan.clone();
        plan.status = PlanStatus::InProgress;

        let total_steps = plan.steps.len();
        let mut completed: HashSet<usize> = HashSet::new();
        let mut results: Vec<StepResult> = Vec::new();
        let mut failed_steps: HashSet<usize> = HashSet::new();

        while completed.len() < total_steps {
            // Find steps ready to execute (dependencies met, not completed)
            let ready: Vec<usize> = (0..total_steps)
                .filter(|&i| {
                    let step_id = plan.steps[i].id;
                    !completed.contains(&step_id)
                        && plan.steps[i]
                            .dependencies
                            .iter()
                            .all(|dep| completed.contains(dep))
                })
                .collect();

            if ready.is_empty() {
                // Remaining steps have unmet deps — skip them
                for i in 0..total_steps {
                    let step_id = plan.steps[i].id;
                    if !completed.contains(&step_id) {
                        results.push(StepResult {
                            step_id,
                            success: false,
                            output: String::new(),
                            error: Some("Skipped: unresolvable dependency".into()),
                            retries: 0,
                            tool_calls_executed: vec![],
                        });
                        plan.steps[i].status = StepStatus::Skipped;
                        completed.insert(step_id);
                    }
                }
                break;
            }

            // Filter out steps with failed dependencies
            let mut runnable = Vec::new();
            for &i in &ready {
                let dep_failed = plan.steps[i]
                    .dependencies
                    .iter()
                    .any(|dep| failed_steps.contains(dep));

                if dep_failed {
                    let step_id = plan.steps[i].id;
                    results.push(StepResult {
                        step_id,
                        success: false,
                        output: String::new(),
                        error: Some("Skipped: dependency failed".into()),
                        retries: 0,
                        tool_calls_executed: vec![],
                    });
                    plan.steps[i].status = StepStatus::Skipped;
                    completed.insert(step_id);
                } else {
                    runnable.push(i);
                }
            }

            if runnable.is_empty() {
                continue;
            }

            // Fail-fast: skip remaining if earlier failure and not continue_on_failure
            if !self.config.continue_on_failure && !failed_steps.is_empty() {
                for &i in &runnable {
                    let step_id = plan.steps[i].id;
                    results.push(StepResult {
                        step_id,
                        success: false,
                        output: String::new(),
                        error: Some("Skipped: fail-fast after earlier failure".into()),
                        retries: 0,
                        tool_calls_executed: vec![],
                    });
                    plan.steps[i].status = StepStatus::Skipped;
                    completed.insert(step_id);
                }
                continue;
            }

            info!(
                batch_size = runnable.len(),
                permits = self.semaphore.available_permits(),
                "Dispatching parallel batch"
            );

            // Dispatch with semaphore-bounded concurrency
            let batch_results = self
                .dispatch_batch(
                    &runnable,
                    &plan,
                    &results,
                    executor,
                    llm,
                    tools,
                    tool_executor,
                )
                .await;

            for (idx, step_result) in runnable.iter().zip(batch_results.into_iter()) {
                let i = *idx;
                let step_id = plan.steps[i].id;
                if step_result.success {
                    plan.steps[i].status = StepStatus::Completed;
                    plan.steps[i].output = Some(step_result.output.clone());
                } else {
                    plan.steps[i].status = StepStatus::Failed;
                    failed_steps.insert(step_id);
                }
                completed.insert(step_id);
                results.push(step_result);
            }
        }

        let tasks_succeeded = results.iter().filter(|r| r.success).count();
        let tasks_skipped = results
            .iter()
            .filter(|r| !r.success && r.error.as_deref().is_some_and(|e| e.starts_with("Skipped")))
            .count();
        let tasks_failed = results.len() - tasks_succeeded - tasks_skipped;

        PoolResult {
            step_results: results,
            total_time_ms: start.elapsed().as_millis() as u64,
            tasks_succeeded,
            tasks_failed,
            tasks_skipped,
        }
    }

    /// Dispatch a batch of steps with semaphore + timeout + rate limiting.
    #[allow(clippy::too_many_arguments)]
    async fn dispatch_batch(
        &self,
        indices: &[usize],
        plan: &Plan,
        prior_results: &[StepResult],
        executor: &Executor,
        llm: &LlmClient,
        tools: &[ToolSchema],
        tool_executor: Option<&dyn ToolExecutor>,
    ) -> Vec<StepResult> {
        let task_timeout = Duration::from_secs(self.config.task_timeout_secs);
        let rate_limit = Duration::from_millis(self.config.rate_limit_ms);

        let step_data: Vec<(Step, Plan, Vec<StepResult>)> = indices
            .iter()
            .map(|&i| (plan.steps[i].clone(), plan.clone(), prior_results.to_vec()))
            .collect();

        let mut futures = Vec::new();

        for (batch_idx, (step, plan_clone, results_clone)) in step_data.into_iter().enumerate() {
            let sem = self.semaphore.clone();

            // Rate limiting: stagger task starts
            if batch_idx > 0 && !rate_limit.is_zero() {
                tokio::time::sleep(rate_limit).await;
            }

            let step_id = step.id;
            let fut = async move {
                // Acquire semaphore permit — blocks if pool is at capacity
                let _permit = sem.acquire().await.expect("semaphore closed");
                debug!(step_id, "Agent acquired pool permit, executing");

                match tokio::time::timeout(
                    task_timeout,
                    executor.execute_step_public(
                        &step,
                        &plan_clone,
                        &results_clone,
                        llm,
                        tools,
                        tool_executor,
                    ),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        warn!(
                            step_id,
                            timeout_secs = task_timeout.as_secs(),
                            "Agent timed out"
                        );
                        StepResult {
                            step_id,
                            success: false,
                            output: String::new(),
                            error: Some(format!("Timed out after {}s", task_timeout.as_secs())),
                            retries: 0,
                            tool_calls_executed: vec![],
                        }
                    }
                }
            };
            futures.push(fut);
        }

        join_all(futures).await
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::{Plan, PlanStatus, Step, StepStatus};

    fn make_step(id: usize, deps: Vec<usize>) -> Step {
        Step {
            id,
            description: format!("Step {}", id),
            tool: None,
            arguments: None,
            dependencies: deps,
            status: StepStatus::Pending,
            output: None,
        }
    }

    fn make_plan(steps: Vec<Step>) -> Plan {
        Plan {
            task: "test task".to_string(),
            steps,
            status: PlanStatus::Created,
        }
    }

    #[test]
    fn test_config_defaults() {
        let config = AgentPoolConfig::default();
        assert_eq!(config.max_concurrent, 4);
        assert_eq!(config.task_timeout_secs, 300);
        assert_eq!(config.rate_limit_ms, 100);
        assert!(config.continue_on_failure);
    }

    #[test]
    fn test_pool_creation() {
        let pool = AgentPool::new(AgentPoolConfig {
            max_concurrent: 8,
            ..Default::default()
        });
        assert_eq!(pool.max_concurrent(), 8);
        assert_eq!(pool.available_permits(), 8);
    }

    #[test]
    fn test_pool_result_aggregation() {
        let result = PoolResult {
            step_results: vec![
                StepResult {
                    step_id: 1,
                    success: true,
                    output: "ok".into(),
                    error: None,
                    retries: 0,
                    tool_calls_executed: vec![],
                },
                StepResult {
                    step_id: 2,
                    success: false,
                    output: String::new(),
                    error: Some("failed".into()),
                    retries: 1,
                    tool_calls_executed: vec![],
                },
                StepResult {
                    step_id: 3,
                    success: false,
                    output: String::new(),
                    error: Some("Skipped: dependency failed".into()),
                    retries: 0,
                    tool_calls_executed: vec![],
                },
            ],
            total_time_ms: 1000,
            tasks_succeeded: 1,
            tasks_failed: 1,
            tasks_skipped: 1,
        };
        assert!(!result.all_succeeded());
        assert!(result.any_succeeded());
    }

    #[test]
    fn test_pool_result_all_succeeded() {
        let result = PoolResult {
            step_results: vec![StepResult {
                step_id: 1,
                success: true,
                output: "done".into(),
                error: None,
                retries: 0,
                tool_calls_executed: vec![],
            }],
            total_time_ms: 100,
            tasks_succeeded: 1,
            tasks_failed: 0,
            tasks_skipped: 0,
        };
        assert!(result.all_succeeded());
        assert!(result.any_succeeded());
    }

    #[test]
    fn test_default_pool() {
        let pool = AgentPool::default_pool();
        assert_eq!(pool.max_concurrent(), 4);
    }

    #[test]
    fn test_dependency_detection() {
        let steps = vec![
            make_step(1, vec![]),
            make_step(2, vec![]),
            make_step(3, vec![1, 2]),
        ];
        let plan = make_plan(steps);

        let independent: Vec<usize> = plan
            .steps
            .iter()
            .filter(|s| s.dependencies.is_empty())
            .map(|s| s.id)
            .collect();
        assert_eq!(independent, vec![1, 2]);
        assert_eq!(plan.steps[2].dependencies, vec![1, 2]);
    }

    #[test]
    fn test_config_serde() {
        let config = AgentPoolConfig {
            max_concurrent: 16,
            task_timeout_secs: 600,
            rate_limit_ms: 50,
            continue_on_failure: false,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: AgentPoolConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.max_concurrent, 16);
        assert_eq!(parsed.task_timeout_secs, 600);
        assert_eq!(parsed.rate_limit_ms, 50);
        assert!(!parsed.continue_on_failure);
    }

    #[tokio::test]
    async fn test_semaphore_permits() {
        let pool = AgentPool::new(AgentPoolConfig {
            max_concurrent: 2,
            ..Default::default()
        });
        assert_eq!(pool.available_permits(), 2);

        let _permit = pool.semaphore.acquire().await.unwrap();
        assert_eq!(pool.available_permits(), 1);

        let _permit2 = pool.semaphore.acquire().await.unwrap();
        assert_eq!(pool.available_permits(), 0);

        drop(_permit);
        assert_eq!(pool.available_permits(), 1);
    }

    #[tokio::test]
    async fn test_pool_execute_empty_plan() {
        let pool = AgentPool::default_pool();
        let plan = make_plan(vec![]);
        let executor = Executor::new();
        let llm = LlmClient::new(zeus_core::Provider::Ollama, "test".to_string()).unwrap();

        let result = pool.execute_plan(&plan, &executor, &llm, &[], None).await;

        assert_eq!(result.step_results.len(), 0);
        assert!(result.all_succeeded());
    }
}
