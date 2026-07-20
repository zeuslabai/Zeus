//! MiniMax OAuth backend — Device Code flow + Anthropic Messages API
//!
//! MiniMax's portal models (M2.5 series) expose an Anthropic-compatible
//! Messages API endpoint. Authentication uses a device code OAuth flow
//! (no local callback server — user visits a URL and enters a code).
//!
//! Key details (from OpenClaw minimax-portal-auth extension):
//! - Device code:  POST {base}/oauth/code
//! - Token poll:   POST {base}/oauth/token
//! - Client ID:    78257093-7e40-4613-99e0-527b14b39113
//! - Scope:        group_id profile model.completion
//! - Grant type:   urn:ietf:params:oauth:grant-type:user_code
//! - Inference:    https://api.minimax.io/anthropic/v1/messages  (global)
//!                 https://api.minimaxi.com/anthropic/v1/messages (CN)
//! - Auth header:  Authorization: Bearer {access_token}

use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{LazyLock, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use zeus_core::{Error, Message, Result, Role, ToolCall, ToolSchema};
use crate::{LlmResponse, StopReason};

// ─── Constants ───────────────────────────────────────────────────────────────

pub const MINIMAX_CLIENT_ID: &str = "78257093-7e40-4613-99e0-527b14b39113";
pub const MINIMAX_SCOPE: &str = "group_id profile model.completion";
pub const MINIMAX_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:user_code";

pub const MINIMAX_OAUTH_BASE_GLOBAL: &str = "https://api.minimax.io";
pub const MINIMAX_OAUTH_BASE_CN: &str = "https://api.minimaxi.com";

pub const MINIMAX_INFERENCE_BASE_GLOBAL: &str = "https://api.minimax.io/anthropic";
pub const MINIMAX_INFERENCE_BASE_CN: &str = "https://api.minimaxi.com/anthropic";

// ─── Token cache ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct MinimaxTokens {
    pub access: String,
    pub refresh: String,
    /// Unix timestamp (ms) when the access token expires.
    pub expires: u64,
    /// Dynamically assigned inference base URL from the OAuth response.
    pub resource_url: Option<String>,
}

static MINIMAX_TOKEN_CACHE: LazyLock<Mutex<Option<MinimaxTokens>>> =
    LazyLock::new(|| Mutex::new(None));

pub fn cache_minimax_tokens(tokens: MinimaxTokens) {
    if let Ok(mut guard) = MINIMAX_TOKEN_CACHE.lock() {
        *guard = Some(tokens);
    }
}

pub fn get_cached_minimax_tokens() -> Option<MinimaxTokens> {
    MINIMAX_TOKEN_CACHE.lock().ok()?.clone()
}

// ─── OAuth wire types ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct MinimaxDeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    /// Unix timestamp (ms) when this device code expires.
    pub expired_in: u64,
    pub interval: Option<u64>,
    pub state: String,
}

#[derive(Debug, Deserialize)]
struct MinimaxTokenResponse {
    status: String,
    access_token: Option<String>,
    refresh_token: Option<String>,
    expired_in: Option<u64>,
    resource_url: Option<String>,
    notification_message: Option<String>,
    base_resp: Option<MinimaxBaseResp>,
}

#[derive(Debug, Deserialize)]
struct MinimaxBaseResp {
    status_msg: Option<String>,
}

// ─── PKCE ─────────────────────────────────────────────────────────────────────

fn generate_pkce_verifier() -> String {
    use base64::Engine;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut h);
    std::thread::current().id().hash(&mut h);
    let seed = h.finish();
    let bytes: Vec<u8> = (0u8..32)
        .map(|i| {
            seed.wrapping_mul(6364136223846793005u64)
                .wrapping_add(i as u64 * 1442695040888963407u64)
                .wrapping_shr(33) as u8
        })
        .collect();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes)
}

fn generate_state() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut h);
    format!("{:016x}", h.finish())
}

// ─── Device code flow ─────────────────────────────────────────────────────────

/// Request a MiniMax device code. Returns the device code struct plus the
/// PKCE verifier (needed for token exchange).
pub async fn start_minimax_device_code(
    client: &Client,
    region: &str, // "global" or "cn"
) -> anyhow::Result<(MinimaxDeviceCode, String)> {
    let base = if region == "cn" { MINIMAX_OAUTH_BASE_CN } else { MINIMAX_OAUTH_BASE_GLOBAL };
    let verifier = generate_pkce_verifier();
    // For device code flow MiniMax accepts the verifier as-is for challenge
    let challenge = verifier.clone();
    let state = generate_state();

    let body = format!(
        "response_type=code&client_id={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
        MINIMAX_CLIENT_ID,
        urlencoding::encode(MINIMAX_SCOPE),
        urlencoding::encode(&challenge),
        urlencoding::encode(&state),
    );

    let resp = client
        .post(format!("{}/oauth/code", base))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("MiniMax device code request failed: {}", e))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("MiniMax device code error: {}", text));
    }

    let device: MinimaxDeviceCode = resp.json().await
        .map_err(|e| anyhow::anyhow!("Failed to parse MiniMax device code: {}", e))?;

    if device.state != state {
        return Err(anyhow::anyhow!("MiniMax OAuth state mismatch — possible CSRF"));
    }

    info!("MiniMax device code issued. Visit: {} — Code: {}", device.verification_uri, device.user_code);
    Ok((device, verifier))
}

/// Poll the MiniMax token endpoint until the user authorizes or the code expires.
pub async fn poll_minimax_token(
    client: &Client,
    user_code: &str,
    verifier: &str,
    expires_at_ms: u64,
    interval_ms: u64,
    region: &str,
) -> anyhow::Result<MinimaxTokens> {
    let base = if region == "cn" { MINIMAX_OAUTH_BASE_CN } else { MINIMAX_OAUTH_BASE_GLOBAL };
    let mut poll_interval = interval_ms;

    loop {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        if now >= expires_at_ms {
            return Err(anyhow::anyhow!("MiniMax OAuth timed out — device code expired"));
        }

        tokio::time::sleep(std::time::Duration::from_millis(poll_interval)).await;

        let body = format!(
            "grant_type={}&client_id={}&user_code={}&code_verifier={}",
            urlencoding::encode(MINIMAX_GRANT_TYPE),
            MINIMAX_CLIENT_ID,
            urlencoding::encode(user_code),
            urlencoding::encode(verifier),
        );

        let resp = match client
            .post(format!("{}/oauth/token", base))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => { warn!("MiniMax token poll error: {}", e); continue; }
        };

        let text = resp.text().await.unwrap_or_default();
        let payload: MinimaxTokenResponse = match serde_json::from_str(&text) {
            Ok(p) => p,
            Err(_) => { let end = zeus_core::floor_char_boundary(&text, 200); warn!("MiniMax token poll: unparseable response: {}", &text[..end]); continue; }
        };

        match payload.status.as_str() {
            "success" => {
                let access = payload.access_token
                    .ok_or_else(|| anyhow::anyhow!("MiniMax OAuth: missing access_token"))?;
                let refresh = payload.refresh_token
                    .ok_or_else(|| anyhow::anyhow!("MiniMax OAuth: missing refresh_token"))?;
                let expires = payload.expired_in
                    .ok_or_else(|| anyhow::anyhow!("MiniMax OAuth: missing expired_in"))?;
                if let Some(msg) = &payload.notification_message {
                    info!("MiniMax OAuth: {}", msg);
                }
                let tokens = MinimaxTokens { access, refresh, expires, resource_url: payload.resource_url };
                cache_minimax_tokens(tokens.clone());
                return Ok(tokens);
            }
            "error" => {
                let msg = payload.base_resp.and_then(|b| b.status_msg).unwrap_or_else(|| "unknown error".into());
                return Err(anyhow::anyhow!("MiniMax OAuth failed: {}", msg));
            }
            _ => {
                // "pending" — slow down slightly and keep polling
                poll_interval = poll_interval.saturating_add(500).min(10_000);
                debug!("MiniMax token poll: pending (interval={}ms)", poll_interval);
            }
        }
    }
}

// ─── Token refresh ───────────────────────────────────────────────────────────

/// Refresh a MiniMax access token using the stored refresh token.
pub async fn refresh_minimax_token(
    client: &Client,
    refresh_token: &str,
    region: &str,
) -> anyhow::Result<MinimaxTokens> {
    let base = if region == "cn" { MINIMAX_OAUTH_BASE_CN } else { MINIMAX_OAUTH_BASE_GLOBAL };

    let body = format!(
        "grant_type=refresh_token&client_id={}&refresh_token={}",
        MINIMAX_CLIENT_ID,
        urlencoding::encode(refresh_token),
    );

    let resp = client
        .post(format!("{}/oauth/token", base))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("MiniMax token refresh failed: {}", e))?;

    let text = resp.text().await.unwrap_or_default();
    let payload: MinimaxTokenResponse = serde_json::from_str(&text)
        .map_err(|e| { let end = zeus_core::floor_char_boundary(&text, 200); anyhow::anyhow!("MiniMax refresh: bad response: {} — {}", e, &text[..end]) })?;

    match payload.status.as_str() {
        "success" => {
            let access = payload.access_token
                .ok_or_else(|| anyhow::anyhow!("MiniMax refresh: missing access_token"))?;
            let refresh = payload.refresh_token
                .ok_or_else(|| anyhow::anyhow!("MiniMax refresh: missing refresh_token"))?;
            let expires = payload.expired_in
                .ok_or_else(|| anyhow::anyhow!("MiniMax refresh: missing expired_in"))?;
            let tokens = MinimaxTokens { access, refresh, expires, resource_url: payload.resource_url };
            cache_minimax_tokens(tokens.clone());
            // Persist to credential store so daemon restart doesn't force re-auth
            persist_minimax_credential(&tokens);
            info!("MiniMax OAuth token refreshed successfully");
            Ok(tokens)
        }
        _ => {
            let msg = payload.base_resp.and_then(|b| b.status_msg).unwrap_or_else(|| "unknown error".into());
            Err(anyhow::anyhow!("MiniMax token refresh failed: {} — re-run `zeus onboard` to re-authenticate", msg))
        }
    }
}

/// Check if the cached MiniMax token is fresh; refresh if expired or within 5min of expiry.
/// Returns the valid access token or an error.
pub async fn ensure_fresh_minimax_token(client: &Client, region: &str) -> Option<String> {
    let tokens = get_cached_minimax_tokens()?;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // If token expires within 5 minutes, refresh
    if now_ms + 300_000 >= tokens.expires {
        info!("MiniMax token expires in <5min, refreshing...");
        match refresh_minimax_token(client, &tokens.refresh, region).await {
            Ok(new_tokens) => return Some(new_tokens.access),
            Err(e) => {
                warn!("MiniMax token refresh failed: {}", e);
                // Return the old token anyway — it might still work for a few more minutes
                return Some(tokens.access);
            }
        }
    }

    Some(tokens.access)
}

/// Persist MiniMax tokens to the credential store (memory + disk).
fn persist_minimax_credential(tokens: &MinimaxTokens) {
    use chrono::{DateTime, Utc};
    let expires_at = DateTime::<Utc>::from_timestamp_millis(tokens.expires as i64)
        .unwrap_or_else(|| Utc::now() + chrono::Duration::hours(1));
    let credential = crate::oauth::StoredCredential {
        provider: "minimax".to_string(),
        kind: crate::oauth::CredentialKind::OAuthToken,
        token: tokens.access.clone(),
        refresh_token: tokens.refresh.clone(),
        expires_at,
        stored_at: Utc::now(),
    };
    crate::oauth::CredentialStore::store_in_memory(credential);
    // Best-effort disk persist — don't fail the refresh if disk write fails
    if let Ok(mut store) = crate::oauth::CredentialStore::load() {
        if let Err(e) = store.store(crate::oauth::StoredCredential {
            provider: "minimax".to_string(),
            kind: crate::oauth::CredentialKind::OAuthToken,
            token: tokens.access.clone(),
            refresh_token: tokens.refresh.clone(),
            expires_at,
            stored_at: Utc::now(),
        }) {
            warn!("Failed to persist MiniMax token to disk: {}", e);
        }
    }
}

// ─── Inference helpers ────────────────────────────────────────────────────────

fn to_anthropic_messages(messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    for msg in messages {
        match msg.role {
            Role::System => {}
            Role::User => {
                if msg.attachments.is_empty() {
                    out.push(json!({"role": "user", "content": msg.content}));
                } else {
                    // Multi-content block: images + text (Anthropic format)
                    let mut content_blocks: Vec<Value> = Vec::new();
                    for att in &msg.attachments {
                        if let Some(block) = crate::multimodal::format_anthropic_attachment(att) {
                            content_blocks.push(block);
                        }
                    }
                    if !msg.content.is_empty() {
                        content_blocks.push(json!({"type": "text", "text": msg.content}));
                    }
                    out.push(json!({"role": "user", "content": content_blocks}));
                }
            }
            Role::Assistant => {
                if !msg.tool_calls.is_empty() {
                    let mut blocks: Vec<Value> = Vec::new();
                    if !msg.content.is_empty() {
                        blocks.push(json!({"type": "text", "text": msg.content}));
                    }
                    for tc in &msg.tool_calls {
                        blocks.push(json!({"type": "tool_use", "id": tc.id, "name": tc.name, "input": tc.arguments}));
                    }
                    out.push(json!({"role": "assistant", "content": blocks}));
                } else if !msg.content.is_empty() {
                    out.push(json!({"role": "assistant", "content": msg.content}));
                }
            }
            Role::Tool => {
                let blocks: Vec<Value> = msg.tool_results.iter().map(|tr| json!({
                    "type": "tool_result",
                    "tool_use_id": tr.call_id,
                    "content": tr.output,
                })).collect();
                out.push(json!({"role": "user", "content": blocks}));
            }
        }
    }
    out
}

fn to_anthropic_tools(tools: &[ToolSchema]) -> Vec<Value> {
    tools.iter().map(|t| json!({
        "name": t.name,
        "description": t.description,
        "input_schema": t.parameters,
    })).collect()
}

fn parse_anthropic_response(response: &Value) -> Result<LlmResponse> {
    let mut content = String::new();
    let mut tool_calls = Vec::new();

    if let Some(blocks) = response.get("content").and_then(|c| c.as_array()) {
        for block in blocks {
            match block.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "text" => {
                    if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                        content.push_str(t);
                    }
                }
                "tool_use" => {
                    tool_calls.push(ToolCall {
                        id: block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        name: block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        arguments: block.get("input").cloned().unwrap_or(json!({})),
                    });
                }
                _ => {}
            }
        }
    }

    let usage = response.get("usage");
    let input_tokens = usage.and_then(|u| u.get("input_tokens")).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let output_tokens = usage.and_then(|u| u.get("output_tokens")).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let stop_reason = if !tool_calls.is_empty() { StopReason::ToolUse } else { StopReason::EndTurn };

    Ok(LlmResponse { content, tool_calls, stop_reason, input_tokens, output_tokens, cached_tokens: 0 })
}

fn inference_url(resource_url: Option<&str>, region: &str) -> String {
    let base = resource_url
        .map(|u| format!("{}/anthropic", u.trim_end_matches('/')))
        .unwrap_or_else(|| {
            if region == "cn" { MINIMAX_INFERENCE_BASE_CN.to_string() }
            else { MINIMAX_INFERENCE_BASE_GLOBAL.to_string() }
        });
    format!("{}/v1/messages", base)
}

// ─── Inference — non-streaming ────────────────────────────────────────────────

pub async fn complete_minimax(
    client: &Client,
    model: &str,
    messages: &[Message],
    tools: &[ToolSchema],
    system: Option<&str>,
    access_token: &str,
    resource_url: Option<&str>,
    region: &str,
) -> Result<LlmResponse> {
    let url = inference_url(resource_url, region);
    let mut body = json!({
        "model": model,
        "max_tokens": 8192,
        "messages": to_anthropic_messages(messages),
    });
    if let Some(sys) = system { body["system"] = json!(sys); }
    if !tools.is_empty() { body["tools"] = json!(to_anthropic_tools(tools)); }

    debug!("MiniMax complete → {}", url);

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Llm(format!("MiniMax request failed: {}", e)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::Llm(format!("MiniMax API error {}: {}", status, text)));
    }

    let response: Value = resp.json().await
        .map_err(|e| Error::Llm(format!("Failed to parse MiniMax response: {}", e)))?;

    parse_anthropic_response(&response)
}

// ─── Inference — streaming ────────────────────────────────────────────────────

pub async fn stream_minimax(
    client: &Client,
    model: &str,
    messages: &[Message],
    tools: &[ToolSchema],
    system: Option<&str>,
    access_token: &str,
    resource_url: Option<&str>,
    region: &str,
) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
    let url = inference_url(resource_url, region);
    let mut body = json!({
        "model": model,
        "max_tokens": 8192,
        "messages": to_anthropic_messages(messages),
        "stream": true,
    });
    if let Some(sys) = system { body["system"] = json!(sys); }
    if !tools.is_empty() { body["tools"] = json!(to_anthropic_tools(tools)); }

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Llm(format!("MiniMax stream request failed: {}", e)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(Error::Llm(format!("MiniMax stream API error {}: {}", status, text)));
    }

    let (tx, rx) = mpsc::channel(100);

    let handle = tokio::spawn(async move {
        use futures::StreamExt;
        let mut content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut input_tokens: usize = 0;
        let mut output_tokens: usize = 0;
        let mut cur_tool_id = String::new();
        let mut cur_tool_name = String::new();
        let mut cur_tool_args = String::new();

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk { Ok(c) => c, Err(e) => { warn!("MiniMax stream chunk error: {}", e); continue; } };
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buffer.find("\n\n") {
                let block = buffer[..pos].to_string();
                buffer = buffer[pos + 2..].to_string();

                for line in block.lines() {
                    let Some(data) = line.strip_prefix("data: ") else { continue };
                    if data == "[DONE]" { continue; }
                    let Ok(event) = serde_json::from_str::<Value>(data) else { continue };

                    match event.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                        "content_block_start" => {
                            if let Some(cb) = event.get("content_block") {
                                if cb.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                                    cur_tool_id = cb.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    cur_tool_name = cb.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    cur_tool_args.clear();
                                }
                            }
                        }
                        "content_block_delta" => {
                            if let Some(delta) = event.get("delta") {
                                match delta.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                                    "text_delta" => {
                                        if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                            content.push_str(text);
                                            let _ = tx.send(text.to_string()).await;
                                        }
                                    }
                                    "input_json_delta" => {
                                        if let Some(partial) = delta.get("partial_json").and_then(|t| t.as_str()) {
                                            cur_tool_args.push_str(partial);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        "content_block_stop" => {
                            if !cur_tool_name.is_empty() {
                                let args: Value = serde_json::from_str(&cur_tool_args).unwrap_or(json!({}));
                                tool_calls.push(ToolCall {
                                    id: if cur_tool_id.is_empty() { format!("call_{}", uuid::Uuid::new_v4()) } else { cur_tool_id.clone() },
                                    name: cur_tool_name.clone(),
                                    arguments: args,
                                });
                                cur_tool_id.clear(); cur_tool_name.clear(); cur_tool_args.clear();
                            }
                        }
                        "message_start" => {
                            if let Some(msg) = event.get("message") {
                                if let Some(usage) = msg.get("usage") {
                                    input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                                }
                            }
                        }
                        "message_delta" => {
                            if let Some(usage) = event.get("usage") {
                                output_tokens = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let stop_reason = if !tool_calls.is_empty() { StopReason::ToolUse } else { StopReason::EndTurn };
        LlmResponse { content, tool_calls, stop_reason, input_tokens, output_tokens, cached_tokens: 0 }
    });

    Ok((rx, handle))
}
