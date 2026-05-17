use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};

use crate::SharedState;

/// List teams
pub async fn list_teams(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let teams = state_guard.orchestra().list_teams().await;
    let team_values: Vec<Value> = teams
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "name": t.name,
                "agent_ids": t.agent_ids,
                "supervisor_id": t.supervisor_id,
                "agent_count": t.agent_count(),
                "created_at": t.created_at.to_rfc3339(),
            })
        })
        .collect();
    let total = team_values.len();
    Json(json!({
        "teams": team_values,
        "total": total,
    }))
}

/// Create a team
pub async fn create_team(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'name' field".to_string()))?;

    let mut team = zeus_orchestra::AgentTeam::new(name);

    if let Some(supervisor) = body.get("supervisor_id").and_then(|v| v.as_str())
        && !supervisor.is_empty()
    {
        team = team.with_supervisor(supervisor.to_string());
    }
    if let Some(agents) = body.get("agent_ids").and_then(|v| v.as_array()) {
        let ids: Vec<String> = agents
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        team = team.with_agents(ids);
    }
    if let Some(policy_val) = body.get("policy") {
        let mut policy = zeus_orchestra::TeamPolicy::default();
        if let Some(v) = policy_val.get("max_depth").and_then(|v| v.as_u64()) {
            policy.max_depth = v as u32;
        }
        if let Some(v) = policy_val.get("budget_tokens").and_then(|v| v.as_u64()) {
            policy.budget_tokens = v;
        }
        if let Some(v) = policy_val.get("timeout_seconds").and_then(|v| v.as_u64()) {
            policy.timeout_seconds = v;
        }
        if let Some(v) = policy_val.get("loop_detection").and_then(|v| v.as_bool()) {
            policy.loop_detection = v;
        }
        if let Some(v) = policy_val.get("quality_threshold").and_then(|v| v.as_f64()) {
            policy.quality_threshold = v;
        }
        if let Some(v) = policy_val
            .get("require_verification")
            .and_then(|v| v.as_bool())
        {
            policy.require_verification = v;
        }
        team = team.with_policy(policy);
    }

    let state_guard = state.read().await;
    let created = state_guard
        .orchestra()
        .create_team(team)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    let mut val = serde_json::to_value(&created).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize error: {e}"),
        )
    })?;
    if let Some(obj) = val.as_object_mut() {
        obj.insert(
            "status".to_string(),
            serde_json::Value::String("created".to_string()),
        );
    }

    Ok((StatusCode::CREATED, Json(val)))
}

/// Get team details
pub async fn get_team(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let team = state_guard
        .orchestra()
        .get_team(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, format!("{e}")))?;
    let val = serde_json::to_value(&team).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize error: {e}"),
        )
    })?;
    Ok(Json(val))
}

/// Update a team
pub async fn update_team(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let mut team = state_guard
        .orchestra()
        .get_team(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, format!("{e}")))?;

    if let Some(name) = body.get("name").and_then(|v| v.as_str()) {
        team.name = name.to_string();
    }
    if let Some(supervisor) = body.get("supervisor_id") {
        team.supervisor_id = supervisor.as_str().map(|s| s.to_string());
    }
    if let Some(agents) = body.get("agent_ids").and_then(|v| v.as_array()) {
        team.agent_ids = agents
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }

    state_guard
        .orchestra()
        .update_team(team.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    let val = serde_json::to_value(&team).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize error: {e}"),
        )
    })?;
    Ok(Json(val))
}

/// Delete a team
pub async fn delete_team(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    state_guard
        .orchestra()
        .delete_team(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, format!("{e}")))?;
    Ok(Json(json!({ "deleted": true, "id": id })))
}

/// Create a delegation
pub async fn create_delegation(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let task = body
        .get("task")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'task' field".to_string()))?;

    let from_agent = body
        .get("from_agent")
        .and_then(|v| v.as_str())
        .unwrap_or("orchestrator");

    let to_agent = body
        .get("to_agent")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Missing 'to_agent' field".to_string(),
            )
        })?;

    let delegation_id = uuid::Uuid::new_v4().to_string();
    let entry = json!({
        "id": delegation_id,
        "task": task,
        "from_agent": from_agent,
        "to_agent": to_agent,
        "status": "pending",
        "created_at": chrono::Utc::now().to_rfc3339(),
    });

    let state = state.read().await;
    state
        .delegations
        .insert(delegation_id.clone(), entry.clone());

    Ok((StatusCode::CREATED, Json(entry)))
}

/// List delegations
pub async fn list_delegations(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let delegations: Vec<Value> = state
        .delegations
        .iter()
        .map(|entry| entry.value().clone())
        .collect();
    let total = delegations.len();
    Json(json!({
        "delegations": delegations,
        "total": total
    }))
}

/// Get smart routing recommendation
pub async fn smart_route(Json(body): Json<Value>) -> Result<Json<Value>, (StatusCode, String)> {
    let task = body
        .get("task")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'task' field".to_string()))?;

    let router = zeus_orchestra::SmartRouter::new();

    // Estimate complexity from task length and keywords
    let complexity = if task.len() < 50 {
        zeus_orchestra::TaskComplexity::Simple
    } else if task.len() < 200 {
        zeus_orchestra::TaskComplexity::Medium
    } else if task.len() < 500 {
        zeus_orchestra::TaskComplexity::Complex
    } else {
        zeus_orchestra::TaskComplexity::Expert
    };

    let model = router.route(complexity);

    Ok(Json(json!({
        "task": task,
        "complexity": format!("{:?}", complexity),
        "recommended_model": model,
        "fallback_chain": router.fallback_chain,
    })))
}
