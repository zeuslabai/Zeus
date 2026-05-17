//! Authentication handlers — login, logout, OAuth, token refresh

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::SharedState;

pub async fn auth_login(
    State(state): State<SharedState>,
    body: Option<Json<Value>>,
) -> Json<Value> {
    // Web-driven OAuth: frontend sends provider/redirect_uri/state/code_verifier
    if let Some(Json(ref b)) = body {
        let provider = b.get("provider").and_then(|v| v.as_str()).unwrap_or("");
        let redirect_uri = b.get("redirect_uri").and_then(|v| v.as_str()).unwrap_or("");
        let frontend_state = b.get("state").and_then(|v| v.as_str()).unwrap_or("");
        let code_verifier = b
            .get("code_verifier")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if !provider.is_empty() && !redirect_uri.is_empty() && !code_verifier.is_empty() {
            return match provider {
                "anthropic" => {
                    let pkce = zeus_llm::PkceChallenge::from_verifier(code_verifier);

                    let mut state_w = state.write().await;

                    // Prune stale entries (>10min)
                    let cutoff = std::time::Instant::now() - std::time::Duration::from_secs(600);
                    state_w
                        .oauth_pending
                        .retain(|_, (_, created)| *created > cutoff);

                    // Store pending state for callback
                    state_w.oauth_pending.insert(
                        frontend_state.to_string(),
                        (code_verifier.to_string(), std::time::Instant::now()),
                    );

                    let client_id = state_w
                        .config
                        .auth
                        .anthropic_client_id
                        .as_deref()
                        .unwrap_or(ANTHROPIC_DEFAULT_CLIENT_ID);

                    let url = format!(
                        "{}?client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
                        ANTHROPIC_AUTHORIZE_URL,
                        urlencoding::encode(client_id),
                        urlencoding::encode(redirect_uri),
                        urlencoding::encode(ANTHROPIC_OAUTH_SCOPES),
                        urlencoding::encode(&pkce.challenge),
                        urlencoding::encode(frontend_state),
                    );

                    Json(json!({
                        "authorize_url": url,
                        "status": "redirect"
                    }))
                }
                "openai" => Json(json!({
                    "authorize_url": "",
                    "status": "error",
                    "error": "OpenAI OAuth is not yet configured. Please use an API key instead."
                })),
                _ => Json(json!({
                    "authorize_url": "",
                    "status": "error",
                    "error": format!("OAuth not available for provider '{}'. Please use an API key.", provider)
                })),
            };
        }
    }

    // Fallback: legacy server-side browser flow (for CLI usage)
    // Use tokio::time::timeout to prevent indefinite blocking when
    // the OAuth callback server cannot receive a browser redirect
    // (e.g. in tests, headless environments, or when port is busy).
    let mgr = zeus_llm::OAuthManager::new();
    match tokio::time::timeout(std::time::Duration::from_secs(120), mgr.login()).await {
        Ok(Ok(tokens)) => Json(json!({
            "status": "authenticated",
            "expires_at": tokens.expires_at.to_rfc3339(),
        })),
        Ok(Err(e)) => Json(json!({
            "error": format!("Failed to login: {}", e),
            "status": "error"
        })),
        Err(_) => Json(json!({
            "error": "OAuth login timed out (120s). Use API key or web-based OAuth flow instead.",
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

    if let Some(ref t) = tokens
        && !t.is_expired()
    {
        let method = if use_oauth { "oauth" } else { "setup_token" };
        return Json(json!({
            "authenticated": true,
            "method": method,
            "expires_at": t.expires_at.to_rfc3339()
        }));
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

/// POST /v1/auth/token — Handle OAuth code exchange, API key saving, or setup token
pub async fn auth_token(State(state): State<SharedState>, Json(body): Json<Value>) -> Json<Value> {
    let code = body.get("code").and_then(|v| v.as_str()).unwrap_or("");
    let code_verifier = body
        .get("code_verifier")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let provider = body.get("provider").and_then(|v| v.as_str()).unwrap_or("");
    let redirect_uri = body
        .get("redirect_uri")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let token = body.get("token").and_then(|v| v.as_str()).unwrap_or("");

    // Case 1: OAuth code exchange (code + code_verifier present)
    if !code.is_empty() && !code_verifier.is_empty() {
        let effective_provider = if provider.is_empty() {
            "anthropic"
        } else {
            provider
        };
        let token_url = match effective_provider {
            "anthropic" => ANTHROPIC_TOKEN_URL,
            _ => {
                return Json(json!({
                    "success": false,
                    "message": format!("OAuth code exchange not supported for provider '{}'", effective_provider)
                }));
            }
        };

        let client_id = {
            let state_r = state.read().await;
            state_r
                .config
                .auth
                .anthropic_client_id
                .clone()
                .unwrap_or_else(|| ANTHROPIC_DEFAULT_CLIENT_ID.to_string())
        };

        match zeus_llm::exchange_authorization_code(
            token_url,
            &client_id,
            code,
            code_verifier,
            redirect_uri,
        )
        .await
        {
            Ok(tokens) => {
                // Store in credential store
                if let Ok(mut cred_store) = zeus_llm::CredentialStore::load() {
                    let _ = cred_store.store(zeus_llm::StoredCredential {
                        provider: effective_provider.to_string(),
                        kind: zeus_llm::CredentialKind::OAuthToken,
                        token: tokens.access_token.clone(),
                        refresh_token: tokens.refresh_token.clone(),
                        expires_at: tokens.expires_at,
                        stored_at: chrono::Utc::now(),
                    });
                }
                let _ = tokens.save();
                state.write().await.config.auth.use_oauth = true;

                Json(json!({
                    "success": true,
                    "token": tokens.access_token,
                    "provider": effective_provider,
                    "message": "OAuth authentication successful"
                }))
            }
            Err(e) => Json(json!({
                "success": false,
                "message": format!("Token exchange failed: {}", e)
            })),
        }
    }
    // Case 2: Provider-specific API key (provider + token, no code)
    else if !provider.is_empty() && !token.is_empty() {
        match zeus_llm::OAuthManager::login_auto_detect(token) {
            Ok(detected_provider) => Json(json!({
                "success": true,
                "provider": detected_provider,
                "message": format!("API key stored for {}", detected_provider)
            })),
            Err(_) => {
                // Auto-detect failed, try storing as explicit provider key
                match zeus_llm::OAuthManager::login_with_api_key(provider, token) {
                    Ok(()) => Json(json!({
                        "success": true,
                        "provider": provider,
                        "message": format!("API key stored for {}", provider)
                    })),
                    Err(e) => Json(json!({
                        "success": false,
                        "message": format!("Failed to store API key: {}", e)
                    })),
                }
            }
        }
    }
    // Case 3: Setup token (token only, no provider)
    else if !token.is_empty() {
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
    // Case 4: Nothing useful provided
    else {
        Json(json!({
            "success": false,
            "message": "No token, API key, or authorization code provided"
        }))
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

const ANTHROPIC_AUTHORIZE_URL: &str = "https://console.anthropic.com/oauth/authorize";
const ANTHROPIC_TOKEN_URL: &str = "https://api.anthropic.com/oauth/token";
const ANTHROPIC_DEFAULT_CLIENT_ID: &str = "9d1c250a-e61b-44b0-b5e0-4e85fbb11600";
const ANTHROPIC_OAUTH_SCOPES: &str = "user:inference";

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
        let (verifier, _created) = state_w.oauth_pending.remove(&params.state).ok_or((
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
pub async fn anthropic_oauth_status(State(_state): State<SharedState>) -> Json<Value> {
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

/// POST /v1/auth/refresh — Refresh an expired OAuth token using stored refresh_token
pub async fn auth_refresh(State(_state): State<SharedState>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    let provider = body.get("provider").and_then(|v| v.as_str()).unwrap_or("anthropic");

    // Check if credential exists and has a refresh token
    let cred = match zeus_llm::OAuthManager::get_credential(provider) {
        Ok(Some(c)) => c,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, Json(json!({
                "error": format!("No credentials stored for provider: {}", provider),
                "refreshed": false,
            })));
        }
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
                "error": format!("Failed to load credentials: {}", e),
                "refreshed": false,
            })));
        }
    };

    if cred.kind != zeus_llm::CredentialKind::OAuthToken {
        return (StatusCode::BAD_REQUEST, Json(json!({
            "error": "Only OAuth tokens can be refreshed. API keys and setup-tokens don't expire.",
            "refreshed": false,
            "kind": format!("{:?}", cred.kind),
        })));
    }

    if cred.refresh_token.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({
            "error": "No refresh_token available. Re-authenticate with /v1/auth/anthropic/login.",
            "refreshed": false,
        })));
    }

    match zeus_llm::OAuthManager::refresh_token(provider).await {
        Ok(Some(tokens)) => {
            (StatusCode::OK, Json(json!({
                "refreshed": true,
                "provider": provider,
                "expires_at": tokens.expires_at.to_rfc3339(),
                "token_prefix": &tokens.access_token[..tokens.access_token.len().min(20)],
            })))
        }
        Ok(None) => {
            (StatusCode::BAD_REQUEST, Json(json!({
                "refreshed": false,
                "error": "Refresh not available for this credential type",
            })))
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
                "refreshed": false,
                "error": format!("Refresh failed: {}", e),
            })))
        }
    }
}

// ============================================================================
// Agent Tasks (S52-T1 — checkpoint/resume)
// ============================================================================

// POST /v1/tasks — create a new agent task (see task_handlers.rs)
