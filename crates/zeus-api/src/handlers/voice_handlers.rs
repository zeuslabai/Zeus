//! Voice & Audio API handlers.
//!
//! Two surfaces:
//! - **STT** (#421): `POST /v1/stt` and OpenAI-compat `POST /v1/audio/transcriptions`
//!   — thin wrapper around `zeus_voice::inbound::transcribe_audio` (Whisper via
//!   Groq/OpenAI). No new infra.
//! - **Twilio outbound calling** (#422): exposes the existing
//!   `zeus_voice::twilio::TwilioProvider` (`VoiceCallProvider` —
//!   `initiate_call`/`hangup_call`/`play_tts`/`get_call_state`/`send_dtmf`) over
//!   REST. The provider is built per-request from `VoiceConfig::default()
//!   .with_env_overrides()` (env: `TWILIO_ACCOUNT_SID`/`TWILIO_AUTH_TOKEN`/
//!   `TWILIO_PHONE_NUMBER`), mirroring `receive_voice_inbound`.

use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, error, info, warn};

use crate::SharedState;

// ============================================================================
// STT (#421)
// ============================================================================

/// Response shape for the STT endpoints.
///
/// Mirrors the OpenAI `POST /v1/audio/transcriptions` minimal contract so
/// clients built against either surface get the same payload.
#[derive(Debug, Serialize)]
pub struct TranscriptionResponse {
    pub text: String,
}

/// `POST /v1/stt`
///
/// Multipart upload (`file` field) of an audio file (wav/mp3/ogg/etc.).
/// Returns `{ "text": "..." }`.
pub async fn transcribe_audio_endpoint(
    State(_state): State<SharedState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<TranscriptionResponse>), (StatusCode, String)> {
    run_transcription(&mut multipart).await
}

/// OpenAI-compatible alias: `POST /v1/audio/transcriptions`.
pub async fn openai_transcriptions(
    State(_state): State<SharedState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<TranscriptionResponse>), (StatusCode, String)> {
    run_transcription(&mut multipart).await
}

/// Core transcription routine shared by both routes.
async fn run_transcription(
    multipart: &mut Multipart,
) -> Result<(StatusCode, Json<TranscriptionResponse>), (StatusCode, String)> {
    let field = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid multipart: {e}")))?
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Missing audio file in multipart body".to_string(),
            )
        })?;

    let filename = field.file_name().unwrap_or("audio.wav").to_string();
    let declared_mime = field
        .content_type()
        .map(|c| c.to_string())
        .unwrap_or_default();

    let bytes = field
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Failed to read body: {e}")))?;

    if bytes.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Empty audio upload".to_string()));
    }

    let mime = resolve_mime(&filename, &declared_mime);

    debug!(
        filename = %filename,
        mime = %mime,
        bytes = bytes.len(),
        "STT endpoint: transcribing upload"
    );

    match zeus_voice::inbound::transcribe_audio(&bytes, &mime, &filename).await {
        Ok(text) => Ok((StatusCode::OK, Json(TranscriptionResponse { text }))),
        Err(e) => {
            error!(error = %e, "Transcription failed");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Transcription failed: {e}"),
            ))
        }
    }
}

/// Pick a MIME for the Whisper upload.
///
/// Order: explicit declared audio content-type > guessed from extension >
/// declared non-audio content-type > `audio/wav`.
fn resolve_mime(filename: &str, declared: &str) -> String {
    if !declared.is_empty() && declared.starts_with("audio/") {
        return declared.to_string();
    }
    let lower = filename.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "wav" => "audio/wav".to_string(),
        "mp3" => "audio/mpeg".to_string(),
        "ogg" | "oga" => "audio/ogg".to_string(),
        "flac" => "audio/flac".to_string(),
        "webm" => "audio/webm".to_string(),
        "m4a" | "mp4" => "audio/mp4".to_string(),
        _ => {
            if declared.is_empty() {
                "audio/wav".to_string()
            } else {
                declared.to_string()
            }
        }
    }
}

// ============================================================================
// Twilio outbound calling (#422) — request bodies
// ============================================================================

/// Body for `POST /v1/voice/call`.
#[derive(Debug, Deserialize)]
pub struct CreateCallRequest {
    /// E.164 destination phone number.
    pub to: String,
    /// Greeting text spoken when the call connects.
    #[serde(default)]
    pub greeting: Option<String>,
}

/// Body for `POST /v1/voice/call/:id/say`.
#[derive(Debug, Deserialize)]
pub struct SayRequest {
    pub text: String,
}

/// Body for `POST /v1/voice/call/:id/dtmf`.
#[derive(Debug, Deserialize)]
pub struct DtmfRequest {
    pub digits: String,
}

// ============================================================================
// Twilio outbound calling — handlers
// ============================================================================

/// `POST /v1/voice/call` — initiate an outbound Twilio call.
///
/// Returns `{ "call_id": "<sid>" }`.
pub async fn create_call(
    State(_state): State<SharedState>,
    Json(req): Json<CreateCallRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    if req.to.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Field `to` is required".to_string()));
    }

    let provider = build_twilio_provider()?;
    let greeting = req.greeting.as_deref().unwrap_or("");

    info!(to = %req.to, "Initiating outbound voice call");

    match provider.initiate_call(&req.to, greeting).await {
        Ok(call_id) => Ok((StatusCode::OK, Json(json!({ "call_id": call_id })))),
        Err(e) => {
            error!(error = %e, "Failed to initiate call");
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Call failed: {e}")))
        }
    }
}

/// `POST /v1/voice/call/:id/hangup`
pub async fn hangup_call(
    State(_state): State<SharedState>,
    Path(call_id): Path<String>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let provider = build_twilio_provider()?;
    match provider.hangup_call(&call_id).await {
        Ok(()) => Ok((StatusCode::OK, Json(json!({ "ok": true })))),
        Err(e) => {
            error!(error = %e, call_id = %call_id, "hangup failed");
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Hangup failed: {e}")))
        }
    }
}

/// `POST /v1/voice/call/:id/say` — play TTS on an active call.
pub async fn say_on_call(
    State(_state): State<SharedState>,
    Path(call_id): Path<String>,
    Json(req): Json<SayRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    if req.text.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Field `text` is required".to_string()));
    }
    let provider = build_twilio_provider()?;
    match provider.play_tts(&call_id, &req.text).await {
        Ok(()) => Ok((StatusCode::OK, Json(json!({ "ok": true })))),
        Err(e) => {
            error!(error = %e, call_id = %call_id, "say failed");
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Say failed: {e}")))
        }
    }
}

/// `GET /v1/voice/call/:id` — current call state.
pub async fn get_call(
    State(_state): State<SharedState>,
    Path(call_id): Path<String>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let provider = build_twilio_provider()?;
    match provider.get_call_state(&call_id).await {
        Ok(state) => {
            // CallState derives Serialize; fall back to Debug if not.
            let payload = serde_json::to_value(&state).unwrap_or_else(|_| {
                json!({ "call_id": call_id, "state": format!("{:?}", state) })
            });
            Ok((StatusCode::OK, Json(payload)))
        }
        Err(e) => {
            error!(error = %e, call_id = %call_id, "get_call_state failed");
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("State fetch failed: {e}")))
        }
    }
}

/// `POST /v1/voice/call/:id/dtmf` — send DTMF tones on an active call.
pub async fn send_dtmf(
    State(_state): State<SharedState>,
    Path(call_id): Path<String>,
    Json(req): Json<DtmfRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    if req.digits.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Field `digits` is required".to_string()));
    }
    let provider = build_twilio_provider()?;
    match provider.send_dtmf(&call_id, &req.digits).await {
        Ok(()) => Ok((StatusCode::OK, Json(json!({ "ok": true })))),
        Err(e) => {
            error!(error = %e, call_id = %call_id, "dtmf failed");
            Err((StatusCode::INTERNAL_SERVER_ERROR, format!("DTMF failed: {e}")))
        }
    }
}

// ============================================================================
// Provider factory
// ============================================================================

/// Build a `TwilioProvider` from env-configured `VoiceConfig`.
///
/// Returns a 503 tuple when Twilio credentials are absent so callers get a
/// clear "not configured" signal rather than an opaque 500.
pub(crate) fn build_twilio_provider(
) -> Result<Box<dyn zeus_voice::provider::VoiceCallProvider>, (StatusCode, String)> {
    let cfg = zeus_voice::VoiceConfig::default().with_env_overrides();
    if cfg.account_sid.trim().is_empty() || cfg.auth_token.trim().is_empty() {
        warn!("Twilio voice call attempted without TWILIO_ACCOUNT_SID/AUTH_TOKEN");
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Twilio voice not configured (set TWILIO_ACCOUNT_SID and TWILIO_AUTH_TOKEN)"
                .to_string(),
        ));
    }
    Ok(Box::new(zeus_voice::twilio::TwilioProvider::new(cfg)))
}
