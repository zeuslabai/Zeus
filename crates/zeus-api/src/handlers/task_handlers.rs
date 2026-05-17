//! Agent task handlers — CRUD for agent tasks (S52-T1 checkpoint/resume)

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde_json::{Value, json};

use crate::SharedState;
use crate::handlers::task_store;

pub async fn create_task(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let id = body["id"].as_str().map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let agent_id = body["agent_id"].as_str().unwrap_or("default").to_string();
    let description = body["description"].as_str().unwrap_or("").to_string();
    let status_str = body["status"].as_str().unwrap_or("pending");
    let checkpoint = body.get("checkpoint").cloned().unwrap_or(json!({}));

    let task = task_store::AgentTask {
        id: id.clone(),
        agent_id,
        description,
        status: task_store::TaskStatus::parse(status_str),
        checkpoint,
        created_at: String::new(),
        updated_at: String::new(),
        scope_json: body.get("scope_json").cloned().unwrap_or(json!({})),
        iterations_used: 0,
        iterations_budget: body["iterations_budget"].as_i64().unwrap_or(20),
        assigned_by: body["assigned_by"].as_str().unwrap_or("coordinator").to_string(),
        source_channel: body["source_channel"].as_str().unwrap_or("").to_string(),
        parent_id: body["parent_id"].as_str().map(|s| s.to_string()),
        branch: body["branch"].as_str().unwrap_or("").to_string(),
        priority: body["priority"].as_i64().unwrap_or(1),
    };

    let guard = state.read().await;
    if guard.task_store.create(&task).await {
        (StatusCode::CREATED, Json(json!({"id": id, "status": "created"})))
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Failed to create task"})))
    }
}

/// GET /v1/tasks — list tasks with optional filters
pub async fn list_tasks(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let agent_id = params.get("agent_id").map(|s| s.as_str());
    let status = params.get("status").map(|s| task_store::TaskStatus::parse(s));
    let limit: usize = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(50);
    let offset: usize = params.get("offset").and_then(|s| s.parse().ok()).unwrap_or(0);

    let guard = state.read().await;
    let tasks = guard.task_store.list(agent_id, status, limit, offset).await;
    Json(json!({"tasks": tasks, "count": tasks.len()}))
}

/// GET /v1/tasks/:id — get a single task
pub async fn get_task(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let guard = state.read().await;
    match guard.task_store.get(&id).await {
        Some(task) => (StatusCode::OK, Json(serde_json::to_value(task).unwrap_or(json!({})))),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "Task not found"}))),
    }
}

/// PUT /v1/tasks/:id — update task status/checkpoint/description
pub async fn update_task(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let status = body["status"].as_str().map(task_store::TaskStatus::parse);
    let checkpoint = body.get("checkpoint");
    let description = body["description"].as_str();

    let guard = state.read().await;
    if guard.task_store.update(&id, status, checkpoint, description).await {
        (StatusCode::OK, Json(json!({"id": id, "updated": true})))
    } else {
        (StatusCode::NOT_FOUND, Json(json!({"error": "Task not found or update failed"})))
    }
}

/// DELETE /v1/tasks/:id — delete a task
pub async fn delete_task(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let guard = state.read().await;
    if guard.task_store.delete(&id).await {
        (StatusCode::OK, Json(json!({"id": id, "deleted": true})))
    } else {
        (StatusCode::NOT_FOUND, Json(json!({"error": "Task not found"})))
    }
}

/// GET /v1/tasks/active — get all active tasks (for resume on startup)
pub async fn get_active_tasks(
    State(state): State<SharedState>,
) -> Json<Value> {
    let guard = state.read().await;
    let tasks = guard.task_store.get_active_tasks().await;
    Json(json!({"tasks": tasks, "count": tasks.len()}))
}

/// GET /v1/tasks/stats — task count by status
pub async fn task_stats(
    State(state): State<SharedState>,
) -> Json<Value> {
    let guard = state.read().await;
    let counts = guard.task_store.count_by_status().await;
    Json(json!(counts))
}

// ============================================================================
// Discord History (S52-T2)
// ============================================================================

// GET /v1/discord/history — query cached Discord messages (see history_handlers.rs)
