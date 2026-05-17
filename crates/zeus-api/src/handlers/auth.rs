//! Authentication API handlers

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::SharedState;

const ANTHROPIC_AUTHORIZE_URL: &str = "https://console.anthropic.com/oauth/authorize";
const ANTHROPIC_TOKEN_URL: &str = "https://api.anthropic.com/oauth/token";
const ANTHROPIC_DEFAULT_CLIENT_ID: &str = "9d1c250a-e61b-44b0-b5e0-4e85fbb11600";
const ANTHROPIC_OAUTH_SCOPES: &str = "user:inference";

/// POST /v1/auth/login — Login with OAuth
pub async fn auth_login() -> Json<Value> {
    let mgr = zeus_llm::OAuthManager::new();
    match mgr.login().await {
        Ok(tokens) => Json(json!({
            "status": "authenticated",
            "expires_at": tokens.expires_at.to_rfc3339(),
        })),
        Err(e) => Json(json!({
            "error": format!("Failed to login: {}", e),
            "status": "error"
        })),
    }
}

/// GET /v1/auth/status — Check current authentication status
pub async fn auth_status(State(state): State<SharedState>) -> Json<Value> {
    let config = &state.read().await.config;
    let use_oauth = config.auth.use_oauth;

    // Check for stored OAuth tokens
    let tokens = zeus_llm::OAuthTokens::load().ok().flatten();

    if let Some(ref t) = tokens {
        if !t.is_expired() {
            let method = if use_oauth { "oauth" } else { "setup_token" };
            return Json(json!({
                "authenticated": true,
                "method": method,
                "expires_at": t.expires_at.to_rfc3339()
            }));
        }
    }

    // Check for API key in environment
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        return Json(json!({
            "authenticated": true,
            "method": "api_key",
            "expires_at": null
        }));
    }

    Json(json!({
        "authenticated": false,
        "method": "none",
        "expires_at": null
    }))
}

/// POST /v1/auth/token — Submit a setup token
pub async fn auth_token(State(state): State<SharedState>, Json(body): Json<Value>) -> Json<Value> {
    let token = body.get("token").and_then(|v| v.as_str()).unwrap_or("");

    if let Some(err) = zeus_llm::validate_setup_token(token) {
        return Json(json!({
            "success": false,
            "message": err
        }));
    }

    match zeus_llm::OAuthManager::login_with_token(token) {
        Ok(_) => {
            state.write().await.config.auth.use_oauth = true;
            Json(json!({
                "success": true,
                "message": "Setup token stored successfully"
            }))
        }
        Err(e) => Json(json!({
            "success": false,
            "message": format!("Failed to store token: {}", e)
        })),
    }
}

/// POST /v1/auth/logout — Clear stored authentication
pub async fn auth_logout(State(state): State<SharedState>) -> Json<Value> {
    match zeus_llm::OAuthManager::logout() {
        Ok(_) => {
            state.write().await.config.auth.use_oauth = false;
            Json(json!({
                "success": true,
                "message": "Logged out successfully"
            }))
        }
        Err(e) => Json(json!({
            "success": false,
            "message": format!("Logout failed: {}", e)
        })),
    }
}

// ============================================================================
// Anthropic OAuth (REST-driven authorization code + PKCE flow)
// ============================================================================

/// GET /v1/auth/anthropic/login — Initiate Anthropic OAuth with PKCE, return 302 redirect
pub async fn anthropic_oauth_login(
    State(state): State<SharedState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let pkce = zeus_llm::PkceChallenge::generate();
    let oauth_state = uuid::Uuid::new_v4().to_string();

    let mut state_w = state.write().await;

    // Prune stale entries (>10min)
    let cutoff = std::time::Instant::now() - std::time::Duration::from_secs(600);
    state_w
        .oauth_pending
        .retain(|_, (_, created)| *created > cutoff);

    // Store this flow's verifier
    state_w.oauth_pending.insert(
        oauth_state.clone(),
        (pkce.verifier.clone(), std::time::Instant::now()),
    );

    let client_id = state_w
        .config
        .auth
        .anthropic_client_id
        .as_deref()
        .unwrap_or(ANTHROPIC_DEFAULT_CLIENT_ID);

    let gateway_port = state_w
        .config
        .gateway
        .as_ref()
        .map(|g| g.port)
        .unwrap_or(8080);

    let redirect_uri = state_w
        .config
        .auth
        .anthropic_redirect_uri
        .clone()
        .unwrap_or_else(|| {
            format!(
                "http://127.0.0.1:{}/v1/auth/anthropic/callback",
                gateway_port
            )
        });

    let url = format!(
        "{}?client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
        ANTHROPIC_AUTHORIZE_URL,
        urlencoding::encode(client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(ANTHROPIC_OAUTH_SCOPES),
        urlencoding::encode(&pkce.challenge),
        urlencoding::encode(&oauth_state),
    );
    drop(state_w);

    Ok(axum::response::Redirect::temporary(&url))
}

#[derive(Debug, Deserialize)]
pub struct OAuthCallbackParams {
    pub code: String,
    pub state: String,
}

/// GET /v1/auth/anthropic/callback — Handle Anthropic OAuth callback, exchange code for tokens
pub async fn anthropic_oauth_callback(
    State(state): State<SharedState>,
    Query(params): Query<OAuthCallbackParams>,
) -> Result<axum::response::Html<String>, (StatusCode, String)> {
    // Look up and consume the PKCE verifier
    let (verifier, redirect_uri, client_id) = {
        let mut state_w = state.write().await;
        let (verifier, _created) = state_w
            .oauth_pending
            .remove(&params.state)
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Invalid or expired OAuth state".to_string(),
            ))?;

        let gateway_port = state_w
            .config
            .gateway
            .as_ref()
            .map(|g| g.port)
            .unwrap_or(8080);

        let redirect_uri = state_w
            .config
            .auth
            .anthropic_redirect_uri
            .clone()
            .unwrap_or_else(|| {
                format!(
                    "http://127.0.0.1:{}/v1/auth/anthropic/callback",
                    gateway_port
                )
            });

        let client_id = state_w
            .config
            .auth
            .anthropic_client_id
            .clone()
            .unwrap_or_else(|| ANTHROPIC_DEFAULT_CLIENT_ID.to_string());

        (verifier, redirect_uri, client_id)
    };

    // Exchange code for tokens
    let tokens = zeus_llm::exchange_authorization_code(
        ANTHROPIC_TOKEN_URL,
        &client_id,
        &params.code,
        &verifier,
        &redirect_uri,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Token exchange failed: {e}"),
        )
    })?;

    // Store in credential store
    let mut cred_store = zeus_llm::CredentialStore::load()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    cred_store
        .store(zeus_llm::StoredCredential {
            provider: "anthropic".to_string(),
            kind: zeus_llm::CredentialKind::OAuthToken,
            token: tokens.access_token.clone(),
            refresh_token: tokens.refresh_token.clone(),
            expires_at: tokens.expires_at,
            stored_at: chrono::Utc::now(),
        })
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Also save legacy format + set config flag
    let _ = tokens.save();
    state.write().await.config.auth.use_oauth = true;

    Ok(axum::response::Html(
        "<html><body style='font-family:sans-serif;text-align:center;padding:60px'>\
         <h1>Zeus \u{2014} Login Successful</h1>\
         <p>You can close this window.</p></body></html>"
            .to_string(),
    ))
}

/// GET /v1/auth/anthropic/status — Check Anthropic credential status
pub async fn anthropic_oauth_status(
    State(_state): State<SharedState>,
) -> Json<Value> {
    // Check credential store for Anthropic
    if let Ok(Some(cred)) = zeus_llm::OAuthManager::get_credential("anthropic") {
        let is_valid = chrono::Utc::now() < cred.expires_at;
        let method = match cred.kind {
            zeus_llm::CredentialKind::OAuthToken => "oauth",
            zeus_llm::CredentialKind::SetupToken => "setup_token",
            zeus_llm::CredentialKind::ApiKey => "api_key",
        };
        return Json(json!({
            "authenticated": is_valid,
            "method": method,
            "expires_at": cred.expires_at.to_rfc3339(),
            "stored_at": cred.stored_at.to_rfc3339(),
        }));
    }

    // Check env var
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        return Json(json!({
            "authenticated": true,
            "method": "api_key",
            "expires_at": null,
            "stored_at": null,
        }));
    }

    Json(json!({
        "authenticated": false,
        "method": "none",
        "expires_at": null,
        "stored_at": null,
    }))
}
