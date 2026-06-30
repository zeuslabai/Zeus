use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::warn;
use zeus_llm::LlmClient;

use crate::SharedState;
use crate::handlers::build_plan_from_body;

pub async fn prometheus_create_plan(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let plan = build_plan_from_body(&body)?;
    let goal = plan.task.clone();

    let state_guard = state.read().await;
    let dag = state_guard
        .strategic_planner()
        .analyze(&plan)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Plan analysis failed: {e}"),
            )
        })?;

    let plan_id = uuid::Uuid::new_v4().to_string();
    let parallel_groups = dag.parallel_groups();
    let critical_path = dag.critical_path();
    let estimated_ms = dag.estimated_total_ms();

    // Store in PlanStore for later retrieval via GET /v1/prometheus/plan/:id
    let stored = crate::plan_store::StoredPlan {
        plan_id: plan_id.clone(),
        goal: goal.clone(),
        status: crate::plan_store::PlanStoreStatus::Created,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        dag: serde_json::to_value(&dag).unwrap_or_default(),
        node_count: dag.nodes.len(),
        topological_order: dag.topological_order.clone(),
        parallel_groups: parallel_groups.clone(),
        critical_path: critical_path.clone(),
        estimated_total_ms: estimated_ms,
        execution_result: None,
        execution_mode: None,
    };
    state_guard.plan_store.store(stored);

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "plan_id": plan_id,
            "goal": goal,
            "nodes": dag.nodes.len(),
            "parallel_groups": parallel_groups,
            "critical_path": critical_path,
            "estimated_total_ms": estimated_ms,
            "topological_order": dag.topological_order,
            "dag": serde_json::to_value(&dag).unwrap_or_default(),
        })),
    ))
}

/// GET /v1/prometheus/plan/:id — get plan status (backed by PlanStore)
pub async fn prometheus_get_plan(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    match state.plan_store.get(&id) {
        Some(plan) => Ok(Json(serde_json::to_value(&plan).unwrap_or_default())),
        None => Err((StatusCode::NOT_FOUND, format!("Plan not found: {}", id))),
    }
}

/// Execution mode for the Prometheus plan executor.
#[derive(Clone)]
pub enum ExecutionMode {
    /// Agent execution: each step spawns a dynamic agent via the AgentRegistry,
    /// sends the step task, and collects the response. Agents have full tool access.
    Agent(crate::SharedState),
    /// Direct LLM execution: each step is sent to the LLM as a prompt (no tools).
    Llm(Box<zeus_core::Config>),
    /// Simulated execution: steps just sleep for `step_delay_ms` and succeed.
    Simulated { step_delay_ms: u64 },
}

impl std::fmt::Debug for ExecutionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Agent(_) => write!(f, "Agent(<SharedState>)"),
            Self::Llm(config) => f.debug_tuple("Llm").field(config).finish(),
            Self::Simulated { step_delay_ms } => f
                .debug_struct("Simulated")
                .field("step_delay_ms", step_delay_ms)
                .finish(),
        }
    }
}

/// POST /v1/prometheus/execute — execute a plan asynchronously
///
/// Builds a DAG from the plan, returns the `plan_id` immediately (202 Accepted),
/// then spawns a background task that walks steps in topological order.
///
/// **Execution modes:**
/// - **LLM mode** (default): each step description is sent to the configured LLM,
///   and the response is captured as step output.
/// - **Simulated mode**: set `"simulate": true` in the request body to skip LLM calls
///   and use sleep-based simulation. Also used as automatic fallback when no LLM is configured.
///
/// Clients subscribe via WebSocket `PrometheusWatch { plan_id }`.
pub async fn prometheus_execute(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let plan = build_plan_from_body(&body)?;
    let goal = plan.task.clone();
    let plan_id = uuid::Uuid::new_v4().to_string();

    let simulate = body
        .get("simulate")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let step_delay_ms = body
        .get("step_delay_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(200);

    let state_guard = state.read().await;
    let dag = state_guard
        .strategic_planner()
        .analyze(&plan)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Plan analysis failed: {e}"),
            )
        })?;

    let broadcast = state_guard.plan_broadcast.clone();

    // Request can specify mode: "agent" (default), "llm", or "simulate"
    let mode_str = body.get("mode").and_then(|v| v.as_str()).unwrap_or("agent");

    let mode = if simulate || mode_str == "simulate" {
        ExecutionMode::Simulated { step_delay_ms }
    } else if mode_str == "llm" {
        // Direct LLM mode (no tool execution)
        match LlmClient::from_config(&state_guard.config) {
            Ok(_) => ExecutionMode::Llm(Box::new(state_guard.config.clone())),
            Err(e) => {
                warn!("LLM not available, falling back to simulated execution: {e}");
                ExecutionMode::Simulated { step_delay_ms }
            }
        }
    } else {
        // Agent mode (default): spawns per-step agents with full tool access
        match LlmClient::from_config(&state_guard.config) {
            Ok(_) => ExecutionMode::Agent(state.clone()),
            Err(e) => {
                warn!("LLM not available for agents, falling back to simulated: {e}");
                ExecutionMode::Simulated { step_delay_ms }
            }
        }
    };

    let total_steps = dag.nodes.len();
    let topo_order = dag.topological_order.clone();
    let estimated_total_ms = dag.estimated_total_ms();
    let mode_label = match &mode {
        ExecutionMode::Agent(_) => "agent",
        ExecutionMode::Llm(_) => "llm",
        ExecutionMode::Simulated { .. } => "simulated",
    };

    // Store in PlanStore for retrieval via GET /v1/prometheus/plan/:id
    let stored = crate::plan_store::StoredPlan {
        plan_id: plan_id.clone(),
        goal: goal.clone(),
        status: crate::plan_store::PlanStoreStatus::Executing,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        dag: serde_json::to_value(&dag).unwrap_or_default(),
        node_count: total_steps,
        topological_order: topo_order.clone(),
        parallel_groups: dag.parallel_groups(),
        critical_path: dag.critical_path(),
        estimated_total_ms,
        execution_result: None,
        execution_mode: Some(mode_label.to_string()),
    };
    state_guard.plan_store.store(stored);
    drop(state_guard);

    // Spawn background executor — returns immediately
    let exec_plan_id = plan_id.clone();
    let exec_goal = goal.clone();
    tokio::spawn(async move {
        let _ = execute_plan_steps(dag, &exec_plan_id, &exec_goal, mode, &broadcast).await;
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "plan_id": plan_id,
            "status": "accepted",
            "goal": goal,
            "total_steps": total_steps,
            "topological_order": topo_order,
            "estimated_total_ms": estimated_total_ms,
            "mode": mode_label,
        })),
    ))
}

/// Result of running plan steps — carries the execution transcript and timing.
pub(crate) struct PlanExecutionResult {
    /// Per-step transcript lines (e.g. "Step 1 (setup): output text")
    pub transcript: Vec<String>,
    /// Wall-clock duration in milliseconds
    pub duration_ms: u64,
}

/// Background executor: walks DAG in topological order, broadcasting step updates.
///
/// In LLM mode, each step description is sent to the LLM and the response is used as output.
/// In simulated mode, steps just sleep and succeed.
/// After all steps: broadcast `prometheus_complete`.
pub async fn execute_plan_steps(
    mut dag: zeus_prometheus::strategic::TaskDAG,
    plan_id: &str,
    goal: &str,
    mode: ExecutionMode,
    broadcast: &crate::websocket::PlanBroadcast,
) -> PlanExecutionResult {
    let total_steps = dag.nodes.len();
    let topo_order = dag.topological_order.clone();
    let start = std::time::Instant::now();
    let mut steps_completed: usize = 0;
    let mut steps_failed: usize = 0;

    // Build LLM client once (if in LLM mode)
    let llm = match &mode {
        ExecutionMode::Llm(config) => match LlmClient::from_config(config) {
            Ok(client) => Some(client),
            Err(e) => {
                warn!("Failed to create LLM client in executor: {e}");
                None
            }
        },
        ExecutionMode::Agent(_) | ExecutionMode::Simulated { .. } => None,
    };

    // Build prior step summaries for context accumulation
    let mut completed_summaries: Vec<String> = Vec::new();

    for (idx, step_id) in topo_order.iter().enumerate() {
        let description = dag
            .nodes
            .get(step_id)
            .map(|n| n.description.clone())
            .unwrap_or_default();
        let tool = dag
            .nodes
            .get(step_id)
            .and_then(|n| n.tool.clone())
            .unwrap_or_else(|| "none".to_string());

        // 1. Mark running + broadcast
        dag.set_running(*step_id);
        let progress_pct = if total_steps > 0 {
            (idx as f64 / total_steps as f64) * 100.0
        } else {
            0.0
        };
        broadcast.send_step_update(crate::websocket::PlanStepUpdate {
            plan_id: plan_id.to_string(),
            step_id: *step_id,
            status: "running".to_string(),
            progress_pct,
            output: format!("Starting: {description} (tool: {tool})"),
        });

        // 2. Execute the step
        let result = execute_single_step(
            &llm,
            &mode,
            goal,
            *step_id,
            &description,
            &tool,
            &completed_summaries,
        )
        .await;

        // 3. Broadcast result
        let done_pct = if total_steps > 0 {
            ((idx + 1) as f64 / total_steps as f64) * 100.0
        } else {
            100.0
        };

        match result {
            Ok(output) => {
                dag.complete_node(*step_id);
                steps_completed += 1;
                completed_summaries.push(format!("Step {step_id} ({description}): {output}"));
                broadcast.send_step_update(crate::websocket::PlanStepUpdate {
                    plan_id: plan_id.to_string(),
                    step_id: *step_id,
                    status: "done".to_string(),
                    progress_pct: done_pct,
                    output,
                });
            }
            Err(error) => {
                dag.fail_node(*step_id);
                steps_failed += 1;
                completed_summaries
                    .push(format!("Step {step_id} ({description}): FAILED — {error}"));
                broadcast.send_step_update(crate::websocket::PlanStepUpdate {
                    plan_id: plan_id.to_string(),
                    step_id: *step_id,
                    status: "failed".to_string(),
                    progress_pct: done_pct,
                    output: error,
                });
            }
        }
    }

    // 4. Broadcast completion
    let duration_ms = start.elapsed().as_millis() as u64;
    let final_status = if steps_failed == 0 {
        "completed"
    } else if steps_completed > 0 {
        "partial"
    } else {
        "failed"
    };
    broadcast.send_complete(crate::websocket::PlanComplete {
        plan_id: plan_id.to_string(),
        status: final_status.to_string(),
        steps_completed,
        steps_failed,
        duration_ms,
    });

    PlanExecutionResult {
        transcript: completed_summaries,
        duration_ms,
    }
}

/// Execute a single plan step. Returns Ok(output) or Err(error_message).
pub async fn execute_single_step(
    llm: &Option<LlmClient>,
    mode: &ExecutionMode,
    goal: &str,
    step_id: usize,
    description: &str,
    tool: &str,
    prior_steps: &[String],
) -> Result<String, String> {
    match (llm, mode) {
        // Agent execution: spawn a dynamic agent with full tool access
        (_, ExecutionMode::Agent(state)) => {
            execute_step_with_agent(state, goal, step_id, description, tool, prior_steps).await
        }
        // Direct LLM execution (no tools)
        (Some(client), ExecutionMode::Llm(_)) => {
            execute_step_with_llm(client, goal, step_id, description, tool, prior_steps).await
        }
        // Simulated execution (fallback)
        (_, ExecutionMode::Simulated { step_delay_ms }) => {
            tokio::time::sleep(std::time::Duration::from_millis(*step_delay_ms)).await;
            Ok(format!("Simulated: {description}"))
        }
        // LLM mode but client creation failed
        (None, ExecutionMode::Llm(_)) => Ok(format!("Simulated (LLM unavailable): {description}")),
    }
}

/// Execute a plan step by spawning a dynamic agent via the AgentRegistry.
///
/// The agent gets full tool access (read_file, write_file, shell, etc.) and a
/// goals context that includes the plan goal, prior step results, and suggested tool.
/// After execution, the agent is unregistered from the registry.
async fn execute_step_with_agent(
    state: &crate::SharedState,
    goal: &str,
    step_id: usize,
    description: &str,
    tool: &str,
    prior_steps: &[String],
) -> Result<String, String> {
    let agent_id = format!("prometheus-step-{step_id}-{}", uuid::Uuid::new_v4());

    let prior_context = if prior_steps.is_empty() {
        String::new()
    } else {
        format!(
            "\n\nPreviously completed steps:\n{}",
            prior_steps.join("\n")
        )
    };

    let goals_context = format!(
        "You are an agent executing step {step_id} of a multi-step plan.\n\
         Overall goal: {goal}\n\
         Your task: {description}\n\
         Suggested tool: {tool}\n\
         {prior_context}\n\n\
         Use your tools to accomplish this task. Be thorough but concise.\n\
         If the suggested tool is relevant, prefer using it.\n\
         Return a clear summary of what you accomplished."
    );

    // 1. Spawn dynamic agent via registry
    {
        let mut state_guard = state.write().await;
        state_guard
            .agent_registry
            .spawn_dynamic(
                &agent_id,
                &format!("Step {step_id} Agent"),
                Some(goals_context),
            )
            .await
            .map_err(|e| format!("Agent spawn failed: {e}"))?;
        // Track compute quota for the ephemeral step agent.
        state_guard.register_agent_compute(&agent_id, 1.0).await;
    }

    // 2. Get agent Arc (clone outside the lock)
    let agent_arc = {
        let state_guard = state.read().await;
        match state_guard.agent_registry.get(&agent_id) {
            Some(instance) => instance.agent.clone(),
            None => return Err(format!("Agent {agent_id} disappeared after spawn")),
        }
    };

    // 3. Run the step task on the agent (outside state lock)
    let result = {
        let mut agent = agent_arc.write().await;
        agent.run(description).await
    };

    // 4. Clean up: unregister the ephemeral agent + release its compute quota.
    {
        let mut state_guard = state.write().await;
        state_guard.agent_registry.unregister(&agent_id);
        state_guard.deregister_agent_compute(&agent_id).await;
    }

    match result {
        Ok(response) => {
            if response.is_empty() {
                Ok(format!("Step {step_id} completed (empty agent response)"))
            } else {
                Ok(response)
            }
        }
        Err(e) => Err(format!("Agent error: {e}")),
    }
}

/// Execute a plan step by calling the LLM directly (no tool execution).
async fn execute_step_with_llm(
    client: &LlmClient,
    goal: &str,
    step_id: usize,
    description: &str,
    tool: &str,
    prior_steps: &[String],
) -> Result<String, String> {
    let system_prompt = format!(
        "You are executing step {step_id} of a multi-step plan.\n\
         Overall goal: {goal}\n\
         Current step: {description}\n\
         Suggested tool: {tool}\n\n\
         {prior_context}\
         Provide a clear, concise response completing this step. \
         Focus on actionable output.",
        prior_context = if prior_steps.is_empty() {
            String::new()
        } else {
            format!(
                "Previously completed steps:\n{}\n\n",
                prior_steps.join("\n")
            )
        },
    );

    let messages = vec![zeus_core::Message::user(description)];

    match client.complete(&messages, &[], Some(&system_prompt)).await {
        Ok(response) => {
            if response.content.is_empty() {
                Ok(format!("Step {step_id} completed (empty LLM response)"))
            } else {
                Ok(response.content)
            }
        }
        Err(e) => Err(format!("LLM error: {e}")),
    }
}

/// GET /v1/prometheus/state — global agent state snapshot
pub async fn prometheus_state(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let agents = state_guard.global_state().list_agents().await;
    let idle = state_guard.global_state().list_idle().await;
    let total = state_guard.global_state().agent_count().await;

    let agent_list: Vec<Value> = agents
        .into_iter()
        .map(|a| serde_json::to_value(a).unwrap_or_default())
        .collect();

    Json(json!({
        "total_agents": total,
        "idle_agents": idle.len(),
        "agents": agent_list,
    }))
}

// ============================================================================
// Workflow Endpoints (chat -> DAG execution)
// ============================================================================

/// GET /v1/replication/lineage — view agent lineage tree
pub async fn replication_lineage(
    State(state): State<SharedState>,
) -> Json<Value> {
    let state_guard = state.read().await;
    let mgr = state_guard.replication_manager.read().await;
    let lineage = mgr.lineage();
    Json(json!({
        "total_agents": lineage.total_agents(),
    }))
}

/// POST /v1/replication/replicate — trigger Conway-style agent replication
pub async fn replication_replicate(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let parent_id = body.get("parent_id").and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "parent_id required".to_string()))?;
    let role = body.get("role").and_then(|v| v.as_str()).unwrap_or("worker").to_string();
    let task = body.get("task").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let capabilities: Vec<String> = body.get("capabilities")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let tools: Vec<String> = body.get("tools")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let parent_balance = body.get("parent_balance").and_then(|v| v.as_u64()).unwrap_or(0);
    let extra_funding = body.get("extra_funding").and_then(|v| v.as_u64()).unwrap_or(0);

    let state_guard = state.read().await;
    let mut mgr = state_guard.replication_manager.write().await;

    let request = zeus_prometheus::ReplicationRequest {
        parent_id: parent_id.to_string(),
        role,
        task,
        capabilities,
        tools,
        system_prompt: body.get("system_prompt").and_then(|v| v.as_str()).map(String::from),
        extra_funding,
    };

    match mgr.replicate(request, parent_balance) {
        Ok(result) => Ok(Json(json!({
            "child_id": result.child_id,
            "birth_cost": result.birth_cost,
            "total_cost": result.total_cost,
            "child_funding": result.child_funding,
            "child_wallet_dir": result.child_wallet_dir,
        }))),
        Err(e) => Err((StatusCode::CONFLICT, format!("Replication failed: {}", e))),
    }
}

/// Request body for POST /v1/workflows
#[derive(Debug, Deserialize)]
pub struct WorkflowRequest {
    pub message: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub step_delay_ms: Option<u64>,
}

/// POST /v1/system/auto-tune — run MetaLoop config optimizer
///
/// Tests config mutations against benchmarks and keeps changes that improve
/// performance. Runs up to `max_iterations` experiments (default: 5).
/// Spawns on a blocking thread because BenchmarkStore uses rusqlite (non-Send).
pub async fn auto_tune(
    State(_state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let max_iters = body.get("max_iterations")
        .and_then(|v| v.as_u64())
        .unwrap_or(5) as u32;

    // MetaLoop contains rusqlite::Connection (non-Send) via BenchmarkStore,
    // so we run it on a dedicated blocking thread with its own tokio runtime.
    let report = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("Runtime: {}", e))?;
        rt.block_on(async {
            let experiment = zeus_prometheus::experiment::ConfigExperiment::new()
                .map_err(|e| format!("Experiment init: {}", e))?;
            let proposer = Box::new(zeus_prometheus::RandomProposer::new(max_iters));
            let mut meta_loop = zeus_prometheus::MetaLoop::new(experiment, proposer);
            meta_loop.run(max_iters).await
                .map_err(|e| format!("Auto-tune: {}", e))
        })
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Task join: {}", e)))?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let iters: Vec<Value> = report.iterations.iter().map(|i| {
        let outcome_str = match &i.outcome {
            zeus_prometheus::experiment::ExperimentOutcome::Kept { .. } => "kept",
            zeus_prometheus::experiment::ExperimentOutcome::Reverted { .. } => "reverted",
            zeus_prometheus::experiment::ExperimentOutcome::Failed { .. } => "failed",
        };
        json!({
            "iter": i.iter,
            "key": i.change.key,
            "value": i.change.value,
            "description": i.change.description,
            "outcome": outcome_str,
        })
    }).collect();

    Ok(Json(json!({
        "kept": report.kept_count,
        "reverted": report.reverted_count,
        "failed": report.failed_count,
        "total_iterations": report.iterations.len(),
        "proposer": report.proposer_name,
        "iterations": iters,
    })))
}
