use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use crate::SharedState;

#[derive(serde::Deserialize)]
pub struct StoreCredentialRequest {
    pub name: String,
    pub value: String,
}

/// GET /v1/credentials — List stored credential names
pub async fn list_credentials(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let names = state.credential_vault.list();
    Json(json!({ "credentials": names.iter().map(|n| json!({"name": n})).collect::<Vec<_>>() }))
}

/// POST /v1/credentials — Store a credential in the vault
pub async fn store_credential(
    State(state): State<SharedState>,
    Json(req): Json<StoreCredentialRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if req.name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name is required".to_string()));
    }
    if req.value.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "value is required".to_string()));
    }
    if !req.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err((
            StatusCode::BAD_REQUEST,
            "name must contain only alphanumeric characters and underscores".to_string(),
        ));
    }
    let state = state.read().await;
    match state.credential_vault.store(&req.name, &req.value).await {
        Ok(()) => {
            unsafe {
                std::env::set_var(&req.name, &req.value);
            }
            let method = if state.credential_vault.has_keychain() {
                "keychain"
            } else {
                "file"
            };
            Ok(Json(json!({ "name": req.name, "stored": true, "method": method })))
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// DELETE /v1/credentials/:name — Delete a credential from the vault
pub async fn delete_credential(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let state = state.read().await;
    match state.credential_vault.delete(&name).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}
