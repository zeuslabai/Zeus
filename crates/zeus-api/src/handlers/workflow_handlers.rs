use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use tracing::info;
use zeus_llm::LlmClient;

use crate::SharedState;
use crate::handlers::prometheus_handlers::{ExecutionMode, WorkflowRequest, execute_plan_steps};

pub(crate) fn build_plan_from_body(
    body: &Value,
) -> Result<zeus_prometheus::planner::Plan, (StatusCode, String)> {
    let goal = body
        .get("goal")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'goal' field".to_string()))?;

    let steps_val = body.get("steps").and_then(|v| v.as_array());

    let steps = if let Some(steps_arr) = steps_val {
        steps_arr
            .iter()
            .enumerate()
            .map(|(i, step_val)| {
                let desc = step_val
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("untitled step")
                    .to_string();
                let tool = step_val
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let deps: Vec<usize> = step_val
                    .get("dependencies")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_u64().map(|n| n as usize))
                            .collect()
                    })
                    .unwrap_or_default();
                zeus_prometheus::planner::Step {
                    id: i,
                    description: desc,
                    tool,
                    arguments: None,
                    dependencies: deps,
                    status: zeus_prometheus::planner::StepStatus::Pending,
                    output: None,
                }
            })
            .collect()
    } else {
        vec![zeus_prometheus::planner::Step {
            id: 0,
            description: goal.to_string(),
            tool: None,
            arguments: None,
            dependencies: vec![],
            status: zeus_prometheus::planner::StepStatus::Pending,
            output: None,
        }]
    };

    Ok(zeus_prometheus::planner::Plan {
        task: goal.to_string(),
        steps,
        status: zeus_prometheus::planner::PlanStatus::Created,
    })
}

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
    let dag = state_guard
        .strategic_planner()
        .analyze(&plan)
        .map_err(|e| {
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
        .keys()
        .map(|id| {
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

    let mut workflow_state =
        crate::WorkflowState::new(workflow_id.clone(), body.message.clone(), workflow_nodes);
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
        let _ = execute_plan_steps(dag, &exec_id, &exec_goal, mode, &broadcast).await;
    });

    info!(
        "Workflow {} created from REST: {}",
        workflow_id, body.message
    );

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
        Ok(Json(serde_json::to_value(workflow.clone()).map_err(
            |e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to serialize workflow: {}", e),
                )
            },
        )?))
    } else {
        Err((StatusCode::NOT_FOUND, format!("Workflow {} not found", id)))
    }
}

/// POST /v1/workflows/:id/cancel — cancel a running workflow
///
/// Sets the workflow status to "cancelled". The background executor is not
/// interrupted mid-step, but will find the workflow in terminal state on
/// its next status write (no further updates will be visible to callers).
pub async fn cancel_workflow(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let mut entry = state_guard
        .workflow_states
        .get_mut(&id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Workflow {} not found", id)))?;
    entry.cancel();
    info!("Workflow {} cancelled via API", id);
    Ok(Json(serde_json::json!({
        "workflow_id": id,
        "status": "cancelled",
    })))
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
