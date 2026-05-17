//! Channel handler endpoints extracted from mod.rs (A3 handlers split)

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::info;

use crate::SharedState;

#[derive(Debug, Deserialize)]
pub struct ChannelBroadcastRequest {
    /// Message content to broadcast
    pub message: String,
    /// Optional event type (e.g. "sprint_complete", "agent_spawned", "error")
    #[serde(default)]
    pub event_type: Option<String>,
}

pub async fn list_channels(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let channels = state.channel_store.list().await;

    let entries: Vec<Value> = channels
        .iter()
        .map(|ch| {
            let status = if ch.enabled { "connected" } else { "disabled" };
            json!({
                "id": ch.id,
                "type": ch.channel_type,
                "name": ch.name,
                "status": status,
                "enabled": ch.enabled,
                "connected_at": ch.created_at.to_rfc3339(),
                "last_message_at": ch.last_message_at.map(|t| t.to_rfc3339()),
            })
        })
        .collect();

    Json(json!({ "channels": entries, "count": entries.len() }))
}

pub async fn channel_broadcast(
    State(state): State<SharedState>,
    Json(body): Json<ChannelBroadcastRequest>,
) -> impl IntoResponse {
    let state = state.read().await;
    let mgr = &state.channel_manager;

    let results = mgr.broadcast_all(&body.message).await;

    let delivered: Vec<Value> = results
        .iter()
        .map(|(channel_type, result)| {
            json!({
                "channel": channel_type,
                "ok": result.is_ok(),
                "error": result.as_ref().err().map(|e| e.to_string()),
            })
        })
        .collect();

    let success_count = results.iter().filter(|(_, r)| r.is_ok()).count();
    let total = results.len();

    Json(json!({
        "ok": true,
        "message": body.message,
        "event_type": body.event_type,
        "delivered": delivered,
        "success_count": success_count,
        "total_channels": total,
    }))
}

pub async fn create_channel(
    State(state): State<SharedState>,
    Json(req): Json<crate::channels::CreateChannelRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let state = state.read().await;

    let channel = state
        .channel_store
        .create(req)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    info!(
        "Created channel: {} ({})",
        channel.name, channel.channel_type
    );

    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(&channel).unwrap_or_default()),
    ))
}

pub async fn get_channel(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let channel = state
        .channel_store
        .get(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Channel not found: {}", id)))?;

    Ok(Json(serde_json::to_value(&channel).unwrap_or_default()))
}

pub async fn update_channel(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<crate::channels::UpdateChannelRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let channel = state
        .channel_store
        .update(&id, req)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;

    info!("Updated channel: {} ({})", channel.name, channel.id);

    Ok(Json(serde_json::to_value(&channel).unwrap_or_default()))
}

pub async fn delete_channel(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    state
        .channel_store
        .delete(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;

    info!("Deleted channel: {}", id);

    Ok(Json(json!({ "deleted": true, "id": id })))
}

pub async fn test_channel(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let channel = state
        .channel_store
        .get(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Channel not found: {}", id)))?;

    // Basic connectivity check: verify required config keys are present
    let (ok, detail) = match channel.channel_type {
        crate::channels::ChannelType::Telegram => {
            let has_token = channel.config.contains_key("bot_token");
            let has_chat = channel.config.contains_key("chat_id");
            if has_token && has_chat {
                (true, "bot_token and chat_id present".to_string())
            } else {
                let mut missing = Vec::new();
                if !has_token {
                    missing.push("bot_token");
                }
                if !has_chat {
                    missing.push("chat_id");
                }
                (
                    false,
                    format!("Missing config keys: {}", missing.join(", ")),
                )
            }
        }
        crate::channels::ChannelType::Discord => {
            if channel.config.contains_key("bot_token") {
                (true, "bot_token present".to_string())
            } else {
                (false, "Missing config key: bot_token".to_string())
            }
        }
        crate::channels::ChannelType::Slack => {
            let has_bot = channel.config.contains_key("bot_token");
            let has_app = channel.config.contains_key("app_token");
            if has_bot && has_app {
                (true, "bot_token and app_token present".to_string())
            } else {
                let mut missing = Vec::new();
                if !has_bot {
                    missing.push("bot_token");
                }
                if !has_app {
                    missing.push("app_token");
                }
                (
                    false,
                    format!("Missing config keys: {}", missing.join(", ")),
                )
            }
        }
        crate::channels::ChannelType::Webhook => {
            if channel.config.contains_key("webhook_url") {
                (true, "webhook_url present".to_string())
            } else {
                (false, "Missing config key: webhook_url".to_string())
            }
        }
    };

    Ok(Json(json!({
        "id": channel.id,
        "name": channel.name,
        "type": channel.channel_type,
        "ok": ok,
        "detail": detail,
        "enabled": channel.enabled,
    })))
}

pub async fn channel_status(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    match state.channel_store.get(&id).await {
        Some(ch) => Ok(Json(json!({
            "id": ch.id,
            "name": ch.name,
            "type": ch.channel_type,
            "status": if ch.enabled { "connected" } else { "disabled" },
            "enabled": ch.enabled,
            "last_message_at": ch.last_message_at.map(|t| t.to_rfc3339()),
            "error": null,
        }))),
        None => Ok(Json(json!({
            "id": id,
            "status": "not_found",
            "error": "Channel not configured",
        }))),
    }
}

pub async fn channel_health(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;

    // Build health status from channel store
    let channels = state.channel_store.list().await;
    let mut channel_reports = Vec::new();
    let mut connected_count = 0usize;

    for ch in &channels {
        let connected = ch.enabled;
        if connected {
            connected_count += 1;
        }
        channel_reports.push(json!({
            "channel_type": ch.channel_type,
            "name": ch.name,
            "connected": connected,
            "last_check": chrono::Utc::now().to_rfc3339(),
            "last_connected": ch.last_message_at.map(|t| t.to_rfc3339()),
            "consecutive_failures": 0,
            "uptime_pct": if connected { 100.0 } else { 0.0 },
        }));
    }

    let total_count = channels.len();
    let overall_healthy = connected_count == total_count && total_count > 0;

    Json(json!({
        "overall_healthy": overall_healthy,
        "connected_count": connected_count,
        "total_count": total_count,
        "channels": channel_reports,
        "checked_at": chrono::Utc::now().to_rfc3339(),
    }))
}

pub async fn connect_channel(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let channel = state
        .channel_store
        .get(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Channel not found: {}", id)))?;

    // Mark as enabled in the store
    let update = crate::channels::UpdateChannelRequest {
        name: None,
        config: None,
        enabled: Some(true),
    };
    state
        .channel_store
        .update(&id, update)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    info!("Connected channel: {} ({})", channel.name, channel.id);

    Ok(Json(json!({
        "message": format!("Channel '{}' connected", channel.name),
        "id": channel.id,
        "status": "connected",
    })))
}

pub async fn disconnect_channel(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let channel = state
        .channel_store
        .get(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Channel not found: {}", id)))?;

    // Mark as disabled in the store
    let update = crate::channels::UpdateChannelRequest {
        name: None,
        config: None,
        enabled: Some(false),
    };
    state
        .channel_store
        .update(&id, update)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    info!("Disconnected channel: {} ({})", channel.name, channel.id);

    Ok(Json(json!({
        "message": format!("Channel '{}' disconnected", channel.name),
        "id": channel.id,
        "status": "disconnected",
    })))
}

pub async fn pair_channel(
    State(state): State<SharedState>,
    Path(channel_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let user_id = body
        .get("user_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Missing 'user_id' field".to_string(),
            )
        })?;

    let channel_type = body
        .get("channel_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let state = state.read().await;
    let code = state
        .pairing_manager
        .generate_code(&channel_id, user_id, channel_type);

    Ok(Json(json!({
        "code": code,
        "channel_id": channel_id,
        "user_id": user_id,
        "expires_in_secs": 900,
        "message": format!("Send this code to verify: {}", code),
    })))
}

pub async fn verify_channel(
    State(state): State<SharedState>,
    Path(channel_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let code = body
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'code' field".to_string()))?;

    let state = state.read().await;
    let pairing = state
        .pairing_manager
        .verify_code(code)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    if pairing.channel_id != channel_id {
        return Err((
            StatusCode::BAD_REQUEST,
            "Code does not match this channel".to_string(),
        ));
    }

    Ok(Json(json!({
        "verified": true,
        "user_id": pairing.user_id,
        "channel_id": pairing.channel_id,
        "channel_type": pairing.channel_type,
        "verified_at": pairing.verified_at.to_rfc3339(),
    })))
}

pub async fn list_channel_pairings(
    State(state): State<SharedState>,
    Path(channel_id): Path<String>,
) -> Json<Value> {
    let state = state.read().await;
    let pairings = state.pairing_manager.pairings_for_channel(&channel_id);
    let items: Vec<Value> = pairings
        .iter()
        .map(|p| {
            json!({
                "user_id": p.user_id,
                "channel_id": p.channel_id,
                "channel_type": p.channel_type,
                "verified_at": p.verified_at.to_rfc3339(),
            })
        })
        .collect();
    Json(json!({
        "channel_id": channel_id,
        "pairings": items,
        "total": items.len(),
    }))
}

/// POST /v1/channels/signal/link-uri
/// Runs `signal-cli link -n zeus`, reads the tsdevice:// pairing URI from stdout,
/// and returns it so the WebUI can render it as a QR code for secondary-device linking.
pub async fn signal_link_uri() -> impl IntoResponse {
    use tokio::process::Command;
    use tokio::time::{timeout, Duration};

    let result = timeout(Duration::from_secs(30), async {
        Command::new("signal-cli")
            .args(["link", "-n", "zeus"])
            .output()
            .await
    })
    .await;

    match result {
        Ok(Ok(output)) => {
            // signal-cli prints the tsdevice:// URI to stdout
            let stdout = String::from_utf8_lossy(&output.stdout);
            let uri = stdout
                .lines()
                .find(|l| l.starts_with("tsdevice://"))
                .unwrap_or("")
                .trim()
                .to_string();
            if uri.is_empty() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "No tsdevice:// URI in signal-cli output", "detail": stderr})),
                )
            } else {
                (StatusCode::OK, Json(json!({"uri": uri})))
            }
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("signal-cli exec failed: {}", e)})),
        ),
        Err(_) => (
            StatusCode::REQUEST_TIMEOUT,
            Json(json!({"error": "signal-cli timed out after 30s"})),
        ),
    }
}

