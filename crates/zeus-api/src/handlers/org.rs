//! Org / multi-identity management handlers (#432).
//!
//! Routes:
//! - POST /v1/org/invite        (admin+)  — mint one-time invite code (raw, shown once)
//! - POST /v1/org/accept        (public)  — redeem invite → principal + first token
//! - GET  /v1/org/members       (admin+)  — list principals (no token material)
//! - DELETE /v1/org/members/:id (admin+)  — disable principal + revoke tokens
//! - POST /v1/org/tokens        (member+) — mint additional token for self

use crate::SharedState;
use crate::identity::{IdentityError, Principal, Role};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CreateInviteRequest {
    /// Role for the invited principal: readonly | member | admin
    pub role: String,
    /// Optional TTL in seconds (default 86400 = 24h)
    pub ttl_secs: Option<u64>,
}

#[derive(Deserialize)]
pub struct AcceptInviteRequest {
    pub code: String,
    pub display_name: String,
}

#[derive(Deserialize)]
pub struct MintTokenRequest {
    pub label: Option<String>,
}

fn identity_error_response(e: IdentityError) -> Response {
    let (status, msg) = match e {
        IdentityError::NotFound => (StatusCode::NOT_FOUND, "not found"),
        IdentityError::InviteUsed => (StatusCode::CONFLICT, "invite already used"),
        IdentityError::InviteExpired => (StatusCode::GONE, "invite expired"),
        IdentityError::InvalidRole => (StatusCode::BAD_REQUEST, "invalid role"),
        IdentityError::PrincipalDisabled => (StatusCode::FORBIDDEN, "principal disabled"),
        IdentityError::TokenExpired => (StatusCode::UNAUTHORIZED, "token expired"),
        IdentityError::Db(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
    };
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

fn no_store_response() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "error": "multi-identity store not configured (single-token legacy mode)"
        })),
    )
        .into_response()
}

/// POST /v1/org/invite — admin+ (scope enforced in middleware).
pub async fn create_invite(
    State(state): State<SharedState>,
    Json(req): Json<CreateInviteRequest>,
) -> Response {
    let store = {
        let s = state.read().await;
        match &s.identity_store {
            Some(st) => st.clone(),
            None => return no_store_response(),
        }
    };
    let role = match Role::parse_role(&req.role) {
        Some(r) if r != Role::Root => r,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "role must be one of: readonly, member, admin"
                })),
            )
                .into_response();
        }
    };
    let ttl = req.ttl_secs.unwrap_or(86_400);
    match store.create_invite(role, ttl) {
        Ok(code) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "invite_code": code,
                "role": role.as_str(),
                "expires_in_secs": ttl,
                "note": "Store this code now — it is shown only once."
            })),
        )
            .into_response(),
        Err(e) => identity_error_response(e),
    }
}

/// POST /v1/org/accept — public (listed in is_public_path).
pub async fn accept_invite(
    State(state): State<SharedState>,
    Json(req): Json<AcceptInviteRequest>,
) -> Response {
    let store = {
        let s = state.read().await;
        match &s.identity_store {
            Some(st) => st.clone(),
            None => return no_store_response(),
        }
    };
    if req.display_name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "display_name required" })),
        )
            .into_response();
    }
    match store.accept_invite(&req.code, req.display_name.trim()) {
        Ok((principal, raw_token)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "principal_id": principal.id,
                "display_name": principal.display_name,
                "role": principal.role.as_str(),
                "token": raw_token,
                "note": "Store this token now — it is shown only once."
            })),
        )
            .into_response(),
        Err(e) => identity_error_response(e),
    }
}

/// GET /v1/org/members — admin+.
pub async fn list_members(State(state): State<SharedState>) -> Response {
    let store = {
        let s = state.read().await;
        match &s.identity_store {
            Some(st) => st.clone(),
            None => return no_store_response(),
        }
    };
    match store.list_members() {
        Ok(members) => Json(serde_json::json!({ "members": members })).into_response(),
        Err(e) => identity_error_response(e),
    }
}

/// DELETE /v1/org/members/:id — admin+. Disables principal + revokes all tokens.
pub async fn remove_member(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> Response {
    // Guard: an admin cannot disable their own account via this route
    // (prevents lockout footguns; root can still do it out-of-band).
    if principal.id == id && principal.role < Role::Root {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "cannot disable your own principal" })),
        )
            .into_response();
    }
    let store = {
        let s = state.read().await;
        match &s.identity_store {
            Some(st) => st.clone(),
            None => return no_store_response(),
        }
    };
    match store.disable_principal(&id) {
        Ok(()) => Json(serde_json::json!({ "disabled": id })).into_response(),
        Err(e) => identity_error_response(e),
    }
}

/// POST /v1/org/tokens — any authenticated principal; mints a token for SELF.
pub async fn mint_token(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<MintTokenRequest>,
) -> Response {
    // Root (legacy single-token mode) has no store row to attach a token to.
    if principal.id == "root" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "root principal cannot mint store tokens; use /v1/org/invite to create principals"
            })),
        )
            .into_response();
    }
    let store = {
        let s = state.read().await;
        match &s.identity_store {
            Some(st) => st.clone(),
            None => return no_store_response(),
        }
    };
    let label = req.label.unwrap_or_else(|| "api".to_string());
    match store.mint_token(&principal.id, &label) {
        Ok(raw) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "token": raw,
                "label": label,
                "note": "Store this token now — it is shown only once."
            })),
        )
            .into_response(),
        Err(e) => identity_error_response(e),
    }
}
