//! Qwen Portal OAuth — Device Code flow
//!
//! Qwen's portal models expose an OpenAI-compatible API endpoint.
//! Authentication uses a standard RFC 8628 device code OAuth flow.
//!
//! Key details (from OpenClaw qwen-portal-auth extension):
//! - Device code:  POST https://chat.qwen.ai/api/v1/oauth2/device/code
//! - Token poll:   POST https://chat.qwen.ai/api/v1/oauth2/device/token
//! - Client ID:    f0304373b74a44d2b584a3fb70ca9e56
//! - Inference:    https://portal.qwen.ai/v1 (OpenAI-compatible)
//! - Auth header:  Authorization: Bearer {access_token}

use reqwest::Client;
use serde::Deserialize;
use std::sync::{LazyLock, Mutex};
use tracing::{debug, info, warn};

// ─── Constants ────────────────────────────────────────────────────────────────

pub const QWEN_OAUTH_CLIENT_ID: &str = "f0304373b74a44d2b584a3fb70ca9e56";
pub const QWEN_OAUTH_BASE: &str = "https://chat.qwen.ai";
pub const QWEN_PORTAL_BASE: &str = "https://portal.qwen.ai/v1";

// ─── Token cache ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct QwenTokens {
    pub access: String,
    pub refresh: Option<String>,
    /// Unix timestamp (seconds) when the access token expires, if known.
    pub expires_at: Option<u64>,
}

static QWEN_TOKEN_CACHE: LazyLock<Mutex<Option<QwenTokens>>> =
    LazyLock::new(|| Mutex::new(None));

pub fn cache_qwen_tokens(tokens: QwenTokens) {
    if let Ok(mut guard) = QWEN_TOKEN_CACHE.lock() {
        *guard = Some(tokens);
    }
}

pub fn get_cached_qwen_tokens() -> Option<QwenTokens> {
    QWEN_TOKEN_CACHE.lock().ok()?.clone()
}

// ─── Wire types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct QwenDeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,    // seconds
    pub interval: u64,      // polling interval (seconds)
}

#[derive(Debug, Deserialize)]
struct QwenTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    /// "authorization_pending", "slow_down", "expired_token", "access_denied"
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

// ─── Device code flow ─────────────────────────────────────────────────────────

/// Request a Qwen device code. Returns the device code struct.
pub async fn start_qwen_device_code(client: &Client) -> anyhow::Result<QwenDeviceCode> {
    let url = format!("{}/api/v1/oauth2/device/code", QWEN_OAUTH_BASE);
    let body = format!("client_id={}", QWEN_OAUTH_CLIENT_ID);

    let resp = client
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Qwen device code request failed: {}", e))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Qwen device code error: {}", text));
    }

    let device: QwenDeviceCode = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to parse Qwen device code: {}", e))?;

    info!(
        "Qwen device code issued. Visit: {} — Code: {}",
        device.verification_uri, device.user_code
    );
    Ok(device)
}

/// Poll the Qwen token endpoint until the user authorizes or the code expires.
pub async fn poll_qwen_token(
    client: &Client,
    device_code: &str,
    expires_in_secs: u64,
    interval_secs: u64,
) -> anyhow::Result<QwenTokens> {
    let url = format!("{}/api/v1/oauth2/device/token", QWEN_OAUTH_BASE);
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(expires_in_secs.saturating_add(5));
    let mut interval = interval_secs.max(5);

    loop {
        if std::time::Instant::now() >= deadline {
            return Err(anyhow::anyhow!("Qwen OAuth timed out — device code expired"));
        }

        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

        let body = format!(
            "grant_type=urn:ietf:params:oauth:grant-type:device_code&client_id={}&device_code={}",
            QWEN_OAUTH_CLIENT_ID,
            urlencoding::encode(device_code),
        );

        let resp = match client
            .post(&url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Qwen token poll error: {}", e);
                continue;
            }
        };

        let payload: QwenTokenResponse = match resp.json().await {
            Ok(p) => p,
            Err(e) => {
                warn!("Qwen token poll: failed to parse response: {}", e);
                continue;
            }
        };

        if let Some(access) = payload.access_token {
            let expires_at = payload.expires_in.map(|secs| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    + secs
            });
            let tokens = QwenTokens {
                access,
                refresh: payload.refresh_token,
                expires_at,
            };
            cache_qwen_tokens(tokens.clone());
            return Ok(tokens);
        }

        match payload.error.as_deref() {
            Some("authorization_pending") => {
                debug!("Qwen token poll: authorization pending (interval={}s)", interval);
            }
            Some("slow_down") => {
                interval = interval.saturating_add(5);
                debug!("Qwen token poll: slow_down → interval={}s", interval);
            }
            Some("expired_token") => {
                return Err(anyhow::anyhow!("Qwen OAuth expired — request a new device code"));
            }
            Some("access_denied") => {
                return Err(anyhow::anyhow!("Qwen OAuth denied by user"));
            }
            Some(other) => {
                let desc = payload.error_description.unwrap_or_default();
                return Err(anyhow::anyhow!("Qwen OAuth error: {} — {}", other, desc));
            }
            None => {
                debug!("Qwen token poll: no error, no token — still waiting");
            }
        }
    }
}

// ─── Token refresh ───────────────────────────────────────────────────────────

/// Refresh a Qwen access token using the stored refresh token.
pub async fn refresh_qwen_token(
    client: &Client,
    refresh_token: &str,
) -> anyhow::Result<QwenTokens> {
    let url = format!("{}/api/v1/oauth2/device/token", QWEN_OAUTH_BASE);

    let body = format!(
        "grant_type=refresh_token&client_id={}&refresh_token={}",
        QWEN_OAUTH_CLIENT_ID,
        urlencoding::encode(refresh_token),
    );

    let resp = client
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Qwen token refresh failed: {}", e))?;

    let payload: QwenTokenResponse = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Qwen refresh: bad response: {}", e))?;

    if let Some(access) = payload.access_token {
        let expires_at = payload.expires_in.map(|secs| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                + secs
        });
        let tokens = QwenTokens {
            access,
            refresh: payload.refresh_token.or(Some(refresh_token.to_string())),
            expires_at,
        };
        cache_qwen_tokens(tokens.clone());
        persist_qwen_credential(&tokens);
        info!("Qwen OAuth token refreshed successfully");
        Ok(tokens)
    } else {
        let err = payload.error.unwrap_or_default();
        let desc = payload.error_description.unwrap_or_default();
        Err(anyhow::anyhow!(
            "Qwen token refresh failed: {} {} — re-authenticate: zeus onboard (Auth step)",
            err, desc
        ))
    }
}

/// Persist Qwen tokens to the credential store (memory + disk).
fn persist_qwen_credential(tokens: &QwenTokens) {
    use chrono::{DateTime, Utc};
    let expires_at = tokens.expires_at
        .and_then(|ts| DateTime::<Utc>::from_timestamp(ts as i64, 0))
        .unwrap_or_else(|| Utc::now() + chrono::Duration::hours(1));
    let credential = crate::oauth::StoredCredential {
        provider: "qwen".to_string(),
        kind: crate::oauth::CredentialKind::OAuthToken,
        token: tokens.access.clone(),
        refresh_token: tokens.refresh.clone().unwrap_or_default(),
        expires_at,
        stored_at: Utc::now(),
    };
    crate::oauth::CredentialStore::store_in_memory(credential);
    if let Ok(mut store) = crate::oauth::CredentialStore::load() {
        if let Err(e) = store.store(crate::oauth::StoredCredential {
            provider: "qwen".to_string(),
            kind: crate::oauth::CredentialKind::OAuthToken,
            token: tokens.access.clone(),
            refresh_token: tokens.refresh.clone().unwrap_or_default(),
            expires_at,
            stored_at: Utc::now(),
        }) {
            warn!("Failed to persist Qwen token to disk: {}", e);
        }
    }
}

/// Check if the cached Qwen token is fresh; refresh if expired or within 5min of expiry.
/// Returns the valid access token or None.
pub async fn ensure_fresh_qwen_token(client: &Client) -> Option<String> {
    let tokens = get_cached_qwen_tokens()?;

    if let Some(expires_at) = tokens.expires_at {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // If token expires within 5 minutes, refresh
        if now + 300 >= expires_at {
            if let Some(ref refresh) = tokens.refresh {
                info!("Qwen token expires in <5min, refreshing...");
                match refresh_qwen_token(client, refresh).await {
                    Ok(new_tokens) => return Some(new_tokens.access),
                    Err(e) => {
                        warn!("Qwen token refresh failed: {}", e);
                        return Some(tokens.access);
                    }
                }
            }
        }
    }

    Some(tokens.access)
}
