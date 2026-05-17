//! Outcome Templates API handlers
//!
//! GET  /v1/templates              — list all templates (paginated, optional ?category=)
//! GET  /v1/templates/categories   — list unique categories
//! GET  /v1/templates/search?q=    — keyword search
//! GET  /v1/templates/:id          — get single template
//! POST /v1/templates              — create user template
//! PUT  /v1/templates/:id          — update user template
//! DELETE /v1/templates/:id        — delete user template
//! POST /v1/templates/:id/apply    — apply template to a goal

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::json;
use zeus_templates::OutcomeTemplate;

use crate::SharedState;

// ── Query params ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListTemplatesQuery {
    pub category: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

// ── Request bodies ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ApplyRequest {
    /// The user's goal string.
    pub goal: String,
    /// Provider categories already configured by the user.
    /// If omitted, the handler reads from the gateway config.
    #[serde(default)]
    pub configured_providers: Vec<String>,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /v1/templates
pub async fn list_templates(
    State(state): State<SharedState>,
    Query(params): Query<ListTemplatesQuery>,
) -> impl IntoResponse {
    let guard = state.read().await;
    let registry = guard.template_registry.clone();
    drop(guard);

    let category = params.category.as_deref();
    let mut templates = registry.list(category);

    let total = templates.len();
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(50).min(200);
    templates = templates.into_iter().skip(offset).take(limit).collect();

    Json(json!({
        "templates": templates,
        "total": total,
        "offset": offset,
        "limit": limit,
    }))
}

/// GET /v1/templates/categories
pub async fn list_categories(State(state): State<SharedState>) -> impl IntoResponse {
    let guard = state.read().await;
    let registry = guard.template_registry.clone();
    drop(guard);

    let cats = registry.categories();
    Json(json!({ "categories": cats }))
}

/// GET /v1/templates/search?q=...
pub async fn search_templates(
    State(state): State<SharedState>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    let query = match params.q {
        Some(ref q) if !q.is_empty() => q.clone(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing query param 'q'"})),
            )
                .into_response();
        }
    };

    let guard = state.read().await;
    let registry = guard.template_registry.clone();
    drop(guard);

    let results = registry.search(&query);
    let total = results.len();
    Json(json!({
        "templates": results,
        "total": total,
        "query": query,
    }))
    .into_response()
}

/// GET /v1/templates/:id
pub async fn get_template(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let guard = state.read().await;
    let registry = guard.template_registry.clone();
    drop(guard);

    match registry.get(&id) {
        Some(t) => Json(t).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("template not found: {}", id)})),
        )
            .into_response(),
    }
}

/// POST /v1/templates
pub async fn create_template(
    State(state): State<SharedState>,
    Json(template): Json<OutcomeTemplate>,
) -> impl IntoResponse {
    let guard = state.read().await;
    let registry = guard.template_registry.clone();
    drop(guard);

    match registry.create(template) {
        Ok(()) => (StatusCode::CREATED, Json(json!({"ok": true}))).into_response(),
        Err(zeus_templates::TemplateError::AlreadyExists(id)) => (
            StatusCode::CONFLICT,
            Json(json!({"error": format!("template already exists: {}", id)})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// PUT /v1/templates/:id
pub async fn update_template(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(template): Json<OutcomeTemplate>,
) -> impl IntoResponse {
    let guard = state.read().await;
    let registry = guard.template_registry.clone();
    drop(guard);

    match registry.update(&id, template) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(zeus_templates::TemplateError::NotFound(nf_id)) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("template not found: {}", nf_id)})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// DELETE /v1/templates/:id
pub async fn delete_template(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let guard = state.read().await;
    let registry = guard.template_registry.clone();
    drop(guard);

    match registry.delete(&id) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(zeus_templates::TemplateError::NotFound(nf_id)) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("template not found: {}", nf_id)})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /v1/templates/:id/apply
///
/// Applies the template to the user's goal and returns an `AppliedTemplate`
/// with the enriched prompt and any missing provider/skill warnings.
/// Present the result to the user for confirmation, then proceed to
/// `POST /v1/goals` with the enriched prompt.
pub async fn apply_template(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<ApplyRequest>,
) -> impl IntoResponse {
    let guard = state.read().await;
    let registry = guard.template_registry.clone();
    let configured_providers = if body.configured_providers.is_empty() {
        detect_configured_providers(&guard.config)
    } else {
        body.configured_providers.clone()
    };
    drop(guard);

    let template = match registry.get(&id) {
        Some(t) => t,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("template not found: {}", id)})),
            )
                .into_response();
        }
    };

    let applied = registry.apply(&template, &body.goal, &configured_providers);

    let warnings: Vec<String> = applied
        .missing_providers
        .iter()
        .map(|p| {
            format!(
                "Provider '{}' is not configured — run `zeus setup` to connect it",
                p
            )
        })
        .collect();

    let mut response = serde_json::to_value(&applied).unwrap_or_default();
    if let Some(obj) = response.as_object_mut() {
        obj.insert("warnings".to_string(), json!(warnings));
    }

    Json(response).into_response()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Detect which provider categories are currently configured.
fn detect_configured_providers(config: &zeus_core::Config) -> Vec<String> {
    let mut providers = Vec::new();

    // LLM — always present (gateway requires at least one LLM provider)
    providers.push("llm".to_string());

    // Image generation
    if config.image_gen.is_some() || std::env::var("ZEUS_IMAGE_GEN_URL").is_ok() {
        providers.push("image_gen".to_string());
    }

    // Voice — Piper TTS or Whisper STT endpoint configured
    if std::env::var("ZEUS_PIPER_URL").is_ok() || std::env::var("ZEUS_WHISPER_URL").is_ok() {
        providers.push("voice".to_string());
    }

    // Video generation
    if std::env::var("ZEUS_COMFYUI_URL").is_ok() {
        providers.push("video_gen".to_string());
    }

    providers
}
