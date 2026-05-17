//! Executor - LLM-backed plan execution engine with REAL tool execution
//!
//! Executes plans by having the LLM decide tool arguments for each step,
//! respecting dependency ordering, executing tools via ToolExecutor, and
//! feeding results back for multi-turn step resolution.

use crate::planner::{Plan, PlanStatus, Step, StepStatus};
use crate::tool_executor::ToolExecutor;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{debug, info, warn};
use zeus_core::{Message, Result, ToolSchema};
use zeus_llm::LlmClient;

/// Plan executor that uses LLM to determine tool arguments and executes tools
pub struct Executor {
    max_retries: usize,
    /// Maximum LLM turns per step (prevents infinite loops within a step)
    max_turns_per_step: usize,
}

impl Executor {
    /// Create a new executor
    pub fn new() -> Self {
        Self {
            max_retries: 3,
            max_turns_per_step: 5,
        }
    }

    /// Execute a plan step-by-step, using the LLM + ToolExecutor to drive each step.
    ///
    /// If `tool_executor` is Some, tools are actually executed. If None, falls back
    /// to LLM-only mode (reports what the LLM says it would do).
    pub async fn execute(
        &self,
        plan: &Plan,
        llm: &LlmClient,
        tools: &[ToolSchema],
        tool_executor: Option<&dyn ToolExecutor>,
    ) -> Result<ExecutionResult> {
        let start = std::time::Instant::now();
        let mut results = Vec::new();
        let mut plan = plan.clone();
        plan.status = PlanStatus::InProgress;

        for i in 0..plan.steps.len() {
            // Check dependencies
            if !self.dependencies_met(&plan.steps[i], &results) {
                let dep_failed = plan.steps[i].dependencies.iter().any(|dep_id| {
                    results
                        .iter()
                        .any(|r: &StepResult| r.step_id == *dep_id && !r.success)
                });

                if dep_failed {
                    results.push(StepResult {
                        step_id: plan.steps[i].id,
                        success: false,
                        output: String::new(),
                        error: Some("Skipped: dependency failed".to_string()),
                        retries: 0,
                        tool_calls_executed: vec![],
                    });
                    plan.steps[i].status = StepStatus::Skipped;
                    continue;
                }
            }

            plan.steps[i].status = StepStatus::InProgress;
            let step_result = self
                .execute_step(&plan.steps[i], &plan, &results, llm, tools, tool_executor)
                .await;

            // Store output on the step for subsequent steps to reference
            if step_result.success {
                plan.steps[i].status = StepStatus::Completed;
                plan.steps[i].output = Some(step_result.output.clone());
            } else {
                plan.steps[i].status = StepStatus::Failed;
            }

            results.push(step_result);
        }

        let any_failed = results
            .iter()
            .any(|r| !r.success && r.error.as_deref() != Some("Skipped: dependency failed"));

        plan.status = if any_failed {
            PlanStatus::Failed
        } else {
            PlanStatus::Completed
        };

        Ok(ExecutionResult {
            plan_status: plan.status,
            step_results: results,
            total_time_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Check if all dependencies for a step have been met
    fn dependencies_met(&self, step: &Step, results: &[StepResult]) -> bool {
        step.dependencies
            .iter()
            .all(|dep_id| results.iter().any(|r| r.step_id == *dep_id && r.success))
    }

    /// Execute a single step: LLM decides action -> tools are executed -> results fed back.
    ///
    /// This is a mini cooking loop scoped to one step. The LLM sees:
    /// - The overall plan context
    /// - Previous step results
    /// - The current step description + suggested arguments
    ///   It can make tool calls which are executed, results fed back, until it responds
    ///   with text (step complete) or max turns reached.
    async fn execute_step(
        &self,
        step: &Step,
        plan: &Plan,
        previous_results: &[StepResult],
        llm: &LlmClient,
        tools: &[ToolSchema],
        tool_executor: Option<&dyn ToolExecutor>,
    ) -> StepResult {
        info!("Executing step {}: {}", step.id, step.description);

        // Build context from previous step results
        let context = previous_results
            .iter()
            .filter(|r| r.success)
            .map(|r| {
                let output_preview: String = r.output.chars().take(500).collect();
                format!("Step {} (ok): {}", r.step_id, output_preview)
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Include suggested arguments if present
        let args_hint = if let Some(ref args) = step.arguments {
            format!(
                "\n\nSuggested arguments for this step: {}",
                serde_json::to_string_pretty(args).unwrap_or_default()
            )
        } else {
            String::new()
        };

        let system = format!(
            "You are executing step {} of a plan for: {}\n\n\
             Previous step results:\n{}\n\n\
             Current step: {}{}\n\n\
             Execute this step. If a tool is needed, call it with the appropriate arguments. \
             If this is a reasoning/analysis step, provide your analysis as text.\n\
             After tool results come back, summarize the outcome concisely.",
            step.id,
            plan.task,
            if context.is_empty() {
                "(none)".to_string()
            } else {
                context
            },
            step.description,
            args_hint,
        );

        // Filter tools to only the one needed if specified
        let step_tools: Vec<ToolSchema> = if let Some(ref tool_name) = step.tool {
            tools
                .iter()
                .filter(|t| t.name == *tool_name)
                .cloned()
                .collect()
        } else {
            tools.to_vec()
        };

        // Retry loop (outer)
        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                debug!(
                    "Retrying step {} (attempt {}/{})",
                    step.id,
                    attempt + 1,
                    self.max_retries + 1
                );
            }

            match self
                .run_step_turns(step, &system, &step_tools, llm, tool_executor)
                .await
            {
                Ok((output, tool_calls)) => {
                    return StepResult {
                        step_id: step.id,
                        success: true,
                        output,
                        error: None,
                        retries: attempt,
                        tool_calls_executed: tool_calls,
                    };
                }
                Err(e) if e.is_retryable() && attempt < self.max_retries => {
                    warn!(
                        "Step {} attempt {} failed (retryable): {}",
                        step.id,
                        attempt + 1,
                        e
                    );
                    continue;
                }
                Err(e) => {
                    return StepResult {
                        step_id: step.id,
                        success: false,
                        output: String::new(),
                        error: Some(format!("Error: {}", e)),
                        retries: attempt,
                        tool_calls_executed: vec![],
                    };
                }
            }
        }

        // Should not reach here, but safety net
        StepResult {
            step_id: step.id,
            success: false,
            output: String::new(),
            error: Some("Max retries exhausted".to_string()),
            retries: self.max_retries,
            tool_calls_executed: vec![],
        }
    }

    /// Run the multi-turn LLM+tool loop for a single step.
    ///
    /// Returns (final_output, tool_calls_made) on success.
    async fn run_step_turns(
        &self,
        step: &Step,
        system: &str,
        tools: &[ToolSchema],
        llm: &LlmClient,
        tool_executor: Option<&dyn ToolExecutor>,
    ) -> Result<(String, Vec<StepToolCall>)> {
        let mut messages = vec![Message::user(&step.description)];
        let mut all_tool_calls = Vec::new();
        let mut final_output = String::new();

        for turn in 0..self.max_turns_per_step {
            debug!(step = step.id, turn = turn + 1, "Step turn");

            let response = llm.complete(&messages, tools, Some(system)).await?;

            // If no tool calls, the LLM is done with this step
            if response.tool_calls.is_empty() {
                final_output = response.content;
                break;
            }

            // Add assistant message with tool calls
            let assistant_msg =
                Message::assistant(&response.content).with_tool_calls(response.tool_calls.clone());
            messages.push(assistant_msg);

            // Execute each tool call
            for call in &response.tool_calls {
                let (success, output) = if let Some(executor) = tool_executor {
                    // REAL execution
                    let result = executor.execute_tool(call).await;
                    (result.success, result.output)
                } else {
                    // LLM-only mode: report what was requested
                    (
                        true,
                        format!(
                            "[Simulated] {}({})",
                            call.name,
                            serde_json::to_string(&call.arguments)
                                .unwrap_or_else(|_| "{}".to_string())
                        ),
                    )
                };

                all_tool_calls.push(StepToolCall {
                    name: call.name.clone(),
                    call_id: call.id.clone(),
                    arguments: call.arguments.clone(),
                    success,
                    output: truncate(&output, 4000),
                });

                info!(
                    tool = %call.name,
                    success = success,
                    step = step.id,
                    "Tool executed in step"
                );

                // Add tool result to conversation so LLM can see it
                let tool_msg = Message::tool(&call.id, success, &output);
                messages.push(tool_msg);
            }

            // Capture text content if present alongside tool calls
            if !response.content.is_empty() {
                final_output = response.content;
            }
        }

        // If we exhausted turns but had tool output, summarize
        if final_output.is_empty() && !all_tool_calls.is_empty() {
            final_output = all_tool_calls
                .iter()
                .map(|tc| {
                    format!(
                        "{}({}): {}",
                        tc.name,
                        if tc.success { "ok" } else { "failed" },
                        truncate(&tc.output, 200)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
        }

        Ok((final_output, all_tool_calls))
    }

    /// Set max retries
    pub fn with_max_retries(mut self, max: usize) -> Self {
        self.max_retries = max;
        self
    }

    /// Set max turns per step
    pub fn with_max_turns_per_step(mut self, max: usize) -> Self {
        self.max_turns_per_step = max;
        self
    }

    /// Execute a plan with parallel step execution.
    ///
    /// Steps whose dependencies are all met are executed concurrently via
    /// `tokio::join_all`. Steps with unmet dependencies wait until the
    /// parallel batch completes, then the next batch of ready steps runs.
    pub async fn execute_parallel(
        &self,
        plan: &Plan,
        llm: &LlmClient,
        tools: &[ToolSchema],
        tool_executor: Option<&dyn ToolExecutor>,
    ) -> Result<ExecutionResult> {
        let start = std::time::Instant::now();
        let mut results: Vec<StepResult> = Vec::new();
        let mut plan = plan.clone();
        plan.status = PlanStatus::InProgress;

        // Track which step IDs are completed
        let mut completed: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let total_steps = plan.steps.len();

        while completed.len() < total_steps {
            // Find all steps that are ready to execute (deps met, not yet completed)
            let ready_indices: Vec<usize> = (0..total_steps)
                .filter(|&i| {
                    !completed.contains(&plan.steps[i].id)
                        && plan.steps[i]
                            .dependencies
                            .iter()
                            .all(|dep| completed.contains(dep))
                })
                .collect();

            if ready_indices.is_empty() {
                // No steps are ready but we haven't completed all — remaining steps
                // must have failed dependencies. Skip them.
                for i in 0..total_steps {
                    if !completed.contains(&plan.steps[i].id) {
                        results.push(StepResult {
                            step_id: plan.steps[i].id,
                            success: false,
                            output: String::new(),
                            error: Some("Skipped: dependency failed".to_string()),
                            retries: 0,
                            tool_calls_executed: vec![],
                        });
                        plan.steps[i].status = StepStatus::Skipped;
                        completed.insert(plan.steps[i].id);
                    }
                }
                break;
            }

            // Check for failed dependencies before executing
            let mut batch_indices = Vec::new();
            for &i in &ready_indices {
                let dep_failed = plan.steps[i].dependencies.iter().any(|dep_id| {
                    results
                        .iter()
                        .any(|r: &StepResult| r.step_id == *dep_id && !r.success)
                });
                if dep_failed {
                    results.push(StepResult {
                        step_id: plan.steps[i].id,
                        success: false,
                        output: String::new(),
                        error: Some("Skipped: dependency failed".to_string()),
                        retries: 0,
                        tool_calls_executed: vec![],
                    });
                    plan.steps[i].status = StepStatus::Skipped;
                    completed.insert(plan.steps[i].id);
                } else {
                    batch_indices.push(i);
                }
            }

            if batch_indices.is_empty() {
                continue;
            }

            info!(
                "Parallel batch: executing {} steps concurrently",
                batch_indices.len()
            );

            // Execute batch in parallel
            if batch_indices.len() == 1 {
                // Single step — execute directly (avoids unnecessary spawn)
                let i = batch_indices[0];
                plan.steps[i].status = StepStatus::InProgress;
                let step_result = self
                    .execute_step(&plan.steps[i], &plan, &results, llm, tools, tool_executor)
                    .await;
                if step_result.success {
                    plan.steps[i].status = StepStatus::Completed;
                    plan.steps[i].output = Some(step_result.output.clone());
                } else {
                    plan.steps[i].status = StepStatus::Failed;
                }
                completed.insert(plan.steps[i].id);
                results.push(step_result);
            } else {
                // Multiple steps — run in parallel
                // Collect cloned data so references outlive the futures
                let mut step_data: Vec<(Step, Plan, Vec<StepResult>)> = Vec::new();
                for &i in &batch_indices {
                    plan.steps[i].status = StepStatus::InProgress;
                    step_data.push((plan.steps[i].clone(), plan.clone(), results.clone()));
                }

                let mut futures = Vec::new();
                for (step, plan_clone, results_clone) in &step_data {
                    futures.push(self.execute_step(
                        step,
                        plan_clone,
                        results_clone,
                        llm,
                        tools,
                        tool_executor,
                    ));
                }

                // Await all futures concurrently
                let batch_results = futures::future::join_all(futures).await;

                for (idx, step_result) in batch_indices.iter().zip(batch_results.into_iter()) {
                    let i = *idx;
                    if step_result.success {
                        plan.steps[i].status = StepStatus::Completed;
                        plan.steps[i].output = Some(step_result.output.clone());
                    } else {
                        plan.steps[i].status = StepStatus::Failed;
                    }
                    completed.insert(plan.steps[i].id);
                    results.push(step_result);
                }
            }
        }

        let any_failed = results
            .iter()
            .any(|r| !r.success && r.error.as_deref() != Some("Skipped: dependency failed"));

        plan.status = if any_failed {
            PlanStatus::Failed
        } else {
            PlanStatus::Completed
        };

        Ok(ExecutionResult {
            plan_status: plan.status,
            step_results: results,
            total_time_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Public wrapper around `execute_step` for use by `AgentPool`.
    ///
    /// Executes a single plan step with context from previous results.
    pub async fn execute_step_public(
        &self,
        step: &Step,
        plan: &Plan,
        previous_results: &[StepResult],
        llm: &LlmClient,
        tools: &[ToolSchema],
        tool_executor: Option<&dyn ToolExecutor>,
    ) -> StepResult {
        self.execute_step(step, plan, previous_results, llm, tools, tool_executor)
            .await
    }

    /// Execute a plan with adaptive replanning: if a step fails, ask the
    /// planner to re-plan the remaining work based on what succeeded.
    ///
    /// Returns the combined result of all execution rounds.
    /// `max_replans` limits how many times we can replan (default: 2).
    pub async fn execute_adaptive(
        &self,
        plan: &Plan,
        planner: &crate::planner::Planner,
        llm: &LlmClient,
        tools: &[ToolSchema],
        tool_executor: Option<&dyn ToolExecutor>,
        max_replans: usize,
    ) -> Result<(ExecutionResult, usize)> {
        self.execute_adaptive_cancellable(
            plan,
            planner,
            llm,
            tools,
            tool_executor,
            max_replans,
            None,
        )
        .await
    }

    /// Execute a plan with adaptive replanning and optional cancellation.
    ///
    /// If `cancelled` is Some and set to true, execution stops between steps
    /// and returns a Failed result with the work completed so far.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_adaptive_cancellable(
        &self,
        plan: &Plan,
        planner: &crate::planner::Planner,
        llm: &LlmClient,
        tools: &[ToolSchema],
        tool_executor: Option<&dyn ToolExecutor>,
        max_replans: usize,
        cancelled: Option<Arc<AtomicBool>>,
    ) -> Result<(ExecutionResult, usize)> {
        let start = std::time::Instant::now();
        let mut all_results: Vec<StepResult> = Vec::new();
        let mut replan_count = 0;
        let mut current_plan = plan.clone();

        loop {
            // Check cancellation before each execution round
            if let Some(ref flag) = cancelled
                && flag.load(Ordering::Relaxed)
            {
                info!("Mission execution cancelled before step execution");
                return Ok((
                    ExecutionResult {
                        plan_status: PlanStatus::Failed,
                        step_results: all_results,
                        total_time_ms: start.elapsed().as_millis() as u64,
                    },
                    replan_count,
                ));
            }

            // Execute current plan (with parallel support)
            let result = self
                .execute_parallel(&current_plan, llm, tools, tool_executor)
                .await?;

            all_results.extend(result.step_results.clone());

            // Check cancellation after execution round
            if let Some(ref flag) = cancelled
                && flag.load(Ordering::Relaxed)
            {
                info!("Mission execution cancelled after step execution");
                return Ok((
                    ExecutionResult {
                        plan_status: PlanStatus::Failed,
                        step_results: all_results,
                        total_time_ms: start.elapsed().as_millis() as u64,
                    },
                    replan_count,
                ));
            }

            // If the plan succeeded, we're done
            if result.plan_status == PlanStatus::Completed {
                return Ok((
                    ExecutionResult {
                        plan_status: PlanStatus::Completed,
                        step_results: all_results,
                        total_time_ms: start.elapsed().as_millis() as u64,
                    },
                    replan_count,
                ));
            }

            // Plan failed — try replanning if we have budget
            if replan_count >= max_replans {
                info!(
                    "Adaptive execution exhausted replan budget ({} replans)",
                    replan_count
                );
                return Ok((
                    ExecutionResult {
                        plan_status: PlanStatus::Failed,
                        step_results: all_results,
                        total_time_ms: start.elapsed().as_millis() as u64,
                    },
                    replan_count,
                ));
            }

            replan_count += 1;
            info!(
                "Adaptive replanning (attempt {}/{})",
                replan_count, max_replans
            );

            match planner
                .replan_remaining(&current_plan, &result.step_results, llm, tools)
                .await
            {
                Ok(new_plan) => {
                    if new_plan.steps.is_empty() {
                        info!("Replan produced empty plan — stopping");
                        return Ok((
                            ExecutionResult {
                                plan_status: PlanStatus::Failed,
                                step_results: all_results,
                                total_time_ms: start.elapsed().as_millis() as u64,
                            },
                            replan_count,
                        ));
                    }
                    current_plan = new_plan;
                }
                Err(e) => {
                    warn!("Replanning failed: {}", e);
                    return Ok((
                        ExecutionResult {
                            plan_status: PlanStatus::Failed,
                            step_results: all_results,
                            total_time_ms: start.elapsed().as_millis() as u64,
                        },
                        replan_count,
                    ));
                }
            }
        }
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

/// Truncate a string to approximately max bytes, respecting UTF-8 char boundaries.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Find a valid char boundary at or before `max`
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// Result of executing a plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// Final plan status
    pub plan_status: PlanStatus,
    /// Results for each step
    pub step_results: Vec<StepResult>,
    /// Total execution time
    pub total_time_ms: u64,
}

/// Result of executing a step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    /// Step ID
    pub step_id: usize,
    /// Whether step succeeded
    pub success: bool,
    /// Output from the step
    pub output: String,
    /// Error message if failed
    pub error: Option<String>,
    /// Number of retries
    pub retries: usize,
    /// Tool calls that were executed during this step
    #[serde(default)]
    pub tool_calls_executed: Vec<StepToolCall>,
}

/// Record of a tool call executed within a step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepToolCall {
    /// Tool name
    pub name: String,
    /// Call ID
    pub call_id: String,
    /// Arguments passed
    pub arguments: serde_json::Value,
    /// Whether it succeeded
    pub success: bool,
    /// Output (truncated)
    pub output: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_step(id: usize, desc: &str, tool: Option<&str>, deps: Vec<usize>) -> Step {
        Step {
            id,
            description: desc.to_string(),
            tool: tool.map(|t| t.to_string()),
            arguments: None,
            dependencies: deps,
            status: StepStatus::Pending,
            output: None,
        }
    }

    #[test]
    fn test_executor_defaults() {
        let executor = Executor::new();
        assert_eq!(executor.max_retries, 3);
        assert_eq!(executor.max_turns_per_step, 5);
    }

    #[test]
    fn test_dependencies_met_empty() {
        let executor = Executor::new();
        let step = make_step(1, "test", None, vec![]);
        assert!(executor.dependencies_met(&step, &[]));
    }

    #[test]
    fn test_dependencies_met_with_results() {
        let executor = Executor::new();
        let step = make_step(2, "test", None, vec![1]);
        let results = vec![StepResult {
            step_id: 1,
            success: true,
            output: "done".to_string(),
            error: None,
            retries: 0,
            tool_calls_executed: vec![],
        }];
        assert!(executor.dependencies_met(&step, &results));
    }

    #[test]
    fn test_dependencies_not_met() {
        let executor = Executor::new();
        let step = make_step(2, "test", None, vec![1]);
        let results = vec![StepResult {
            step_id: 1,
            success: false,
            output: String::new(),
            error: Some("failed".to_string()),
            retries: 0,
            tool_calls_executed: vec![],
        }];
        assert!(!executor.dependencies_met(&step, &results));
    }

    #[test]
    fn test_executor_default_trait() {
        let executor = Executor::default();
        assert_eq!(executor.max_retries, 3);
    }

    #[test]
    fn test_with_max_retries() {
        let executor = Executor::new().with_max_retries(5);
        assert_eq!(executor.max_retries, 5);
    }

    #[test]
    fn test_with_max_retries_zero() {
        let executor = Executor::new().with_max_retries(0);
        assert_eq!(executor.max_retries, 0);
    }

    #[test]
    fn test_with_max_turns_per_step() {
        let executor = Executor::new().with_max_turns_per_step(10);
        assert_eq!(executor.max_turns_per_step, 10);
    }

    #[test]
    fn test_dependencies_met_multiple_deps_all_success() {
        let executor = Executor::new();
        let step = make_step(3, "final step", None, vec![1, 2]);
        let results = vec![
            StepResult {
                step_id: 1,
                success: true,
                output: "ok".to_string(),
                error: None,
                retries: 0,
                tool_calls_executed: vec![],
            },
            StepResult {
                step_id: 2,
                success: true,
                output: "ok".to_string(),
                error: None,
                retries: 0,
                tool_calls_executed: vec![],
            },
        ];
        assert!(executor.dependencies_met(&step, &results));
    }

    #[test]
    fn test_dependencies_met_multiple_deps_one_failed() {
        let executor = Executor::new();
        let step = make_step(3, "final step", None, vec![1, 2]);
        let results = vec![
            StepResult {
                step_id: 1,
                success: true,
                output: "ok".to_string(),
                error: None,
                retries: 0,
                tool_calls_executed: vec![],
            },
            StepResult {
                step_id: 2,
                success: false,
                output: String::new(),
                error: Some("failed".to_string()),
                retries: 0,
                tool_calls_executed: vec![],
            },
        ];
        assert!(!executor.dependencies_met(&step, &results));
    }

    #[test]
    fn test_dependencies_met_missing_dep_in_results() {
        let executor = Executor::new();
        let step = make_step(2, "test", None, vec![1]);
        assert!(!executor.dependencies_met(&step, &[]));
    }

    #[test]
    fn test_step_result_serialization() {
        let result = StepResult {
            step_id: 1,
            success: true,
            output: "completed successfully".to_string(),
            error: None,
            retries: 2,
            tool_calls_executed: vec![StepToolCall {
                name: "shell".to_string(),
                call_id: "tc1".to_string(),
                arguments: serde_json::json!({"command": "ls"}),
                success: true,
                output: "file1\nfile2".to_string(),
            }],
        };
        let json = serde_json::to_string(&result).unwrap();
        let deser: StepResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.step_id, 1);
        assert!(deser.success);
        assert_eq!(deser.tool_calls_executed.len(), 1);
        assert_eq!(deser.tool_calls_executed[0].name, "shell");
    }

    #[test]
    fn test_step_result_with_error() {
        let result = StepResult {
            step_id: 3,
            success: false,
            output: String::new(),
            error: Some("Error: timeout".to_string()),
            retries: 3,
            tool_calls_executed: vec![],
        };
        let json = serde_json::to_string(&result).unwrap();
        let deser: StepResult = serde_json::from_str(&json).unwrap();
        assert!(!deser.success);
        assert_eq!(deser.error.as_deref(), Some("Error: timeout"));
    }

    #[test]
    fn test_execution_result_serialization() {
        let result = ExecutionResult {
            plan_status: PlanStatus::Completed,
            step_results: vec![
                StepResult {
                    step_id: 1,
                    success: true,
                    output: "done".to_string(),
                    error: None,
                    retries: 0,
                    tool_calls_executed: vec![],
                },
                StepResult {
                    step_id: 2,
                    success: true,
                    output: "also done".to_string(),
                    error: None,
                    retries: 1,
                    tool_calls_executed: vec![],
                },
            ],
            total_time_ms: 5000,
        };
        let json = serde_json::to_string(&result).unwrap();
        let deser: ExecutionResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.plan_status, PlanStatus::Completed);
        assert_eq!(deser.step_results.len(), 2);
        assert_eq!(deser.total_time_ms, 5000);
    }

    #[test]
    fn test_execution_result_failed_plan() {
        let result = ExecutionResult {
            plan_status: PlanStatus::Failed,
            step_results: vec![StepResult {
                step_id: 1,
                success: false,
                output: String::new(),
                error: Some("error".to_string()),
                retries: 3,
                tool_calls_executed: vec![],
            }],
            total_time_ms: 10000,
        };
        assert_eq!(result.plan_status, PlanStatus::Failed);
        assert!(!result.step_results[0].success);
    }

    #[test]
    fn test_step_tool_call_serialization() {
        let tc = StepToolCall {
            name: "read_file".to_string(),
            call_id: "tc_42".to_string(),
            arguments: serde_json::json!({"path": "/tmp/test.txt"}),
            success: true,
            output: "file contents here".to_string(),
        };
        let json = serde_json::to_string(&tc).unwrap();
        let deser: StepToolCall = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.name, "read_file");
        assert_eq!(deser.call_id, "tc_42");
        assert!(deser.success);
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hello", 100), "hello");
    }

    #[test]
    fn test_truncate_long() {
        let s = "a".repeat(5000);
        let result = truncate(&s, 100);
        assert_eq!(result.len(), 103); // 100 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_exact() {
        let s = "a".repeat(100);
        assert_eq!(truncate(&s, 100), s);
    }

    #[test]
    fn test_dependencies_met_with_empty_results() {
        let executor = Executor::new();
        let step = make_step(3, "depends on 1 and 2", None, vec![1, 2]);
        assert!(!executor.dependencies_met(&step, &[]));
    }

    #[test]
    fn test_execution_result_all_success() {
        let result = ExecutionResult {
            plan_status: PlanStatus::Completed,
            step_results: vec![
                StepResult {
                    step_id: 1,
                    success: true,
                    output: "step 1 done".to_string(),
                    error: None,
                    retries: 0,
                    tool_calls_executed: vec![],
                },
                StepResult {
                    step_id: 2,
                    success: true,
                    output: "step 2 done".to_string(),
                    error: None,
                    retries: 0,
                    tool_calls_executed: vec![],
                },
                StepResult {
                    step_id: 3,
                    success: true,
                    output: "step 3 done".to_string(),
                    error: None,
                    retries: 0,
                    tool_calls_executed: vec![],
                },
            ],
            total_time_ms: 1500,
        };
        assert_eq!(result.plan_status, PlanStatus::Completed);
        assert!(result.step_results.iter().all(|r| r.success));
    }

    #[test]
    fn test_execution_result_partial_failure() {
        let result = ExecutionResult {
            plan_status: PlanStatus::Failed,
            step_results: vec![
                StepResult {
                    step_id: 1,
                    success: true,
                    output: "ok".to_string(),
                    error: None,
                    retries: 0,
                    tool_calls_executed: vec![],
                },
                StepResult {
                    step_id: 2,
                    success: false,
                    output: String::new(),
                    error: Some("timeout".to_string()),
                    retries: 3,
                    tool_calls_executed: vec![],
                },
            ],
            total_time_ms: 8000,
        };
        assert_eq!(result.plan_status, PlanStatus::Failed);
        let successes = result.step_results.iter().filter(|r| r.success).count();
        let failures = result.step_results.iter().filter(|r| !r.success).count();
        assert_eq!(successes, 1);
        assert_eq!(failures, 1);
    }

    #[test]
    fn test_execution_result_serialization_roundtrip() {
        let result = ExecutionResult {
            plan_status: PlanStatus::Completed,
            step_results: vec![StepResult {
                step_id: 1,
                success: true,
                output: "done".to_string(),
                error: None,
                retries: 0,
                tool_calls_executed: vec![StepToolCall {
                    name: "shell".to_string(),
                    call_id: "tc1".to_string(),
                    arguments: serde_json::json!({"command": "echo hi"}),
                    success: true,
                    output: "hi".to_string(),
                }],
            }],
            total_time_ms: 3000,
        };
        let json = serde_json::to_string(&result).unwrap();
        let deser: ExecutionResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.step_results[0].tool_calls_executed.len(), 1);
        assert_eq!(deser.step_results[0].tool_calls_executed[0].output, "hi");
    }

    #[test]
    fn test_truncate_empty() {
        assert_eq!(truncate("", 100), "");
    }

    #[test]
    fn test_truncate_utf8_multibyte() {
        // "é" is 2 bytes in UTF-8; truncating at byte 1 would panic without char-boundary check
        let s = "éééééééééé"; // 10 chars, 20 bytes
        let result = truncate(&s, 5);
        assert!(result.ends_with("..."));
        // Should not panic and should be valid UTF-8
        assert!(result.is_char_boundary(0));
    }

    #[test]
    fn test_truncate_utf8_emoji() {
        // Emoji are 4 bytes each
        let s = "🎉🎉🎉🎉🎉"; // 5 chars, 20 bytes
        let result = truncate(&s, 6);
        assert!(result.ends_with("..."));
        // Should truncate to a valid char boundary (4 bytes = 1 emoji)
        assert!(result.starts_with("🎉"));
    }

    #[test]
    fn test_truncate_utf8_chinese() {
        // CJK chars are 3 bytes each
        let s = "你好世界再见"; // 6 chars, 18 bytes
        let result = truncate(&s, 7);
        assert!(result.ends_with("..."));
        // 7 bytes → backs down to 6 (2 chars boundary)
        assert!(result.starts_with("你好"));
    }

    #[test]
    fn test_dependencies_chain_three_steps() {
        let executor = Executor::new();
        // Step 3 depends on step 2, which depends on step 1
        let step3 = make_step(3, "final", None, vec![2]);
        let results = vec![
            StepResult {
                step_id: 1,
                success: true,
                output: "ok".to_string(),
                error: None,
                retries: 0,
                tool_calls_executed: vec![],
            },
            StepResult {
                step_id: 2,
                success: true,
                output: "ok".to_string(),
                error: None,
                retries: 0,
                tool_calls_executed: vec![],
            },
        ];
        assert!(executor.dependencies_met(&step3, &results));
    }

    #[test]
    fn test_dependencies_partial_missing() {
        let executor = Executor::new();
        // Depends on 1 and 2, but only 1 has a result
        let step = make_step(3, "test", None, vec![1, 2]);
        let results = vec![StepResult {
            step_id: 1,
            success: true,
            output: "ok".to_string(),
            error: None,
            retries: 0,
            tool_calls_executed: vec![],
        }];
        assert!(!executor.dependencies_met(&step, &results));
    }

    #[test]
    fn test_step_tool_call_failed() {
        let tc = StepToolCall {
            name: "shell".to_string(),
            call_id: "tc_99".to_string(),
            arguments: serde_json::json!({"command": "rm -rf /"}),
            success: false,
            output: "permission denied".to_string(),
        };
        let json = serde_json::to_string(&tc).unwrap();
        let deser: StepToolCall = serde_json::from_str(&json).unwrap();
        assert!(!deser.success);
        assert_eq!(deser.output, "permission denied");
    }

    #[test]
    fn test_step_result_skipped_dependency() {
        let result = StepResult {
            step_id: 3,
            success: false,
            output: String::new(),
            error: Some("Skipped: dependency failed".to_string()),
            retries: 0,
            tool_calls_executed: vec![],
        };
        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("Skipped: dependency failed"));
    }

    #[test]
    fn test_make_step_with_arguments() {
        let step = Step {
            id: 1,
            description: "test".to_string(),
            tool: Some("read_file".to_string()),
            arguments: Some(serde_json::json!({"path": "/tmp/test.txt"})),
            dependencies: vec![],
            status: StepStatus::Pending,
            output: None,
        };
        assert!(step.arguments.is_some());
        assert_eq!(step.arguments.unwrap()["path"], "/tmp/test.txt");
    }

    #[test]
    fn test_step_with_output() {
        let step = Step {
            id: 1,
            description: "test".to_string(),
            tool: Some("shell".to_string()),
            arguments: None,
            dependencies: vec![],
            status: StepStatus::Completed,
            output: Some("hello world".to_string()),
        };
        assert_eq!(step.output.as_deref(), Some("hello world"));
    }

    #[test]
    fn test_execution_result_all_skipped() {
        let result = ExecutionResult {
            plan_status: PlanStatus::Failed,
            step_results: vec![
                StepResult {
                    step_id: 1,
                    success: false,
                    output: String::new(),
                    error: Some("Error: network".to_string()),
                    retries: 3,
                    tool_calls_executed: vec![],
                },
                StepResult {
                    step_id: 2,
                    success: false,
                    output: String::new(),
                    error: Some("Skipped: dependency failed".to_string()),
                    retries: 0,
                    tool_calls_executed: vec![],
                },
                StepResult {
                    step_id: 3,
                    success: false,
                    output: String::new(),
                    error: Some("Skipped: dependency failed".to_string()),
                    retries: 0,
                    tool_calls_executed: vec![],
                },
            ],
            total_time_ms: 2000,
        };
        assert_eq!(result.plan_status, PlanStatus::Failed);
        // Only step 1 actually failed; steps 2+3 were skipped
        let real_failures = result
            .step_results
            .iter()
            .filter(|r| !r.success && r.error.as_deref() != Some("Skipped: dependency failed"))
            .count();
        assert_eq!(real_failures, 1);
    }

    #[test]
    fn test_step_tool_call_multiple_in_step() {
        let result = StepResult {
            step_id: 1,
            success: true,
            output: "multi-tool step done".to_string(),
            error: None,
            retries: 0,
            tool_calls_executed: vec![
                StepToolCall {
                    name: "read_file".to_string(),
                    call_id: "tc1".to_string(),
                    arguments: serde_json::json!({"path": "/a"}),
                    success: true,
                    output: "contents of a".to_string(),
                },
                StepToolCall {
                    name: "write_file".to_string(),
                    call_id: "tc2".to_string(),
                    arguments: serde_json::json!({"path": "/b", "content": "data"}),
                    success: true,
                    output: "written".to_string(),
                },
                StepToolCall {
                    name: "shell".to_string(),
                    call_id: "tc3".to_string(),
                    arguments: serde_json::json!({"command": "ls"}),
                    success: false,
                    output: "error: not found".to_string(),
                },
            ],
        };
        assert_eq!(result.tool_calls_executed.len(), 3);
        let successful = result
            .tool_calls_executed
            .iter()
            .filter(|tc| tc.success)
            .count();
        assert_eq!(successful, 2);
    }

    #[test]
    fn test_builder_chain() {
        let executor = Executor::new()
            .with_max_retries(5)
            .with_max_turns_per_step(10);
        assert_eq!(executor.max_retries, 5);
        assert_eq!(executor.max_turns_per_step, 10);
    }
}
