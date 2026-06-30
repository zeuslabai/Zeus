//! Workflow API handlers

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{info, warn};
use zeus_llm::LlmClient;

use crate::SharedState;
use super::ExecutionMode;

#[derive(Debug, Deserialize)]
pub struct WorkflowRequest {
    pub message: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub step_delay_ms: Option<u64>,
}

/// POST /v1/workflows — create a workflow from natural language, execute as DAG
///
/// Uses the LLM Planner to decompose the message into steps, analyzes them into
/// a TaskDAG, returns 202 Accepted immediately, then executes in background.
/// Clients can watch progress via WebSocket `prometheus_watch` with the returned workflow_id.
pub async fn workflow_from_chat(
    State(state): State<SharedState>,
    Json(body): Json<WorkflowRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let state_guard = state.read().await;

    // 1. Build LLM client (required for planning)
    let llm = LlmClient::from_config(&state_guard.config).map_err(|e| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("LLM required for workflow planning: {e}"),
        )
    })?;

    // 2. Get tool schemas
    let tool_schemas = state_guard.tools.schemas();

    // 3. Call Planner to decompose message
    let planner = zeus_prometheus::planner::Planner::new();
    let plan = planner
        .create_plan(&body.message, &llm, &tool_schemas)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Planning failed: {e}"),
            )
        })?;
    let goal = plan.task.clone();

    // 4. Analyze into TaskDAG
    let dag = state_guard.strategic_planner.analyze(&plan).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("DAG analysis failed: {e}"),
        )
    })?;

    let workflow_id = format!("wf-{}", uuid::Uuid::new_v4());
    let parallel_groups = dag.parallel_groups();
    let critical_path = dag.critical_path();
    let estimated_total_ms = dag.estimated_total_ms();
    let total_steps = dag.nodes.len();

    // 5. Determine execution mode
    let step_delay_ms = body.step_delay_ms.unwrap_or(200);
    let mode_str = body.mode.as_deref().unwrap_or("agent");
    let mode = if mode_str == "simulate" {
        ExecutionMode::Simulated { step_delay_ms }
    } else if mode_str == "llm" {
        match LlmClient::from_config(&state_guard.config) {
            Ok(_) => ExecutionMode::Llm(Box::new(state_guard.config.clone())),
            Err(_) => ExecutionMode::Simulated { step_delay_ms },
        }
    } else {
        match LlmClient::from_config(&state_guard.config) {
            Ok(_) => ExecutionMode::Agent(state.clone()),
            Err(_) => ExecutionMode::Simulated { step_delay_ms },
        }
    };
    let mode_label = match &mode {
        ExecutionMode::Agent(_) => "agent",
        ExecutionMode::Llm(_) => "llm",
        ExecutionMode::Simulated { .. } => "simulated",
    };

    // 6. Build step summaries
    let steps: Vec<Value> = dag
        .nodes
        .iter()
        .map(|(id, node)| {
            json!({
                "id": id,
                "description": node.description,
                "tool": node.tool,
            })
        })
        .collect();

    // 6b. Create workflow state for tracking
    let workflow_nodes: Vec<crate::WorkflowNodeStatus> = dag
        .nodes
        .iter()
        .map(|(id, _node)| {
            let deps = dag
                .reverse_edges
                .get(id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|dep_id| dep_id.to_string())
                .collect();
            crate::WorkflowNodeStatus {
                node_id: id.to_string(),
                status: "pending".to_string(),
                started_at: None,
                completed_at: None,
                result: None,
                error: None,
                dependencies: deps,
            }
        })
        .collect();

    let mut workflow_state = crate::WorkflowState::new(
        workflow_id.clone(),
        body.message.clone(),
        workflow_nodes,
    );
    workflow_state.status = "pending".to_string();

    let broadcast = state_guard.plan_broadcast.clone();
    let workflow_states = state_guard.workflow_states.clone();
    drop(state_guard);

    // Store workflow state
    workflow_states.insert(workflow_id.clone(), workflow_state);

    // 7. Spawn background executor
    let exec_id = workflow_id.clone();
    let exec_goal = goal.clone();
    tokio::spawn(async move {
        execute_plan_steps(dag, &exec_id, &exec_goal, mode, &broadcast).await;
    });

    info!("Workflow {} created from REST: {}", workflow_id, body.message);

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "workflow_id": workflow_id,
            "goal": goal,
            "steps": steps,
            "total_steps": total_steps,
            "parallel_groups": parallel_groups,
            "critical_path": critical_path,
            "estimated_total_ms": estimated_total_ms,
            "mode": mode_label,
            "watch_via": format!("WebSocket PrometheusWatch with plan_id: {}", workflow_id),
        })),
    ))
}

/// GET /v1/workflows/:id — get workflow status
///
/// Returns real-time workflow execution status with node progress,
/// dependencies, and completion percentage.
pub async fn get_workflow(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;

    if let Some(workflow) = state_guard.workflow_states.get(&id) {
        Ok(Json(serde_json::to_value(workflow.clone()).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to serialize workflow: {}", e),
            )
        })?))
    } else {
        Err((
            StatusCode::NOT_FOUND,
            format!("Workflow {} not found", id),
        ))
    }
}

/// GET /v1/workflows — list all workflows
///
/// Returns a list of all workflow executions with their current status
pub async fn list_workflows(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;

    let workflows: Vec<Value> = state_guard
        .workflow_states
        .iter()
        .map(|entry| {
            let workflow = entry.value();
            json!({
                "workflow_id": workflow.workflow_id,
                "status": workflow.status,
                "message": workflow.message,
                "progress_percentage": workflow.progress_percentage,
                "total_nodes": workflow.total_nodes,
                "completed_nodes": workflow.completed_nodes,
                "failed_nodes": workflow.failed_nodes,
                "created_at": workflow.created_at,
                "started_at": workflow.started_at,
                "completed_at": workflow.completed_at,
            })
        })
        .collect();

    Json(json!({
        "workflows": workflows,
        "total": workflows.len(),
    }))
}

/// GET /v1/workflows/:id/artifacts — List artifacts for an orchestration session.
pub async fn workflow_artifacts(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let session = state_guard
        .orchestration
        .get(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Session not found".to_string()))?;
    drop(state_guard);

    let artifacts: Vec<Value> = session
        .artifacts
        .iter()
        .filter_map(|a| serde_json::to_value(a).ok())
        .collect();

    Ok(Json(json!({
        "session_id": id,
        "artifacts": artifacts,
        "total": session.artifacts.len(),
    })))
}

/// GET /v1/workflows/:id/download — Download deliverable ZIP for an orchestration.
pub async fn workflow_download(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<axum::response::Response<axum::body::Body>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let session = state_guard
        .orchestration
        .get(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Session not found".to_string()))?;
    drop(state_guard);

    if let zeus_prometheus::orchestrate::OrchestrationPhase::Delivered {
        ref artifact_path, ..
    } = session.phase
    {
        if !artifact_path.is_empty() {
            let path = std::path::Path::new(artifact_path);
            if path.exists() {
                let bytes = tokio::fs::read(path).await.map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to read artifact: {e}"),
                    )
                })?;
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("deliverable.zip");
                return Ok(axum::response::Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "application/zip")
                    .header(
                        "Content-Disposition",
                        format!("attachment; filename=\"{filename}\""),
                    )
                    .body(axum::body::Body::from(bytes))
                    .unwrap());
            }
        }
    }

    Ok(axum::response::Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(axum::body::Body::from("No deliverable available"))
        .unwrap())
}

/// Background executor: walks DAG in topological order, broadcasting step updates.
///
/// In LLM mode, each step description is sent to the LLM and the response is used as output.
/// In simulated mode, steps just sleep and succeed.
/// After all steps: broadcast `prometheus_complete`.
pub(crate) async fn execute_plan_steps(
    mut dag: zeus_prometheus::strategic::TaskDAG,
    plan_id: &str,
    goal: &str,
    mode: ExecutionMode,
    broadcast: &crate::websocket::PlanBroadcast,
) {
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
                completed_summaries
                    .push(format!("Step {step_id} ({description}): {output}"));
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
}

/// Execute a single plan step. Returns Ok(output) or Err(error_message).
async fn execute_single_step(
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
        (None, ExecutionMode::Llm(_)) => {
            Ok(format!("Simulated (LLM unavailable): {description}"))
        }
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

    match client
        .complete(&messages, &[], Some(&system_prompt))
        .await
    {
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
