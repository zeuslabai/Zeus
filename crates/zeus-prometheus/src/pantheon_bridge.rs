//! Pantheon Bridge — Connects Prometheus Planner/Executor to Pantheon missions
//!
//! This module bridges two systems:
//! - **Prometheus**: `Planner` decomposes goals → `Plan` with `Step`s; `Executor` runs them
//! - **Pantheon**: `PantheonOrchestrator` manages missions with `MissionTask`s and team assembly
//!
//! The bridge:
//! 1. Takes a mission goal and uses `Planner::create_plan()` for LLM-backed decomposition
//! 2. Converts `Step`s → `MissionTask`s with dependency mapping
//! 3. Assigns tasks to team members based on tool capabilities
//! 4. Drives execution through the `Executor`, emitting `MissionEvent`s
//! 5. Handles adaptive replanning on failures

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tracing::{debug, info, warn};

use crate::spawner::SpawnOutcome;
use zeus_core::ToolSchema;
use zeus_llm::LlmClient;
use zeus_llm::router::ModelRouter;

use crate::agent_pool::AgentPool;
use crate::executor::{ExecutionResult, Executor};
use crate::planner::{Plan, Planner, Step};
use crate::tool_executor::ToolExecutor;

use zeus_orchestra::pantheon::{
    AgentRole, MissionConstraints, MissionState, MissionTask, PantheonOrchestrator,
};

// ---------------------------------------------------------------------------
// Checkpoint callback — allows callers to persist mission state after each task
// ---------------------------------------------------------------------------

/// Trait for persisting mission state after each task completion/failure.
/// Implemented by PantheonStore in zeus-api to avoid circular dependencies.
pub trait MissionCheckpointer: Send + Sync {
    fn checkpoint(&self, mission_id: &str) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

// ---------------------------------------------------------------------------
// Plan → MissionTask conversion
// ---------------------------------------------------------------------------

/// Convert a Prometheus `Plan` into a vec of `MissionTask`s for a Pantheon mission.
///
/// Maps step IDs → task IDs and preserves dependency ordering.
/// Returns (tasks, step_id_to_task_id_map) for cross-referencing.
pub fn plan_to_mission_tasks(
    plan: &Plan,
    mission_id: &str,
) -> (Vec<MissionTask>, HashMap<usize, String>) {
    let mut tasks = Vec::with_capacity(plan.steps.len());
    let mut step_to_task: HashMap<usize, String> = HashMap::new();

    // First pass: create tasks and build ID mapping
    for step in &plan.steps {
        let task = MissionTask::new(mission_id, &step.description);
        step_to_task.insert(step.id, task.id.clone());
        tasks.push(task);
    }

    // Second pass: wire up dependencies using the ID mapping
    for (i, step) in plan.steps.iter().enumerate() {
        let dep_task_ids: Vec<String> = step
            .dependencies
            .iter()
            .filter_map(|dep_step_id| step_to_task.get(dep_step_id).cloned())
            .collect();
        tasks[i].dependencies = dep_task_ids;
    }

    (tasks, step_to_task)
}

/// Find the best team member for a step based on tool name matching.
///
/// If the step has a tool, find a worker whose capabilities contain that tool name.
/// Falls back to the first available worker, then coordinator.
fn best_agent_for_step(
    step: &Step,
    team: &[zeus_orchestra::pantheon::TeamMember],
) -> Option<String> {
    if let Some(ref tool_name) = step.tool {
        // Try to find a worker with matching capability
        if let Some(member) = team.iter().find(|m| {
            m.role == AgentRole::Worker && m.capabilities.iter().any(|c| c.contains(tool_name))
        }) {
            return Some(member.agent_id.clone());
        }
    }

    // Fall back to first available worker
    if let Some(worker) = team.iter().find(|m| m.role == AgentRole::Worker) {
        return Some(worker.agent_id.clone());
    }

    // Last resort: coordinator
    team.iter()
        .find(|m| m.role == AgentRole::Coordinator)
        .map(|m| m.agent_id.clone())
}

// ---------------------------------------------------------------------------
// MissionDriver — drives a Pantheon mission using Prometheus
// ---------------------------------------------------------------------------

/// Drives a Pantheon mission end-to-end using the Prometheus Planner and Executor.
///
/// Lifecycle:
/// 1. Create mission via `PantheonOrchestrator`
/// 2. Assemble team
/// 3. Decompose goal via `Planner::create_plan()`
/// 4. Convert steps → tasks, assign to team
/// 5. Execute via `Executor` with event emission
/// 6. Handle failures with adaptive replanning
pub struct MissionDriver {
    orchestrator: Arc<PantheonOrchestrator>,
    planner: Planner,
    executor: Executor,
    llm: Arc<LlmClient>,
    tools: Vec<ToolSchema>,
    max_replans: usize,
    /// Optional checkpoint callback — persists mission state after each task update.
    checkpointer: Option<Arc<dyn MissionCheckpointer>>,
    /// Optional model router for per-step model selection.
    router: Option<ModelRouter>,
    /// Optional agent pool for parallel step execution.
    agent_pool: Option<AgentPool>,
}

impl MissionDriver {
    pub fn new(
        orchestrator: Arc<PantheonOrchestrator>,
        llm: Arc<LlmClient>,
        tools: Vec<ToolSchema>,
    ) -> Self {
        Self {
            orchestrator,
            planner: Planner::new(),
            executor: Executor::new(),
            llm,
            tools,
            max_replans: 2,
            checkpointer: None,
            router: None,
            agent_pool: None,
        }
    }

    pub fn with_max_steps(mut self, max: usize) -> Self {
        self.planner = self.planner.with_max_steps(max);
        self
    }

    pub fn with_max_retries(mut self, max: usize) -> Self {
        self.executor = self.executor.with_max_retries(max);
        self
    }

    pub fn with_max_replans(mut self, max: usize) -> Self {
        self.max_replans = max;
        self
    }

    /// Attach a checkpoint callback for persisting mission state after each task.
    pub fn with_checkpointer(mut self, checkpointer: Arc<dyn MissionCheckpointer>) -> Self {
        self.checkpointer = Some(checkpointer);
        self
    }

    /// Attach a model router for per-step model selection.
    ///
    /// When set, each plan step gets classified by `TaskType` and routed to
    /// the optimal LLM. Falls back to `self.llm` when routing is disabled
    /// or no route matches.
    pub fn with_router(mut self, router: ModelRouter) -> Self {
        self.router = Some(router);
        self
    }

    /// Attach an agent pool for parallel step execution.
    ///
    /// When set, `drive_mission` dispatches independent steps through
    /// the pool's semaphore-bounded parallelism instead of running
    /// them sequentially via `execute_adaptive`.
    pub fn with_agent_pool(mut self, pool: AgentPool) -> Self {
        self.agent_pool = Some(pool);
        self
    }

    /// Attach a default agent pool (4 concurrent workers).
    pub fn with_default_pool(self) -> Self {
        self.with_agent_pool(AgentPool::default_pool())
    }

    /// Select the LLM client for a given plan step.
    ///
    /// If routing is enabled, classifies the step description and creates
    /// a per-step LlmClient. Otherwise returns the default client.
    #[allow(dead_code)]

    /// Launch a mission: create, plan, assemble, assign, and return the mission ID.
    ///
    /// Does NOT execute — call `drive_mission()` to start execution.
    /// This separation allows the frontend to show the plan before execution begins.
    pub async fn plan_mission(
        &self,
        goal: &str,
        constraints: MissionConstraints,
        required_capabilities: Vec<String>,
    ) -> zeus_core::Result<PlannedMission> {
        // 1. Create mission
        let mission = self
            .orchestrator
            .create_mission(goal, constraints)
            .await
            .map_err(|e| zeus_core::Error::config(e.to_string()))?;

        let mission_id = mission.id.clone();
        info!(mission_id = %mission_id, goal = %goal, "Mission created, planning...");

        // 2. Decompose goal into plan via LLM
        let plan = self
            .planner
            .create_plan(goal, &self.llm, &self.tools)
            .await?;

        info!(
            mission_id = %mission_id,
            steps = plan.steps.len(),
            "Plan created with {} steps",
            plan.steps.len()
        );

        // 3. Convert plan steps → mission tasks
        let (tasks, step_to_task) = plan_to_mission_tasks(&plan, &mission_id);

        // 4. Add tasks to mission
        self.orchestrator
            .add_tasks(&mission_id, tasks)
            .await
            .map_err(|e| zeus_core::Error::config(e.to_string()))?;

        // 5. Assemble team
        let team = self
            .orchestrator
            .assemble_team(&mission_id, required_capabilities)
            .await
            .map_err(|e| zeus_core::Error::config(e.to_string()))?;

        info!(
            mission_id = %mission_id,
            team_size = team.len(),
            "Team assembled with {} agents",
            team.len()
        );

        // 6. Assign tasks to team members
        let mission = self
            .orchestrator
            .get_mission(&mission_id)
            .await
            .ok_or_else(|| {
                zeus_core::Error::config(format!("mission {} disappeared", mission_id))
            })?;

        for step in &plan.steps {
            if let Some(task_id) = step_to_task.get(&step.id)
                && let Some(agent_id) = best_agent_for_step(step, &mission.team)
            {
                let _ = self
                    .orchestrator
                    .assign_task(&mission_id, task_id, &agent_id)
                    .await;
                debug!(
                    task_id = %task_id,
                    agent_id = %agent_id,
                    step = step.id,
                    "Task assigned"
                );
            }
        }

        Ok(PlannedMission {
            mission_id,
            plan,
            step_to_task,
        })
    }

    /// Drive a planned mission to completion.
    ///
    /// Executes the plan via the Prometheus Executor, updating Pantheon state
    /// as each step completes. Supports adaptive replanning on failure.
    ///
    /// If `cancelled` is provided, the execution loop checks it between steps
    /// and aborts early when set to `true` (e.g. by pause/cancel intervention).
    pub async fn drive_mission(
        &self,
        planned: &PlannedMission,
        tool_executor: Option<&dyn ToolExecutor>,
    ) -> zeus_core::Result<MissionResult> {
        self.drive_mission_cancellable(planned, tool_executor, None)
            .await
    }

    /// Drive a planned mission with cancellation support.
    pub async fn drive_mission_cancellable(
        &self,
        planned: &PlannedMission,
        tool_executor: Option<&dyn ToolExecutor>,
        cancelled: Option<Arc<AtomicBool>>,
    ) -> zeus_core::Result<MissionResult> {
        let mission_id = &planned.mission_id;
        info!(mission_id = %mission_id, "Driving mission execution...");

        // Start the first task to transition mission to Executing
        if let Some(first_task_id) = planned.step_to_task.get(&1) {
            self.orchestrator
                .start_task(mission_id, first_task_id)
                .await
                .map_err(|e| zeus_core::Error::config(e.to_string()))?;
        }

        // Execute: use agent pool if available, otherwise sequential adaptive
        let (exec_result, replan_count) = if let Some(ref pool) = self.agent_pool {
            info!(mission_id = %mission_id, "Using AgentPool for parallel execution");
            let pool_result = pool
                .execute_plan(
                    &planned.plan,
                    &self.executor,
                    &self.llm,
                    &self.tools,
                    tool_executor,
                )
                .await;
            let exec_result = ExecutionResult {
                plan_status: if pool_result.all_succeeded() {
                    crate::planner::PlanStatus::Completed
                } else {
                    crate::planner::PlanStatus::Failed
                },
                step_results: pool_result.step_results,
                total_time_ms: pool_result.total_time_ms,
            };
            (exec_result, 0)
        } else {
            self.executor
                .execute_adaptive_cancellable(
                    &planned.plan,
                    &self.planner,
                    &self.llm,
                    &self.tools,
                    tool_executor,
                    self.max_replans,
                    cancelled,
                )
                .await?
        };

        // Map execution results back to Pantheon tasks
        self.sync_results_to_pantheon(mission_id, &exec_result, &planned.step_to_task)
            .await;

        let mission = self.orchestrator.get_mission(mission_id).await;
        let final_state = mission
            .as_ref()
            .map(|m| m.state.clone())
            .unwrap_or(MissionState::Failed);

        info!(
            mission_id = %mission_id,
            state = %final_state,
            replans = replan_count,
            time_ms = exec_result.total_time_ms,
            "Mission execution complete"
        );

        Ok(MissionResult {
            mission_id: mission_id.clone(),
            final_state,
            execution_result: exec_result,
            replan_count,
            spawn_outcomes: Vec::new(), // Populated by caller if spawner was active
        })
    }

    /// Convenience: plan + drive in one call.
    pub async fn launch_and_drive(
        &self,
        goal: &str,
        constraints: MissionConstraints,
        required_capabilities: Vec<String>,
        tool_executor: Option<&dyn ToolExecutor>,
    ) -> zeus_core::Result<MissionResult> {
        let planned = self
            .plan_mission(goal, constraints, required_capabilities)
            .await?;
        self.drive_mission(&planned, tool_executor).await
    }

    /// Sync Prometheus execution results back to Pantheon mission state.
    ///
    /// After each task completion/failure, triggers the checkpoint callback
    /// (if attached) to persist the mission state to SQLite for crash recovery.
    async fn sync_results_to_pantheon(
        &self,
        mission_id: &str,
        exec_result: &ExecutionResult,
        step_to_task: &HashMap<usize, String>,
    ) {
        for step_result in &exec_result.step_results {
            if let Some(task_id) = step_to_task.get(&step_result.step_id) {
                if step_result.success {
                    // Emit activity events for tool calls
                    for tc in &step_result.tool_calls_executed {
                        self.orchestrator.record_activity(
                            mission_id,
                            "prometheus",
                            &format!("tool_call: {}", tc.name),
                            serde_json::json!({
                                "tool": tc.name,
                                "success": tc.success,
                                "output_preview": truncate_str(&tc.output, 200),
                            }),
                        );
                    }

                    if let Err(e) = self
                        .orchestrator
                        .complete_task(mission_id, task_id, step_result.output.clone())
                        .await
                    {
                        warn!(
                            task_id = %task_id,
                            error = %e,
                            "Failed to complete task in Pantheon"
                        );
                    }
                } else {
                    let error = step_result
                        .error
                        .clone()
                        .unwrap_or_else(|| "Unknown error".to_string());

                    if let Err(e) = self
                        .orchestrator
                        .fail_task(mission_id, task_id, error)
                        .await
                    {
                        warn!(
                            task_id = %task_id,
                            error = %e,
                            "Failed to mark task as failed in Pantheon"
                        );
                    }
                }

                // Checkpoint: persist mission state after each task update
                if let Some(ref cp) = self.checkpointer {
                    cp.checkpoint(mission_id).await;
                    debug!(mission_id = %mission_id, task_id = %task_id, "Mission state checkpointed");
                }
            }
        }
    }

    /// Infer required capabilities from a plan's tool usage.
    pub fn infer_capabilities(plan: &Plan) -> Vec<String> {
        let mut caps: Vec<String> = plan.steps.iter().filter_map(|s| s.tool.clone()).collect();
        caps.sort();
        caps.dedup();
        caps
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// A mission that has been planned and assigned but not yet executed.
pub struct PlannedMission {
    pub mission_id: String,
    pub plan: Plan,
    pub step_to_task: HashMap<usize, String>,
}

/// The result of driving a mission to completion.
pub struct MissionResult {
    pub mission_id: String,
    pub final_state: MissionState,
    pub execution_result: ExecutionResult,
    pub replan_count: usize,
    /// Outcomes from any agents spawned during mission execution.
    pub spawn_outcomes: Vec<SpawnOutcome>,
}

impl MissionResult {
    pub fn succeeded(&self) -> bool {
        matches!(self.final_state, MissionState::Completed)
    }

    pub fn total_time_ms(&self) -> u64 {
        self.execution_result.total_time_ms
    }

    pub fn steps_succeeded(&self) -> usize {
        self.execution_result
            .step_results
            .iter()
            .filter(|r| r.success)
            .count()
    }

    pub fn steps_failed(&self) -> usize {
        self.execution_result
            .step_results
            .iter()
            .filter(|r| !r.success)
            .count()
    }

    pub fn total_tool_calls(&self) -> usize {
        self.execution_result
            .step_results
            .iter()
            .map(|r| r.tool_calls_executed.len())
            .sum()
    }

    /// Number of agents spawned during mission execution.
    pub fn spawns_total(&self) -> usize {
        self.spawn_outcomes.len()
    }

    /// Number of spawned agents that completed successfully.
    pub fn spawns_succeeded(&self) -> usize {
        self.spawn_outcomes.iter().filter(|o| o.success).count()
    }

    /// Collect all successful spawn outputs.
    pub fn spawn_outputs(&self) -> Vec<&str> {
        self.spawn_outcomes
            .iter()
            .filter(|o| o.success)
            .map(|o| o.output.as_str())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::StepResult;
    use crate::planner::{PlanStatus, Step, StepStatus};
    use zeus_orchestra::pantheon::TaskState;

    fn make_plan(steps: Vec<(usize, &str, Option<&str>, Vec<usize>)>) -> Plan {
        Plan {
            task: "test task".to_string(),
            steps: steps
                .into_iter()
                .map(|(id, desc, tool, deps)| Step {
                    id,
                    description: desc.to_string(),
                    tool: tool.map(|t| t.to_string()),
                    arguments: None,
                    dependencies: deps,
                    status: StepStatus::Pending,
                    output: None,
                })
                .collect(),
            status: PlanStatus::Created,
        }
    }

    #[test]
    fn test_plan_to_mission_tasks_empty() {
        let plan = Plan {
            task: "empty".to_string(),
            steps: vec![],
            status: PlanStatus::Created,
        };
        let (tasks, mapping) = plan_to_mission_tasks(&plan, "m-1");
        assert!(tasks.is_empty());
        assert!(mapping.is_empty());
    }

    #[test]
    fn test_plan_to_mission_tasks_single_step() {
        let plan = make_plan(vec![(1, "Do the thing", Some("shell"), vec![])]);
        let (tasks, mapping) = plan_to_mission_tasks(&plan, "m-1");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].mission_id, "m-1");
        assert_eq!(tasks[0].description, "Do the thing");
        assert!(tasks[0].dependencies.is_empty());
        assert!(mapping.contains_key(&1));
    }

    #[test]
    fn test_plan_to_mission_tasks_with_dependencies() {
        let plan = make_plan(vec![
            (1, "Read schema", Some("read_file"), vec![]),
            (2, "Write code", Some("write_file"), vec![1]),
            (3, "Run tests", Some("shell"), vec![2]),
        ]);
        let (tasks, mapping) = plan_to_mission_tasks(&plan, "m-2");
        assert_eq!(tasks.len(), 3);

        // Task 0 has no deps
        assert!(tasks[0].dependencies.is_empty());

        // Task 1 depends on task 0's ID
        assert_eq!(tasks[1].dependencies.len(), 1);
        assert_eq!(tasks[1].dependencies[0], mapping[&1]);

        // Task 2 depends on task 1's ID
        assert_eq!(tasks[2].dependencies.len(), 1);
        assert_eq!(tasks[2].dependencies[0], mapping[&2]);
    }

    #[test]
    fn test_plan_to_mission_tasks_multi_deps() {
        let plan = make_plan(vec![
            (1, "Step A", None, vec![]),
            (2, "Step B", None, vec![]),
            (3, "Step C", None, vec![1, 2]),
        ]);
        let (tasks, mapping) = plan_to_mission_tasks(&plan, "m-3");
        assert_eq!(tasks[2].dependencies.len(), 2);
        assert!(tasks[2].dependencies.contains(&mapping[&1]));
        assert!(tasks[2].dependencies.contains(&mapping[&2]));
    }

    #[test]
    fn test_plan_to_mission_tasks_preserves_descriptions() {
        let plan = make_plan(vec![
            (1, "Initialize database connection", Some("shell"), vec![]),
            (2, "Run migration scripts", Some("shell"), vec![1]),
        ]);
        let (tasks, _) = plan_to_mission_tasks(&plan, "m-4");
        assert_eq!(tasks[0].description, "Initialize database connection");
        assert_eq!(tasks[1].description, "Run migration scripts");
    }

    #[test]
    fn test_plan_to_mission_tasks_all_pending() {
        let plan = make_plan(vec![(1, "A", None, vec![]), (2, "B", None, vec![])]);
        let (tasks, _) = plan_to_mission_tasks(&plan, "m-5");
        for task in &tasks {
            assert_eq!(task.state, TaskState::Pending);
            assert!(task.assigned_to.is_none());
            assert!(task.result.is_none());
        }
    }

    #[test]
    fn test_best_agent_for_step_no_team() {
        let step = Step {
            id: 1,
            description: "test".to_string(),
            tool: Some("shell".to_string()),
            arguments: None,
            dependencies: vec![],
            status: StepStatus::Pending,
            output: None,
        };
        assert!(best_agent_for_step(&step, &[]).is_none());
    }

    #[test]
    fn test_best_agent_for_step_matching_worker() {
        use zeus_orchestra::pantheon::TeamMember;

        let team = vec![
            TeamMember {
                agent_id: "coord-1".to_string(),
                name: "Zeus-C".to_string(),
                role: AgentRole::Coordinator,
                capabilities: vec!["coordinate".to_string()],
                joined_at: chrono::Utc::now(),
            },
            TeamMember {
                agent_id: "worker-1".to_string(),
                name: "Zeus-W1".to_string(),
                role: AgentRole::Worker,
                capabilities: vec!["code".to_string(), "shell".to_string()],
                joined_at: chrono::Utc::now(),
            },
        ];

        let step = Step {
            id: 1,
            description: "run command".to_string(),
            tool: Some("shell".to_string()),
            arguments: None,
            dependencies: vec![],
            status: StepStatus::Pending,
            output: None,
        };

        let agent = best_agent_for_step(&step, &team);
        assert_eq!(agent, Some("worker-1".to_string()));
    }

    #[test]
    fn test_best_agent_for_step_no_matching_falls_back_to_worker() {
        use zeus_orchestra::pantheon::TeamMember;

        let team = vec![
            TeamMember {
                agent_id: "coord-1".to_string(),
                name: "Zeus-C".to_string(),
                role: AgentRole::Coordinator,
                capabilities: vec!["coordinate".to_string()],
                joined_at: chrono::Utc::now(),
            },
            TeamMember {
                agent_id: "worker-1".to_string(),
                name: "Zeus-W1".to_string(),
                role: AgentRole::Worker,
                capabilities: vec!["code".to_string()],
                joined_at: chrono::Utc::now(),
            },
        ];

        let step = Step {
            id: 1,
            description: "deploy".to_string(),
            tool: Some("deploy_tool".to_string()),
            arguments: None,
            dependencies: vec![],
            status: StepStatus::Pending,
            output: None,
        };

        // No worker has "deploy_tool", falls back to first worker
        let agent = best_agent_for_step(&step, &team);
        assert_eq!(agent, Some("worker-1".to_string()));
    }

    #[test]
    fn test_best_agent_for_step_no_tool_gets_worker() {
        use zeus_orchestra::pantheon::TeamMember;

        let team = vec![
            TeamMember {
                agent_id: "coord-1".to_string(),
                name: "Zeus-C".to_string(),
                role: AgentRole::Coordinator,
                capabilities: vec!["coordinate".to_string()],
                joined_at: chrono::Utc::now(),
            },
            TeamMember {
                agent_id: "worker-1".to_string(),
                name: "Zeus-W1".to_string(),
                role: AgentRole::Worker,
                capabilities: vec!["code".to_string()],
                joined_at: chrono::Utc::now(),
            },
        ];

        let step = Step {
            id: 1,
            description: "think about it".to_string(),
            tool: None,
            arguments: None,
            dependencies: vec![],
            status: StepStatus::Pending,
            output: None,
        };

        let agent = best_agent_for_step(&step, &team);
        assert_eq!(agent, Some("worker-1".to_string()));
    }

    #[test]
    fn test_best_agent_only_coordinator() {
        use zeus_orchestra::pantheon::TeamMember;

        let team = vec![TeamMember {
            agent_id: "coord-1".to_string(),
            name: "Zeus-C".to_string(),
            role: AgentRole::Coordinator,
            capabilities: vec!["coordinate".to_string()],
            joined_at: chrono::Utc::now(),
        }];

        let step = Step {
            id: 1,
            description: "do work".to_string(),
            tool: Some("shell".to_string()),
            arguments: None,
            dependencies: vec![],
            status: StepStatus::Pending,
            output: None,
        };

        // No workers, falls back to coordinator
        let agent = best_agent_for_step(&step, &team);
        assert_eq!(agent, Some("coord-1".to_string()));
    }

    #[test]
    fn test_infer_capabilities_empty() {
        let plan = Plan {
            task: "test".to_string(),
            steps: vec![],
            status: PlanStatus::Created,
        };
        let caps = MissionDriver::infer_capabilities(&plan);
        assert!(caps.is_empty());
    }

    #[test]
    fn test_infer_capabilities_dedupes() {
        let plan = make_plan(vec![
            (1, "Step 1", Some("shell"), vec![]),
            (2, "Step 2", Some("read_file"), vec![]),
            (3, "Step 3", Some("shell"), vec![]),
            (4, "Step 4", None, vec![]),
        ]);
        let caps = MissionDriver::infer_capabilities(&plan);
        assert_eq!(caps, vec!["read_file", "shell"]);
    }

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 100), "hello");
    }

    #[test]
    fn test_truncate_str_long() {
        let s = "a".repeat(300);
        let result = truncate_str(&s, 100);
        assert!(result.len() <= 103);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_str_emoji() {
        let s = "Hello world! Here is some text with an emoji at the end 🎉🎉🎉🎉🎉";
        let result = truncate_str(s, 60);
        assert!(result.ends_with("..."));
        // Should not panic on multi-byte emoji boundary
    }

    #[test]
    fn test_mission_result_helpers() {
        let result = MissionResult {
            mission_id: "m-1".to_string(),
            final_state: MissionState::Completed,
            execution_result: ExecutionResult {
                plan_status: PlanStatus::Completed,
                step_results: vec![
                    StepResult {
                        step_id: 1,
                        success: true,
                        output: "done".to_string(),
                        error: None,
                        retries: 0,
                        tool_calls_executed: vec![
                            crate::executor::StepToolCall {
                                name: "shell".to_string(),
                                call_id: "tc1".to_string(),
                                arguments: serde_json::json!({}),
                                success: true,
                                output: "ok".to_string(),
                            },
                            crate::executor::StepToolCall {
                                name: "read_file".to_string(),
                                call_id: "tc2".to_string(),
                                arguments: serde_json::json!({}),
                                success: true,
                                output: "contents".to_string(),
                            },
                        ],
                    },
                    StepResult {
                        step_id: 2,
                        success: false,
                        output: String::new(),
                        error: Some("failed".to_string()),
                        retries: 2,
                        tool_calls_executed: vec![],
                    },
                ],
                total_time_ms: 5000,
            },
            replan_count: 0,
            spawn_outcomes: Vec::new(),
        };

        assert!(result.succeeded());
        assert_eq!(result.total_time_ms(), 5000);
        assert_eq!(result.steps_succeeded(), 1);
        assert_eq!(result.steps_failed(), 1);
        assert_eq!(result.total_tool_calls(), 2);
    }

    #[test]
    fn test_mission_result_failed() {
        let result = MissionResult {
            mission_id: "m-2".to_string(),
            final_state: MissionState::Failed,
            execution_result: ExecutionResult {
                plan_status: PlanStatus::Failed,
                step_results: vec![],
                total_time_ms: 100,
            },
            replan_count: 2,
            spawn_outcomes: Vec::new(),
        };

        assert!(!result.succeeded());
        assert_eq!(result.steps_succeeded(), 0);
        assert_eq!(result.steps_failed(), 0);
        assert_eq!(result.total_tool_calls(), 0);
    }
}
