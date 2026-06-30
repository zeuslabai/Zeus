use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateAgentFromPersonaRequest {
    pub name: Option<String>,
    pub model: Option<String>,
    pub autonomy: Option<String>,
}

/// GET /v1/personas — List available persona templates
pub async fn list_personas(State(state): State<crate::SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let workspace = state_guard.config.workspace_path().to_path_buf();
    drop(state_guard);

    let templates = zeus_core::PersonaTemplate::load_all(&workspace.join("personas"));

    let items: Vec<Value> = templates
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "model": t.model,
                "tools": t.tools,
            })
        })
        .collect();

    Json(json!({ "personas": items, "count": items.len() }))
}

/// GET /v1/personas/:name — Get a specific persona template
pub async fn get_persona(
    State(state): State<crate::SharedState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let workspace = state_guard.config.workspace_path().to_path_buf();
    drop(state_guard);

    let template = zeus_core::PersonaTemplate::find(&name, &workspace).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Persona not found: {}", name),
        )
    })?;

    Ok(Json(json!({
        "name": template.name,
        "description": template.description,
        "model": template.model,
        "tools": template.tools,
        "persona_text": template.persona_text,
    })))
}
