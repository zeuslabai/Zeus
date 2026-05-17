//! Security-related handlers — approvals for tool execution.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::SharedState;

// ============================================================================
// Request Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct DenyRequest {
    pub reason: Option<String>,
}

// ============================================================================
// Approvals
// ============================================================================

/// GET /v1/approvals — list pending approvals
pub async fn list_approvals(State(state): State<SharedState>) -> Json<Value> {
    let guard = state.read().await;
    let pending = guard.approvals.list_pending();
    Json(json!(pending))
}

/// POST /v1/approvals/:id/approve — approve a pending tool execution
pub async fn approve_execution(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let mut guard = state.write().await;
    match guard.approvals.approve(&id) {
        Ok(()) => (StatusCode::OK, Json(json!({"approved": true, "id": id}))),
        Err(msg) => (StatusCode::NOT_FOUND, Json(json!({"error": msg}))),
    }
}

/// POST /v1/approvals/:id/deny — deny a pending tool execution
pub async fn deny_execution(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    body: Option<Json<DenyRequest>>,
) -> (StatusCode, Json<Value>) {
    let reason = body.and_then(|b| b.0.reason);
    let mut guard = state.write().await;
    match guard.approvals.deny(&id, reason) {
        Ok(()) => (StatusCode::OK, Json(json!({"denied": true, "id": id}))),
        Err(msg) => (StatusCode::NOT_FOUND, Json(json!({"error": msg}))),
    }
}
