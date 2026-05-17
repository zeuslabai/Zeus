//! Goals handlers

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use crate::SharedState;
use super::{try_llm_goal_analysis, heuristic_goal_analysis};

pub async fn goals_list(State(_state): State<SharedState>) -> Json<Value> {
    let db_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("goals.db");

    match zeus_prometheus::GoalStack::new(&db_path) {
        Ok(stack) => {
            let active = stack.active_goals().unwrap_or_default();
            let active_values: Vec<Value> = active
                .iter()
                .filter_map(|g| serde_json::to_value(g).ok())
                .collect();
            let total = active_values.len();
            Json(json!({
                "goals": active_values,
                "total": total,
            }))
        }
        Err(e) => Json(json!({
            "goals": [],
            "total": 0,
            "error": format!("GoalStack unavailable: {e}"),
        })),
    }
}

pub async fn goals_create(
    State(_state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let description = body
        .get("description")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'description'".to_string()))?;

    let priority = match body.get("priority").and_then(|v| v.as_str()) {
        Some("critical") => zeus_prometheus::Priority::Critical,
        Some("high") => zeus_prometheus::Priority::High,
        Some("low") => zeus_prometheus::Priority::Low,
        Some("background") => zeus_prometheus::Priority::Background,
        _ => zeus_prometheus::Priority::Normal,
    };

    let source = match body.get("source").and_then(|v| v.as_str()) {
        Some("system") => zeus_prometheus::GoalSource::System,
        Some("agent") => zeus_prometheus::GoalSource::System,
        _ => zeus_prometheus::GoalSource::User,
    };

    let db_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("goals.db");

    let stack = zeus_prometheus::GoalStack::new(&db_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("GoalStack unavailable: {e}"),
        )
    })?;

    let goal = zeus_prometheus::Goal::new(description, priority, source);
    let id = stack.add(&goal).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create goal: {e}"),
        )
    })?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "description": description,
            "priority": format!("{:?}", priority).to_lowercase(),
            "status": "pending",
        })),
    ))
}

pub async fn goals_get(
    State(_state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let db_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("goals.db");

    let stack = zeus_prometheus::GoalStack::new(&db_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("GoalStack unavailable: {e}"),
        )
    })?;

    let goal = stack.get(&id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to query goal: {e}"),
        )
    })?;

    match goal {
        Some(g) => {
            let val = serde_json::to_value(&g).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Serialization error: {e}"),
                )
            })?;
            Ok(Json(val))
        }
        None => Err((StatusCode::NOT_FOUND, format!("Goal '{id}' not found"))),
    }
}

pub async fn goals_update_status(
    State(_state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let status_str = body.get("status").and_then(|v| v.as_str()).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Missing 'status' field".to_string(),
        )
    })?;

    let detail = body
        .get("detail")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let status = match status_str {
        "pending" => zeus_prometheus::GoalStatus::Pending,
        "active" => zeus_prometheus::GoalStatus::Active,
        "blocked" => zeus_prometheus::GoalStatus::Blocked {
            reason: if detail.is_empty() {
                "Blocked".to_string()
            } else {
                detail.clone()
            },
        },
        "completed" => zeus_prometheus::GoalStatus::Completed {
            outcome: if detail.is_empty() {
                "Completed".to_string()
            } else {
                detail.clone()
            },
        },
        "failed" => zeus_prometheus::GoalStatus::Failed {
            reason: if detail.is_empty() {
                "Failed".to_string()
            } else {
                detail.clone()
            },
        },
        "abandoned" => zeus_prometheus::GoalStatus::Abandoned {
            reason: if detail.is_empty() {
                "Abandoned".to_string()
            } else {
                detail.clone()
            },
        },
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "Invalid status '{other}'. Must be: pending, active, blocked, completed, failed, abandoned"
                ),
            ));
        }
    };

    let db_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("goals.db");

    let stack = zeus_prometheus::GoalStack::new(&db_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("GoalStack unavailable: {e}"),
        )
    })?;

    stack.update_status(&id, status).map_err(|e| {
        let msg = format!("{e}");
        if msg.contains("not found") {
            (StatusCode::NOT_FOUND, format!("Goal '{id}' not found"))
        } else {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update goal: {e}"),
            )
        }
    })?;

    // Return the updated goal
    let goal = stack.get(&id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to retrieve updated goal: {e}"),
        )
    })?;

    match goal {
        Some(g) => {
            let val = serde_json::to_value(&g).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Serialization error: {e}"),
                )
            })?;
            Ok(Json(val))
        }
        None => Err((
            StatusCode::NOT_FOUND,
            format!("Goal '{id}' not found after update"),
        )),
    }
}

pub async fn goals_analyze(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let goal_text = body
        .get("goal")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'goal' field".to_string()))?;

    // Try LLM-based analysis first
    match try_llm_goal_analysis(&state, &body, goal_text).await {
        Some(mut analysis) => {
            analysis["analysis_method"] = json!("llm");
            Ok(Json(analysis))
        }
        None => {
            // Fall back to heuristic analysis
            Ok(Json(heuristic_goal_analysis(goal_text)))
        }
    }
}

