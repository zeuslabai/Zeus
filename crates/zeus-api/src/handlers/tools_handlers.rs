//! Tool listing and execution handlers

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use tracing::debug;

use crate::SharedState;

pub async fn list_tools(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;

    let tools: Vec<Value> = state
        .tools
        .schemas()
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "description": s.description,
                "category": s.category(),
                "parameters": s.parameters
            })
        })
        .collect();

    Json(json!({ "tools": tools }))
}

pub async fn execute_tool(
    State(state): State<SharedState>,
    Path(name): Path<String>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, (StatusCode, String)> {
    if body.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "missing request body".to_string(),
        ));
    }
    let raw: Value = serde_json::from_slice(&body)
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    let args = match raw.get("arguments").cloned() {
        Some(a) => a,
        None => raw,
    };
    debug!("Execute tool: {} with {:?}", name, args);

    let state = state.read().await;

    let result = state.tools.execute(&name, args).await;

    match result {
        Ok(output) => Ok(Json(json!({
            "success": true,
            "output": output
        }))),
        Err(e) => Ok(Json(json!({
            "success": false,
            "error": e.to_string()
        }))),
    }
}
