//! Inbound Webhook Handler
//!
//! Provides HMAC-SHA256 signature verification, JSON payload parsing,
//! agent routing via the agent registry, and Athena action logging.

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};
use zeus_athena::{ActionLog, ActionType, Athena, AthenaConfig as AthenaWriterConfig};

use super::{WebhookPayload, WebhookResponse};
use crate::SharedState;
use crate::webhook_triggers::{TriggerEngine, WebhookEvent};

// ============================================================================
// Signature Verification
// ============================================================================

/// Verify HMAC-SHA256 webhook signature.
///
/// Expects `signature_header` in GitHub-style format: `"sha256=<hex>"`.
/// Performs constant-time comparison to prevent timing attacks.
fn verify_signature(body: &[u8], signature_header: &str, secret: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let Ok(mut mac) = <Hmac<Sha256>>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let expected = mac.finalize().into_bytes();
    let expected_hex: String = expected.iter().map(|b| format!("{:02x}", b)).collect();

    let Some(provided) = signature_header.strip_prefix("sha256=") else {
        return false;
    };

    // Constant-time byte comparison to prevent timing attacks
    if provided.len() != expected_hex.len() {
        return false;
    }
    provided
        .as_bytes()
        .iter()
        .zip(expected_hex.as_bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

// ============================================================================
// Handlers
// ============================================================================

/// Default maximum webhook payload size (256 KB).
/// Overridable via `[gateway].max_webhook_payload_bytes` in config.toml.
const DEFAULT_MAX_WEBHOOK_PAYLOAD_SIZE: usize = zeus_core::MAX_WEBHOOK_PAYLOAD_BYTES;

/// Default maximum message length within a webhook payload (50 KB).
/// Overridable via `[gateway].max_webhook_message_bytes` in config.toml.
const DEFAULT_MAX_WEBHOOK_MESSAGE_LEN: usize = zeus_core::MAX_WEBHOOK_MESSAGE_BYTES;

/// POST /v1/webhooks
///
/// Receives an inbound webhook message:
/// 1. Enforces payload size limits.
/// 2. Verifies HMAC-SHA256 signature if `ZEUS_WEBHOOK_SECRET` env var is set.
/// 3. Parses JSON body into `WebhookPayload`.
/// 4. Routes message to the first available spawned agent via `agent.run()`.
/// 5. Logs the event to Athena (falls back to workspace notes if unconfigured).
pub async fn receive_webhook(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<WebhookResponse>, (StatusCode, String)> {
    // --- 0. Payload size limit (configurable via [gateway]) ---
    let max_payload = {
        let s = state.read().await;
        s.config
            .gateway
            .as_ref()
            .map(|g| g.max_webhook_payload_bytes)
            .unwrap_or(DEFAULT_MAX_WEBHOOK_PAYLOAD_SIZE)
    };
    if body.len() > max_payload {
        warn!("Webhook payload too large: {} bytes", body.len());
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "Payload too large ({} bytes, max {})",
                body.len(),
                max_payload
            ),
        ));
    }

    // --- 1. Signature verification ---
    if let Ok(secret) = std::env::var("ZEUS_WEBHOOK_SECRET") {
        let sig_header = headers
            .get("x-hub-signature-256")
            .or_else(|| headers.get("x-webhook-signature"))
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if !verify_signature(&body, sig_header, &secret) {
            warn!("Webhook signature verification failed");
            return Err((
                StatusCode::UNAUTHORIZED,
                "Invalid or missing webhook signature".to_string(),
            ));
        }
        info!("Webhook signature verified");
    }

    // --- 2. Parse JSON payload ---
    let payload: WebhookPayload = serde_json::from_slice(&body).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid JSON payload: {}", e),
        )
    })?;

    // --- 2b. Message length limit (configurable via [gateway]) ---
    let max_msg_len = {
        let s = state.read().await;
        s.config
            .gateway
            .as_ref()
            .map(|g| g.max_webhook_message_bytes)
            .unwrap_or(DEFAULT_MAX_WEBHOOK_MESSAGE_LEN)
    };
    if payload.message.len() > max_msg_len {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Message too long ({} chars, max {})",
                payload.message.len(),
                max_msg_len
            ),
        ));
    }

    let source = payload.source.as_deref().unwrap_or("unknown");
    let sender = payload.sender.as_deref().unwrap_or("anonymous");
    let channel = payload.channel.as_deref().unwrap_or("default");

    info!(
        "Webhook received from {}/{}: {} chars from {}",
        source,
        channel,
        payload.message.len(),
        sender
    );

    let id = uuid::Uuid::new_v4().to_string();

    // --- 3. Evaluate trigger-action pipelines ---
    let event = WebhookEvent::new(
        payload.source.clone(),
        payload.message.clone(),
        payload.sender.clone(),
        payload.channel.clone(),
        payload.metadata.clone(),
    );
    {
        let st = state.read().await;
        let matched = st.trigger_engine.evaluate(&event).await;
        for trigger in &matched {
            let result = TriggerEngine::dispatch_action(trigger, &event);
            if result.success {
                info!(
                    "Trigger '{}' fired ({}): {:?}",
                    result.trigger_name, result.action_type, result.output
                );
            }
            st.trigger_engine.record_fire(&trigger.id).await;
        }
    }

    // --- 4. Route to agent (default fallback) ---
    let agent_response = route_to_agent(&state, source, channel, sender, &payload.message).await;

    // --- 5. Log to Athena ---
    let processed = log_to_athena(
        &state,
        source,
        channel,
        sender,
        &payload.message,
        agent_response.as_deref(),
    )
    .await;

    Ok(Json(WebhookResponse {
        received: true,
        id,
        processed,
    }))
}

/// POST /v1/webhooks/:source
///
/// Convenience variant that injects the URL path segment as the `source` field.
pub async fn receive_webhook_source(
    State(state): State<SharedState>,
    Path(source): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<WebhookResponse>, (StatusCode, String)> {
    let mut value: Value = serde_json::from_slice(&body)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert("source".to_string(), json!(source));
    }
    let patched = Bytes::from(
        serde_json::to_vec(&value)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
    );
    receive_webhook(State(state), headers, patched).await
}

/// GET /v1/webhooks
pub async fn webhook_health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "endpoint": "webhook",
        "accepts": ["POST"],
        "content_type": "application/json",
        "signature_verification": std::env::var("ZEUS_WEBHOOK_SECRET").is_ok(),
    }))
}

// ============================================================================
// Helpers
// ============================================================================

/// Route the webhook message to the first available spawned agent.
async fn route_to_agent(
    state: &SharedState,
    source: &str,
    channel: &str,
    sender: &str,
    message: &str,
) -> Option<String> {
    let (agent_id, agent_arc): (String, Arc<RwLock<zeus_agent::Agent>>) = {
        let mut st = state.write().await;
        let instance = st.agent_registry.list().into_iter().next()?;
        let id = instance.agent_id.clone();
        st.agent_registry.update_activity(&id);
        let arc = st.agent_registry.get(&id)?.agent.clone();
        (id, arc)
    };

    let prompt = format!("[Webhook from {source}/{channel}] {sender}: {message}");
    let mut agent = agent_arc.write().await;

    match agent.run(&prompt).await {
        Ok(response) => {
            info!(
                "Agent '{}' processed webhook: {} chars",
                agent_id,
                response.len()
            );
            Some(response)
        }
        Err(e) => {
            warn!("Agent '{}' failed to process webhook: {}", agent_id, e);
            None
        }
    }
}

/// Log the webhook event to Athena if configured, falling back to workspace notes.
async fn log_to_athena(
    state: &SharedState,
    source: &str,
    channel: &str,
    sender: &str,
    message: &str,
    agent_response: Option<&str>,
) -> bool {
    let st = state.read().await;

    let description = format!(
        "Webhook {source}/{channel} from {sender}: {}{}",
        if message.len() > 200 {
            format!("{}...", zeus_core::truncate_str(message, 200))
        } else {
            message.to_string()
        },
        agent_response
            .map(|r| format!(" → {}", zeus_core::truncate_str(r, 120)))
            .unwrap_or_default()
    );

    if let Some(core_cfg) = &st.config.athena {
        // Convert zeus_core::AthenaConfig → zeus_athena::AthenaConfig
        let athena_cfg = AthenaWriterConfig::new(core_cfg.vault_path.clone());
        match Athena::new(athena_cfg) {
            Ok(athena) => {
                let log =
                    ActionLog::new(ActionType::MessageReceived, description).with_tool("webhook");
                match athena.log_action(&log).await {
                    Ok(_) => return true,
                    Err(e) => warn!("Athena log_action failed: {}", e),
                }
            }
            Err(e) => warn!("Failed to init Athena: {}", e),
        }
    }

    // Fallback: workspace daily note
    let note = format!("[Webhook] {source}/{channel}: {sender}: {message}");
    st.workspace.note(&note).await.is_ok()
}

// ============================================================================
// Webhook Trigger CRUD Handlers
// ============================================================================

/// GET /v1/webhooks/triggers — list all triggers
pub async fn list_triggers(State(state): State<SharedState>) -> Json<Value> {
    let st = state.read().await;
    let triggers = st.trigger_engine.list().await;
    Json(json!({ "triggers": triggers, "count": triggers.len() }))
}

/// Request body for creating a trigger
#[derive(Debug, serde::Deserialize)]
pub struct CreateTriggerRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub condition: crate::webhook_triggers::TriggerCondition,
    pub action: crate::webhook_triggers::TriggerAction,
    #[serde(default)]
    pub priority: Option<u32>,
}

/// POST /v1/webhooks/triggers — create a new trigger
pub async fn create_trigger(
    State(state): State<SharedState>,
    Json(req): Json<CreateTriggerRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let st = state.read().await;
    let trigger = st
        .trigger_engine
        .create(
            req.name,
            req.description,
            req.condition,
            req.action,
            req.priority,
        )
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(json!({ "trigger": trigger })))
}

/// DELETE /v1/webhooks/triggers/:id — delete a trigger
pub async fn delete_trigger(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let st = state.read().await;
    st.trigger_engine
        .delete(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(json!({ "deleted": true, "id": id })))
}

/// PUT /v1/webhooks/triggers/:id/enable — enable a trigger
pub async fn enable_trigger(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let st = state.read().await;
    st.trigger_engine
        .set_enabled(&id, true)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(json!({ "enabled": true, "id": id })))
}

/// PUT /v1/webhooks/triggers/:id/disable — disable a trigger
pub async fn disable_trigger(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let st = state.read().await;
    st.trigger_engine
        .set_enabled(&id, false)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;
    Ok(Json(json!({ "enabled": false, "id": id })))
}

// ============================================================================
// Twilio WhatsApp Webhook Handler
// ============================================================================

/// POST /v1/webhooks/whatsapp
///
/// Receives inbound WhatsApp messages from Twilio.
/// Twilio sends form-urlencoded data with the `whatsapp:` prefix on From/To.
///
/// This endpoint:
/// 1. Parses the Twilio form-urlencoded webhook payload
/// 2. Routes the message through the channel manager
/// 3. Returns TwiML empty response (Twilio expects XML or empty 200)
pub async fn receive_whatsapp_webhook(
    State(state): State<SharedState>,
    body: axum::body::Bytes,
) -> Result<(StatusCode, String), (StatusCode, String)> {
    use zeus_channels::twilio_whatsapp::TwilioWhatsAppWebhook;

    if body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Empty webhook payload".to_string()));
    }

    // Parse the Twilio form-urlencoded payload
    let webhook: TwilioWhatsAppWebhook = serde_urlencoded::from_bytes(&body).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid webhook payload: {e}"),
        )
    })?;

    // Skip status callbacks (no body, has MessageStatus)
    if webhook.message_status.is_some() && webhook.body.is_empty() {
        info!(
            sid = %webhook.message_sid,
            status = ?webhook.message_status,
            "WhatsApp status callback received"
        );
        return Ok((
            StatusCode::OK,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Response/>".to_string(),
        ));
    }

    let sender = webhook.sender_phone().to_string();
    let profile = webhook.profile_name.as_deref().unwrap_or("unknown");

    info!(
        from = %sender,
        profile = %profile,
        sid = %webhook.message_sid,
        body_len = webhook.body.len(),
        media = %webhook.num_media,
        "Inbound WhatsApp message via Twilio"
    );

    // Build a ChannelMessage and inject into the channel manager
    {
        let st = state.read().await;
        let cm = &st.channel_manager;
        let tx = cm.inbound_tx();

        let source = zeus_channels::ChannelSource::with_chat(
            "twilio_whatsapp",
            &sender,
            webhook.recipient_phone(),
        );
        let mut msg = zeus_channels::ChannelMessage::new(source, webhook.body.clone());
        msg.id = webhook.message_sid.clone();

        if let Err(e) = tx.send(msg).await {
            warn!("Failed to inject WhatsApp message: {e}");
        }
    }

    // Return empty TwiML response (Twilio expects this)
    Ok((
        StatusCode::OK,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Response/>".to_string(),
    ))
}

/// GET /v1/webhooks/whatsapp
///
/// Health check for WhatsApp webhook endpoint.
pub async fn whatsapp_webhook_health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "endpoint": "whatsapp",
        "provider": "twilio",
        "accepts": ["POST"],
        "content_type": "application/x-www-form-urlencoded",
    }))
}

// ============================================================================
// Twilio Voice Inbound Webhook Handlers
// ============================================================================

/// POST /v1/voice/inbound
///
/// Receives inbound phone calls from Twilio.
/// Returns TwiML to greet the caller and connect to a media stream.
pub async fn receive_voice_inbound(
    State(_state): State<SharedState>,
    body: axum::body::Bytes,
) -> Result<
    (
        StatusCode,
        [(axum::http::header::HeaderName, &'static str); 1],
        String,
    ),
    (StatusCode, String),
> {
    use zeus_voice::inbound::{InboundCallWebhook, TwilioVoiceConfig, inbound_call_twiml};

    if body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Empty webhook payload".to_string()));
    }

    let webhook: InboundCallWebhook = serde_urlencoded::from_bytes(&body).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid inbound call payload: {e}"),
        )
    })?;

    info!(
        call_sid = %webhook.call_sid,
        from = %webhook.from,
        to = %webhook.to,
        direction = %webhook.direction,
        "Inbound voice call via Twilio"
    );

    // Build TwilioVoiceConfig from environment variables
    let voice_config = TwilioVoiceConfig::from_env();

    let twiml = inbound_call_twiml(&voice_config);

    Ok((
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/xml")],
        twiml,
    ))
}

/// POST /v1/voice/recording-status
///
/// Receives recording status callbacks from Twilio.
/// When a recording is completed, downloads it and runs Whisper transcription.
pub async fn receive_recording_status(
    State(_state): State<SharedState>,
    body: axum::body::Bytes,
) -> Result<(StatusCode, String), (StatusCode, String)> {
    use zeus_voice::inbound::{
        RecordingStatusWebhook, TwilioVoiceConfig, recording_transcription_pipeline,
    };

    if body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Empty payload".to_string()));
    }

    let webhook: RecordingStatusWebhook = serde_urlencoded::from_bytes(&body).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid recording payload: {e}"),
        )
    })?;

    info!(
        recording_sid = %webhook.recording_sid,
        call_sid = %webhook.call_sid,
        status = %webhook.recording_status,
        duration = %webhook.recording_duration,
        "Recording status callback"
    );

    // Only process completed recordings
    if webhook.recording_status != "completed" {
        return Ok((StatusCode::OK, "OK".to_string()));
    }

    // Build config from environment variables
    let voice_config = TwilioVoiceConfig::from_env();

    let call_sid = webhook.call_sid.clone();
    let recording_sid = webhook.recording_sid.clone();
    let duration = webhook.recording_duration.clone();

    // Run pipeline in background — don't block the webhook response
    tokio::spawn(async move {
        match recording_transcription_pipeline(
            &voice_config,
            &call_sid,
            &recording_sid,
            Some(&duration),
        )
        .await
        {
            Ok(result) => {
                info!(
                    "Transcription complete for {}: {} chars",
                    recording_sid,
                    result.text.len()
                );
            }
            Err(e) => {
                warn!("Transcription pipeline failed for {}: {}", recording_sid, e);
            }
        }
    });

    Ok((StatusCode::OK, "OK".to_string()))
}

/// GET /v1/voice/inbound
///
/// Health check for voice inbound webhook.
pub async fn voice_inbound_health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "endpoint": "voice_inbound",
        "provider": "twilio",
        "accepts": ["POST"],
        "content_type": "application/x-www-form-urlencoded",
        "features": ["inbound_calls", "recording", "transcription"],
    }))
}
