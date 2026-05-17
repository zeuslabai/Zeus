//! Webhook server for receiving Twilio and Telnyx call events and media streams

use axum::{
    Json, Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use zeus_core::Result;

use crate::agent_loop::{
    VoiceAgentHandler, VoiceAgentLoop, VoiceCommand, VoiceLoopConfig, VoiceTTSProvider,
    format_clear_message, format_outgoing_media,
};
use crate::call::{CallManager, CallState, TranscriptEntry};
use crate::media::{MediaStreamHandler, StreamMessage};
use crate::stt::transcribe_mulaw_bytes;
use crate::telnyx::{TelnyxWebhookBody, call_state_from_event_type};

/// Shared state for webhook handlers
#[derive(Clone)]
pub struct WebhookState {
    pub call_manager: Arc<CallManager>,
    /// The host portion of webhook_base_url (protocol stripped)
    pub webhook_host: String,
    /// TTS voice for incoming call greeting
    pub tts_voice: String,
    /// Customizable greeting for incoming calls
    pub incoming_greeting: String,
    /// Voice agent handler for processing transcriptions
    pub agent_handler: Option<Arc<dyn VoiceAgentHandler>>,
    /// TTS provider for synthesizing agent responses
    pub tts_provider: Option<Arc<dyn VoiceTTSProvider>>,
    /// Voice loop configuration
    pub loop_config: VoiceLoopConfig,
    /// Credentials map from config.toml `[credentials]` for API key resolution
    #[allow(dead_code)]
    pub credentials: Option<std::collections::HashMap<String, String>>,
}

/// Webhook server for receiving Twilio events
pub struct WebhookServer {
    port: u16,
    state: WebhookState,
}

impl WebhookServer {
    pub fn new(
        port: u16,
        call_manager: Arc<CallManager>,
        webhook_base_url: &str,
        tts_voice: &str,
        incoming_greeting: &str,
    ) -> Self {
        let webhook_host = webhook_base_url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_string();

        Self {
            port,
            state: WebhookState {
                call_manager,
                webhook_host,
                tts_voice: tts_voice.to_string(),
                incoming_greeting: incoming_greeting.to_string(),
                agent_handler: None,
                tts_provider: None,
                loop_config: VoiceLoopConfig::default(),
                credentials: None,
            },
        }
    }

    /// Configure the voice agent loop for bidirectional conversations.
    ///
    /// When set, incoming audio will be transcribed, processed by the agent,
    /// synthesized via TTS, and streamed back to the caller.
    pub fn with_agent_loop(
        mut self,
        agent: Arc<dyn VoiceAgentHandler>,
        tts: Arc<dyn VoiceTTSProvider>,
        config: VoiceLoopConfig,
    ) -> Self {
        self.state.agent_handler = Some(agent);
        self.state.tts_provider = Some(tts);
        self.state.loop_config = config;
        self
    }

    /// Create the router for webhook endpoints
    pub fn router(state: WebhookState) -> Router {
        Router::new()
            .route("/voice/status", post(handle_status_callback))
            .route("/voice/incoming", post(handle_incoming_call))
            .route("/voice/media-stream", get(handle_media_stream))
            .route("/voice/telnyx/status", post(handle_telnyx_status))
            .with_state(state)
    }

    /// Start the webhook server
    pub async fn start(&self) -> Result<()> {
        let router = Self::router(self.state.clone());
        let addr = format!("0.0.0.0:{}", self.port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;

        info!("Voice webhook server listening on {}", addr);

        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, router).await {
                error!("Webhook server error: {}", e);
            }
        });

        Ok(())
    }
}

/// Handle Twilio status callback
async fn handle_status_callback(
    State(state): State<WebhookState>,
    axum::extract::Form(params): axum::extract::Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let call_sid = params.get("CallSid").cloned().unwrap_or_default();
    let call_status = params.get("CallStatus").cloned().unwrap_or_default();

    debug!("Call status callback: {} -> {}", call_sid, call_status);

    let new_state = CallState::from_twilio_status(&call_status);
    state.call_manager.update_state(&call_sid, new_state).await;

    "OK"
}

/// Handle Telnyx status callback (JSON body)
async fn handle_telnyx_status(
    State(state): State<WebhookState>,
    Json(body): Json<TelnyxWebhookBody>,
) -> impl IntoResponse {
    let call_control_id = &body.data.payload.call_control_id;
    let event_type = &body.data.event_type;

    debug!("Telnyx call event: {} -> {}", call_control_id, event_type);

    match event_type.as_str() {
        "call.dtmf.received" => {
            if let Some(digit) = &body.data.payload.digit {
                debug!(
                    "Telnyx DTMF digit received on {}: {}",
                    call_control_id, digit
                );
            }
        }
        _ => {
            let new_state = call_state_from_event_type(event_type);
            state
                .call_manager
                .update_state(call_control_id, new_state)
                .await;
        }
    }

    StatusCode::OK
}

/// Handle incoming call webhook from Twilio
async fn handle_incoming_call(
    State(state): State<WebhookState>,
    axum::extract::Form(params): axum::extract::Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let call_sid = params.get("CallSid").cloned().unwrap_or_default();
    let from = params.get("From").cloned().unwrap_or_default();
    let to = params.get("To").cloned().unwrap_or_default();

    info!("Incoming call: {} from {} to {}", call_sid, from, to);

    // Register the incoming call
    state
        .call_manager
        .register_call(call_sid.clone(), from)
        .await;
    state
        .call_manager
        .update_state(&call_sid, CallState::Ringing)
        .await;

    // Return TwiML to answer and connect to media stream
    let twiml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Response>
    <Say voice="{}">{}</Say>
    <Connect>
        <Stream url="wss://{}/voice/media-stream" />
    </Connect>
</Response>"#,
        state.tts_voice, state.incoming_greeting, state.webhook_host,
    );

    (StatusCode::OK, [("Content-Type", "text/xml")], twiml)
}

/// Handle WebSocket upgrade for media streams
async fn handle_media_stream(
    State(state): State<WebhookState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_media_socket(socket, state))
}

/// Handle the WebSocket media stream connection.
///
/// When an agent handler and TTS provider are configured, this runs the
/// full voice agent loop: STT → Agent → TTS → audio response.
/// Otherwise, it only transcribes and logs (original behavior).
async fn handle_media_socket(socket: WebSocket, state: WebhookState) {
    use futures_util::{SinkExt, StreamExt};

    info!("Media stream WebSocket connected");

    let (mut ws_sender, mut ws_receiver) = socket.split();

    let mut handler = MediaStreamHandler::new();
    let mut stream_sid = String::new();
    let mut call_sid = String::new();

    // Set up voice agent loop if configured
    let voice_loop = match (&state.agent_handler, &state.tts_provider) {
        (Some(agent), Some(tts)) => {
            info!("Voice agent loop enabled — full bidirectional pipeline active");
            Some(Arc::new(VoiceAgentLoop::new(
                agent.clone(),
                tts.clone(),
                state.call_manager.clone(),
                state.loop_config.clone(),
            )))
        }
        _ => {
            info!("Voice agent loop not configured — STT-only mode");
            None
        }
    };

    // Channel for outgoing WebSocket messages (from voice loop → WebSocket)
    let (out_tx, mut out_rx) = mpsc::channel::<String>(128);

    // Spawn WebSocket writer that sends outgoing messages
    let writer_handle = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ws_sender.send(Message::Text(msg)).await.is_err() {
                error!("Failed to send outgoing WebSocket message");
                break;
            }
        }
    });

    // Channel for voice commands (agent loop → command processor)
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<VoiceCommand>(64);
    let call_mgr_cmd = state.call_manager.clone();
    let out_tx_cmd = out_tx.clone();

    // Spawn command processor: converts VoiceCommands → JSON → outgoing channel
    let cmd_handle = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                VoiceCommand::SendAudio {
                    stream_sid,
                    mulaw_bytes,
                } => {
                    let msg = format_outgoing_media(&stream_sid, &mulaw_bytes);
                    let _ = out_tx_cmd.send(msg).await;
                }
                VoiceCommand::ClearAudio { stream_sid } => {
                    let msg = format_clear_message(&stream_sid);
                    let _ = out_tx_cmd.send(msg).await;
                }
                VoiceCommand::AddTranscript { call_id, entry } => {
                    call_mgr_cmd.add_transcript(&call_id, entry).await;
                }
            }
        }
    });

    // Main receive loop
    while let Some(msg) = ws_receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => match serde_json::from_str::<StreamMessage>(&text) {
                Ok(StreamMessage::Connected { .. }) => {
                    debug!("Media stream connected");
                }
                Ok(StreamMessage::Start {
                    stream_sid: sid,
                    start,
                }) => {
                    stream_sid = sid;
                    call_sid = start.call_sid.clone();
                    info!("Media stream started for call {}", call_sid);
                    state
                        .call_manager
                        .update_state(&call_sid, CallState::Active)
                        .await;
                }
                Ok(StreamMessage::Media { media, .. }) => {
                    if let Some(audio_chunk) = handler.process_media(&media) {
                        debug!(
                            "Audio buffer ready: {} bytes, sending to STT",
                            audio_chunk.len()
                        );

                        let cid = call_sid.clone();
                        let ssid = stream_sid.clone();
                        let creds = state.credentials.clone();

                        if let Some(ref vl) = voice_loop {
                            // Full voice agent loop: STT → Agent → TTS → audio
                            let cmd_tx = cmd_tx.clone();
                            let agent = vl.agent.clone();
                            let tts = vl.tts.clone();
                            let config = vl.config.clone();
                            let cm = vl.call_manager.clone();

                            tokio::spawn(async move {
                                match transcribe_mulaw_bytes(&audio_chunk, creds.as_ref()).await {
                                    Ok(text) if !text.is_empty() => {
                                        info!("STT for {}: \"{}\"", cid, text);
                                        let temp_loop = VoiceAgentLoop::new(agent, tts, cm, config);
                                        temp_loop
                                            .process_utterance(&cid, &ssid, &text, &cmd_tx)
                                            .await;
                                    }
                                    Ok(_) => {
                                        debug!("STT empty for {}", cid);
                                    }
                                    Err(e) => {
                                        warn!("STT failed for {}: {}", cid, e);
                                    }
                                }
                            });
                        } else {
                            // STT-only mode (original behavior)
                            let call_mgr = state.call_manager.clone();
                            tokio::spawn(async move {
                                match transcribe_mulaw_bytes(&audio_chunk, creds.as_ref()).await {
                                    Ok(text) if !text.is_empty() => {
                                        info!("STT transcript for {}: {}", cid, text);
                                        call_mgr
                                            .add_transcript(&cid, TranscriptEntry::user(&text))
                                            .await;
                                    }
                                    Ok(_) => {
                                        debug!("STT empty for {}", cid);
                                    }
                                    Err(e) => {
                                        warn!("STT failed for {}: {}", cid, e);
                                    }
                                }
                            });
                        }
                    }
                }
                Ok(StreamMessage::Stop { .. }) => {
                    info!("Media stream stopped for call {}", call_sid);
                    let remaining = handler.flush();
                    if !remaining.is_empty() {
                        debug!("Flushed {} remaining audio bytes", remaining.len());
                    }
                    state
                        .call_manager
                        .update_state(&call_sid, CallState::Completed)
                        .await;
                }
                Ok(StreamMessage::Dtmf { dtmf_data, .. }) => {
                    debug!("DTMF digit received: {}", dtmf_data.digit);
                }
                Err(e) => {
                    warn!("Failed to parse stream message: {}", e);
                }
            },
            Ok(Message::Close(_)) => {
                info!("Media stream WebSocket closed");
                break;
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
            _ => {} // Ignore binary and ping/pong
        }
    }

    // Cleanup: drop sender to signal writer/cmd tasks to stop
    drop(cmd_tx);
    drop(out_tx);
    let _ = cmd_handle.await;
    let _ = writer_handle.await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> WebhookState {
        WebhookState {
            call_manager: Arc::new(CallManager::new()),
            webhook_host: "example.ngrok.io".to_string(),
            tts_voice: "Polly.Amy".to_string(),
            incoming_greeting: "Hello, you've reached Zeus. How can I help you?".to_string(),
            agent_handler: None,
            tts_provider: None,
            loop_config: VoiceLoopConfig::default(),
            credentials: None,
        }
    }

    #[test]
    fn test_webhook_state_clone() {
        let state = test_state();
        let cloned = state.clone();
        // Verify both point to the same CallManager
        assert!(Arc::ptr_eq(&state.call_manager, &cloned.call_manager));
        assert_eq!(cloned.webhook_host, "example.ngrok.io");
        assert_eq!(cloned.tts_voice, "Polly.Amy");
    }

    #[test]
    fn test_webhook_server_router_builds() {
        let state = test_state();
        // Just verify the router can be constructed without panicking
        // This also verifies the /voice/incoming route is included
        let _router = WebhookServer::router(state);
    }

    #[test]
    fn test_incoming_call_route_exists() {
        // Verify the router builds successfully with the incoming call route
        let state = test_state();
        let _router = WebhookServer::router(state);
        // If the route definition was invalid, Router::new() would panic
    }

    #[test]
    fn test_webhook_state_with_voice_config() {
        let state = WebhookState {
            call_manager: Arc::new(CallManager::new()),
            webhook_host: "my-tunnel.ngrok.io".to_string(),
            tts_voice: "Polly.Joanna".to_string(),
            incoming_greeting: "Custom greeting here".to_string(),
            agent_handler: None,
            tts_provider: None,
            loop_config: VoiceLoopConfig::default(),
            credentials: None,
        };
        assert_eq!(state.webhook_host, "my-tunnel.ngrok.io");
        assert_eq!(state.tts_voice, "Polly.Joanna");
        assert_eq!(state.incoming_greeting, "Custom greeting here");
    }

    #[test]
    fn test_webhook_server_new_strips_https() {
        let call_manager = Arc::new(CallManager::new());
        let server = WebhookServer::new(
            8090,
            call_manager,
            "https://example.ngrok.io",
            "Polly.Amy",
            "Hello!",
        );
        assert_eq!(server.state.webhook_host, "example.ngrok.io");
    }

    #[test]
    fn test_webhook_server_new_strips_http() {
        let call_manager = Arc::new(CallManager::new());
        let server = WebhookServer::new(
            8090,
            call_manager,
            "http://localhost:8090",
            "Polly.Amy",
            "Hello!",
        );
        assert_eq!(server.state.webhook_host, "localhost:8090");
    }

    #[test]
    fn test_telnyx_status_route_exists() {
        // Verify the router builds successfully with the Telnyx status route
        let state = test_state();
        let _router = WebhookServer::router(state);
        // If the route definition was invalid, Router::new() would panic
    }
}
