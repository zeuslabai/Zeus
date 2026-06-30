//! Device-code OAuth + CLI-credential endpoints (#216b backend)
//!
//! Qwen and MiniMax are NOT routed through the generic RFC 8628 engine in
//! zeus-auth (`run_device_code_flow`) — MiniMax uses a non-standard grant_type
//! (`urn:ietf:params:oauth:grant-type:user_code`) and both need custom token
//! caching + inference URL handling. These handlers dispatch on the `provider`
//! param to the provider-specific modules in zeus-llm (`qwen_oauth`, `minimax`).
//!
//! Flow:
//!   POST /v1/auth/device/start  {provider, region?} → session_id + user_code + verification_uri
//!     (spawns a background poll task that completes the flow server-side)
//!   GET  /v1/auth/device/poll?session=<id>          → {status: pending|complete|error}
//!
//! CLI creds:
//!   GET  /v1/auth/cli-creds         → detection only (no raw secrets — /v1/auth/* is auth-exempt)
//!   POST /v1/auth/cli-creds/import  → import Gemini CLI creds into the credential store
//!
//! On successful device-code auth, tokens are cached in-process
//! (`cache_qwen_tokens` / `cache_minimax_tokens`) and persisted to the
//! zeus-llm CredentialStore so inference picks them up immediately.

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

// ─── Pending session registry ─────────────────────────────────────────────────

#[derive(Clone, Debug)]
enum DeviceSessionStatus {
    Pending,
    Complete,
    Error(String),
}

#[derive(Clone, Debug)]
struct DeviceSession {
    provider: String,
    status: DeviceSessionStatus,
}

static DEVICE_SESSIONS: LazyLock<Mutex<HashMap<String, DeviceSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn set_session(id: &str, provider: &str, status: DeviceSessionStatus) {
    if let Ok(mut map) = DEVICE_SESSIONS.lock() {
        map.insert(
            id.to_string(),
            DeviceSession {
                provider: provider.to_string(),
                status,
            },
        );
    }
}

fn get_session(id: &str) -> Option<DeviceSession> {
    DEVICE_SESSIONS.lock().ok()?.get(id).cloned()
}

fn remove_session(id: &str) {
    if let Ok(mut map) = DEVICE_SESSIONS.lock() {
        map.remove(id);
    }
}

// ─── Credential persistence ───────────────────────────────────────────────────

fn store_credential(
    provider: &str,
    access: &str,
    refresh: Option<String>,
    expires_at: chrono::DateTime<chrono::Utc>,
) {
    match zeus_llm::CredentialStore::load() {
        Ok(mut store) => {
            if let Err(e) = store.store(zeus_llm::StoredCredential {
                provider: provider.to_string(),
                kind: zeus_llm::CredentialKind::OAuthToken,
                token: access.to_string(),
                refresh_token: refresh.unwrap_or_default(),
                expires_at,
                stored_at: chrono::Utc::now(),
            }) {
                tracing::warn!("Failed to persist {} device-code credential: {}", provider, e);
            }
        }
        Err(e) => tracing::warn!("Failed to load credential store: {}", e),
    }
}

// ─── POST /v1/auth/device/start ───────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DeviceStartRequest {
    pub provider: String,
    #[serde(default)]
    pub region: Option<String>,
}

pub async fn device_code_start(
    body: Option<Json<DeviceStartRequest>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let Some(Json(req)) = body else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Missing JSON body: {\"provider\": \"qwen\"|\"minimax\"}".to_string(),
        ));
    };

    let client = reqwest::Client::new();
    let session_id = uuid::Uuid::new_v4().to_string();

    match req.provider.as_str() {
        "qwen" => {
            let code = zeus_llm::qwen_oauth::start_qwen_device_code(&client)
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Qwen device code request failed: {e}")))?;

            set_session(&session_id, "qwen", DeviceSessionStatus::Pending);

            let device_code = code.device_code.clone();
            let expires_in = code.expires_in;
            let interval = code.interval;
            let sid = session_id.clone();
            tokio::spawn(async move {
                match zeus_llm::qwen_oauth::poll_qwen_token(&client, &device_code, expires_in, interval).await {
                    Ok(tokens) => {
                        let expires_at = tokens
                            .expires_at
                            .and_then(|secs| chrono::DateTime::from_timestamp(secs as i64, 0))
                            .unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::hours(1));
                        store_credential("qwen", &tokens.access, tokens.refresh.clone(), expires_at);
                        zeus_llm::qwen_oauth::cache_qwen_tokens(tokens);
                        set_session(&sid, "qwen", DeviceSessionStatus::Complete);
                    }
                    Err(e) => set_session(&sid, "qwen", DeviceSessionStatus::Error(e.to_string())),
                }
            });

            Ok(Json(json!({
                "session_id": session_id,
                "provider": "qwen",
                "user_code": code.user_code,
                "verification_uri": code.verification_uri,
                "verification_uri_complete": code.verification_uri_complete,
                "expires_in": code.expires_in,
                "interval": code.interval,
            })))
        }
        "minimax" => {
            let region = req.region.as_deref().unwrap_or("global").to_string();
            let (code, verifier) = zeus_llm::minimax::start_minimax_device_code(&client, &region)
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, format!("MiniMax device code request failed: {e}")))?;

            set_session(&session_id, "minimax", DeviceSessionStatus::Pending);

            let user_code = code.user_code.clone();
            let expires_at_ms = code.expired_in;
            let interval_ms = code.interval.unwrap_or(5000);
            let sid = session_id.clone();
            let region_bg = region.clone();
            tokio::spawn(async move {
                match zeus_llm::minimax::poll_minimax_token(
                    &client, &user_code, &verifier, expires_at_ms, interval_ms, &region_bg,
                )
                .await
                {
                    Ok(tokens) => {
                        let expires_at = chrono::DateTime::from_timestamp_millis(tokens.expires as i64)
                            .unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::hours(1));
                        store_credential("minimax", &tokens.access, Some(tokens.refresh.clone()), expires_at);
                        zeus_llm::minimax::cache_minimax_tokens(tokens);
                        set_session(&sid, "minimax", DeviceSessionStatus::Complete);
                    }
                    Err(e) => set_session(&sid, "minimax", DeviceSessionStatus::Error(e.to_string())),
                }
            });

            Ok(Json(json!({
                "session_id": session_id,
                "provider": "minimax",
                "user_code": code.user_code,
                "verification_uri": code.verification_uri,
                "expires_at_ms": expires_at_ms,
                "interval_ms": interval_ms,
                "region": region,
            })))
        }
        other => Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Unsupported device-code provider '{other}'. Supported: qwen, minimax."
            ),
        )),
    }
}

// ─── GET /v1/auth/device/poll ─────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DevicePollParams {
    pub session: String,
}

pub async fn device_code_poll(Query(params): Query<DevicePollParams>) -> Json<Value> {
    match get_session(&params.session) {
        Some(session) => match session.status {
            DeviceSessionStatus::Pending => Json(json!({
                "status": "pending",
                "provider": session.provider,
            })),
            DeviceSessionStatus::Complete => {
                remove_session(&params.session);
                Json(json!({
                    "status": "complete",
                    "provider": session.provider,
                }))
            }
            DeviceSessionStatus::Error(e) => {
                remove_session(&params.session);
                Json(json!({
                    "status": "error",
                    "provider": session.provider,
                    "error": e,
                }))
            }
        },
        None => Json(json!({
            "status": "unknown",
            "error": "No such device-code session (expired, already consumed, or never started)",
        })),
    }
}

// ─── GET /v1/auth/cli-creds ───────────────────────────────────────────────────
//
// Detection only. /v1/auth/* routes are auth-exempt, so raw tokens are never
// returned here — the WebUI only needs to know whether creds exist.

pub async fn cli_creds_status() -> Json<Value> {
    let gemini = match zeus_auth::import_gemini_cli_credentials() {
        Ok(creds) => json!({
            "found": true,
            "has_refresh_token": creds.refresh_token.is_some(),
            "has_client_id": creds.client_id.is_some(),
            "expiry": creds.expiry,
        }),
        Err(e) => json!({
            "found": false,
            "error": e.to_string(),
        }),
    };

    let client_credentials = match zeus_auth::extract_gemini_cli_credentials() {
        Some((client_id, _secret)) => {
            // Mask: show enough to identify, never the secret
            let masked: String = client_id.chars().take(12).collect();
            json!({ "found": true, "client_id_prefix": masked })
        }
        None => json!({ "found": false }),
    };

    Json(json!({
        "gemini": gemini,
        "client_credentials": client_credentials,
    }))
}

// ─── POST /v1/auth/cli-creds/import ───────────────────────────────────────────

pub async fn cli_creds_import() -> Result<Json<Value>, (StatusCode, String)> {
    let creds = zeus_auth::import_gemini_cli_credentials().map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            format!("No Gemini CLI credentials found: {e}"),
        )
    })?;

    let expires_at = creds
        .expiry
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::hours(1));

    store_credential("google", &creds.access_token, creds.refresh_token.clone(), expires_at);

    Ok(Json(json!({
        "imported": true,
        "provider": "google",
        "has_refresh_token": creds.refresh_token.is_some(),
        "expires_at": expires_at.to_rfc3339(),
    })))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_device_start_rejects_unknown_provider() {
        let body = Json(DeviceStartRequest {
            provider: "anthropic".to_string(),
            region: None,
        });
        let result = device_code_start(Some(body)).await;
        let err = result.err().expect("unknown provider must be rejected");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("anthropic"));
    }

    #[tokio::test]
    async fn test_device_start_rejects_missing_body() {
        let result = device_code_start(None).await;
        let err = result.err().expect("missing body must be rejected");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_device_poll_unknown_session() {
        let Json(result) = device_code_poll(Query(DevicePollParams {
            session: "no-such-session".to_string(),
        }))
        .await;
        assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("unknown"));
    }

    #[tokio::test]
    async fn test_session_registry_roundtrip() {
        set_session("s1", "qwen", DeviceSessionStatus::Pending);
        let Json(pending) = device_code_poll(Query(DevicePollParams {
            session: "s1".to_string(),
        }))
        .await;
        assert_eq!(pending.get("status").and_then(|v| v.as_str()), Some("pending"));

        // Terminal states are consumed on read
        set_session("s1", "qwen", DeviceSessionStatus::Complete);
        let Json(done) = device_code_poll(Query(DevicePollParams {
            session: "s1".to_string(),
        }))
        .await;
        assert_eq!(done.get("status").and_then(|v| v.as_str()), Some("complete"));
        assert!(get_session("s1").is_none(), "complete session must be consumed");

        set_session("s2", "minimax", DeviceSessionStatus::Error("denied".into()));
        let Json(err) = device_code_poll(Query(DevicePollParams {
            session: "s2".to_string(),
        }))
        .await;
        assert_eq!(err.get("status").and_then(|v| v.as_str()), Some("error"));
        assert_eq!(err.get("error").and_then(|v| v.as_str()), Some("denied"));
    }

    #[tokio::test]
    async fn test_cli_creds_status_shape() {
        // Must always return both keys regardless of whether creds exist on this box
        let Json(result) = cli_creds_status().await;
        assert!(result.get("gemini").is_some());
        assert!(result.get("client_credentials").is_some());
        let gemini = result.get("gemini").unwrap();
        assert!(gemini.get("found").and_then(|v| v.as_bool()).is_some());
        // Never leak raw tokens
        assert!(gemini.get("access_token").is_none());
        assert!(gemini.get("refresh_token").is_none());
    }
}
