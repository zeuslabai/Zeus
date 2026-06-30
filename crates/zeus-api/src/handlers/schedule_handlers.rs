//! Schedule Handlers — extracted from mod.rs (Track D3)

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use crate::SharedState;

/// List all schedules
pub async fn list_schedules(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let schedules = state.scheduler().list_schedules().await;
    let total = schedules.len();
    let entries: Vec<Value> = schedules
        .into_iter()
        .map(|s| serde_json::to_value(s).unwrap_or_default())
        .collect();
    Json(json!({
        "schedules": entries,
        "total": total,
    }))
}

/// Create a schedule
pub async fn create_schedule(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'name' field".to_string()))?;

    let cron_expression = body
        .get("cron_expression")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Missing 'cron_expression' field".to_string(),
            )
        })?;

    let task = body
        .get("task")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'task' field".to_string()))?;

    let mut schedule =
        zeus_orchestra::scheduler::ScheduleDefinition::new(name, cron_expression, task).map_err(
            |e| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("Invalid cron expression: {e}"),
                )
            },
        )?;

    if let Some(desc) = body.get("description").and_then(|v| v.as_str()) {
        schedule = schedule.with_description(desc);
    }
    if let Some(mode_str) = body.get("delivery_mode").and_then(|v| v.as_str())
        && let Ok(mode) = serde_json::from_value::<zeus_orchestra::scheduler::DeliveryMode>(
            Value::String(mode_str.to_string()),
        )
    {
        schedule = schedule.with_delivery_mode(mode);
    }
    if let Some(retries) = body.get("max_retries").and_then(|v| v.as_u64()) {
        schedule = schedule.with_max_retries(retries as u32);
    }

    let state = state.read().await;
    let added = state
        .scheduler()
        .add_schedule(schedule)
        .await
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(added).unwrap_or_default()),
    ))
}

/// Get a schedule by ID
pub async fn get_schedule(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let schedule = state
        .scheduler()
        .get_schedule(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(serde_json::to_value(schedule).unwrap_or_default()))
}

/// Update a schedule
pub async fn update_schedule(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    // Fetch existing schedule
    let mut schedule = state
        .scheduler()
        .get_schedule(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    // Apply partial updates
    if let Some(name) = body.get("name").and_then(|v| v.as_str()) {
        schedule.name = name.to_string();
    }
    if let Some(cron_expr) = body.get("cron_expression").and_then(|v| v.as_str()) {
        // Validate the new cron expression
        let next = zeus_orchestra::scheduler::calculate_next_run(cron_expr).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Invalid cron expression: {e}"),
            )
        })?;
        schedule.cron_expression = cron_expr.to_string();
        schedule.next_run = next;
    }
    if let Some(task) = body.get("task").and_then(|v| v.as_str()) {
        schedule.task = task.to_string();
    }
    if let Some(desc) = body.get("description").and_then(|v| v.as_str()) {
        schedule.description = Some(desc.to_string());
    }
    if let Some(enabled) = body.get("enabled").and_then(|v| v.as_bool()) {
        schedule.enabled = enabled;
    }
    if let Some(mode_str) = body.get("delivery_mode").and_then(|v| v.as_str())
        && let Ok(mode) = serde_json::from_value::<zeus_orchestra::scheduler::DeliveryMode>(
            Value::String(mode_str.to_string()),
        )
    {
        schedule.delivery_mode = mode;
    }
    if let Some(retries) = body.get("max_retries").and_then(|v| v.as_u64()) {
        schedule.max_retries = retries as u32;
    }
    schedule.updated_at = Some(chrono::Utc::now());

    let updated = state
        .scheduler()
        .update_schedule(schedule)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(Json(serde_json::to_value(updated).unwrap_or_default()))
}

/// Delete a schedule
pub async fn delete_schedule(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    state
        .scheduler()
        .delete_schedule(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(json!({
        "id": id,
        "deleted": true
    })))
}

/// Pause a schedule
pub async fn pause_schedule(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let paused = state
        .scheduler()
        .pause_schedule(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(serde_json::to_value(paused).unwrap_or_default()))
}

/// Resume a schedule
pub async fn resume_schedule(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let resumed = state
        .scheduler()
        .resume_schedule(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(serde_json::to_value(resumed).unwrap_or_default()))
}

/// List runs for a schedule
pub async fn list_schedule_runs(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let state = state.read().await;
    let runs = state.scheduler().runs_for_schedule(&id).await;
    let total = runs.len();
    let entries: Vec<Value> = runs
        .into_iter()
        .map(|r| serde_json::to_value(r).unwrap_or_default())
        .collect();
    Json(json!({
        "schedule_id": id,
        "runs": entries,
        "total": total,
    }))
}
