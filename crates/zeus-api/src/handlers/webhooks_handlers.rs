use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use crate::SharedState;

#[derive(Debug, serde::Deserialize)]
pub struct RegisterWebhookRequest {
    pub url: String,
    pub events: Vec<crate::webhook_outbound::WebhookEventType>,
    #[serde(default)]
    pub secret: Option<String>,
}

/// GET /v1/webhooks/outbound — List all outbound webhook registrations
pub async fn list_outbound_webhooks(State(state): State<SharedState>) -> Json<Value> {
    let st = state.read().await;
    let hooks = st.webhook_manager.list().await;
    Json(json!({
        "webhooks": hooks,
        "count": hooks.len()
    }))
}

/// POST /v1/webhooks/outbound — Register a new outbound webhook
pub async fn register_outbound_webhook(
    State(state): State<SharedState>,
    Json(req): Json<RegisterWebhookRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let st = state.read().await;
    match st
        .webhook_manager
        .register(req.url, req.events, req.secret)
        .await
    {
        Ok(hook) => Ok((StatusCode::CREATED, Json(json!(hook)))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e)),
    }
}

/// DELETE /v1/webhooks/outbound/:id — Delete an outbound webhook
pub async fn delete_outbound_webhook(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let st = state.read().await;
    match st.webhook_manager.delete(&id).await {
        Ok(()) => Ok(Json(json!({
            "message": "Webhook deleted",
            "id": id
        }))),
        Err(e) if e.contains("not found") => Err((StatusCode::NOT_FOUND, e)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}
