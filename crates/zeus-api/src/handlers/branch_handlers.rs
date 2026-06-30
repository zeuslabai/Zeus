//! Branch handlers — session branching endpoints

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::info;
use crate::SharedState;

#[derive(Debug, Deserialize)]
pub struct CreateBranchRequest {
    pub at_index: usize,
    #[serde(default)]
    pub label: Option<String>,
}

/// POST /v1/sessions/:id/branch -- Create a branch from a session at a given message index
pub async fn create_branch(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<CreateBranchRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let state = state.read().await;

    let bp = state
        .branch_manager
        .create_branch(&id, req.at_index, req.label)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    info!(
        "Created branch {} from session {} at index {}",
        bp.branch_session_id, bp.parent_session_id, bp.branch_at_index
    );

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "parent_session_id": bp.parent_session_id,
            "branch_session_id": bp.branch_session_id,
            "branch_at_index": bp.branch_at_index,
            "created": bp.created.to_rfc3339(),
            "label": bp.label,
        })),
    ))
}

/// GET /v1/sessions/:id/branches -- List branches of a session
pub async fn list_branches(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let state = state.read().await;

    let branches = state.branch_manager.list_branches(&id).await;

    let entries: Vec<Value> = branches
        .iter()
        .map(|bp| {
            json!({
                "parent_session_id": bp.parent_session_id,
                "branch_session_id": bp.branch_session_id,
                "branch_at_index": bp.branch_at_index,
                "created": bp.created.to_rfc3339(),
                "label": bp.label,
            })
        })
        .collect();

    Json(json!({ "branches": entries, "count": entries.len() }))
}
