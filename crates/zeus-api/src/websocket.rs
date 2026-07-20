//! WebSocket endpoint for real-time streaming chat.
//!
//! Provides a WebSocket upgrade at `GET /v1/ws` that supports:
//! - Streaming chat responses (text chunks, tool calls, tool results)
//! - Studio orchestration commands (team creation, delegation, etc.)
//! - Prometheus plan execution watching (live DAG step progress)
//! - Ping/pong keepalive
//!
//! ## Protocol
//!
//! Client → Server (JSON):
//! ```json
//! {"type": "chat", "message": "hello", "session_id": "optional"}
//! {"type": "cancel_stream"}
//! {"type": "studio", "message": "create team alpha", "session_id": "optional", "team_id": "optional"}
//! {"type": "prometheus_watch", "plan_id": "plan-abc123"}
//! {"type": "reaction", "message_id": "msg-123", "emoji": "thumbsup"}
//! {"type": "pin", "message_id": "msg-123", "pinned": true}
//! {"type": "edit", "message_id": "msg-123", "content": "updated text"}
//! {"type": "forward", "message_id": "msg-123", "target_type": "session", "target_id": "sess-456"}
//! {"type": "delete", "message_id": "msg-123"}
//! {"type": "read_receipt", "message_id": "msg-123"}
//! {"type": "ping"}
//! ```
//!
//! Server → Client (JSON):
//! ```json
//! {"type": "started"}
//! {"type": "text_chunk", "chunk": "Hello"}
//! {"type": "tool_call", "name": "shell", "args": {...}}
//! {"type": "tool_result", "name": "shell", "success": true, "output": "..."}
//! {"type": "response_complete", "content": "full response"}
//! {"type": "finished", "iterations": 3}
//! {"type": "error", "message": "..."}
//! {"type": "pong"}
//! {"type": "stream_cancelled"}
//! {"type": "studio_event", "event_type": "team_created", "data": {...}}
//! {"type": "prometheus_update", "plan_id": "...", "step_id": 0, "status": "completed", "progress_pct": 50.0, "output": "..."}
//! {"type": "prometheus_complete", "plan_id": "...", "status": "completed", "steps_completed": 3, "steps_failed": 0, "duration_ms": 1234}
//! {"type": "message_ack", "action": "reaction", "message_id": "msg-123", "success": true}
//! ```

use axum::{
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, broadcast, mpsc};
use tracing::{debug, error, info, warn};

use zeus_agent::AgentEvent;
use zeus_llm::LlmClient;
use zeus_session::Session;

use crate::SharedState;

// ============================================================================
// Prometheus plan execution broadcast
// ============================================================================

/// A step-level update emitted during plan execution, broadcast to watching clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStepUpdate {
    pub plan_id: String,
    pub step_id: usize,
    pub status: String,
    pub progress_pct: f64,
    pub output: String,
}

/// A plan-level completion event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanComplete {
    pub plan_id: String,
    pub status: String,
    pub steps_completed: usize,
    pub steps_failed: usize,
    pub duration_ms: u64,
}

/// Events sent through the plan update broadcast channel.
#[derive(Debug, Clone)]
pub enum PlanEvent {
    StepUpdate(PlanStepUpdate),
    Complete(PlanComplete),
}

/// Shared broadcaster for Prometheus plan execution events.
/// Stored in AppState; handlers call `send()`, WebSocket clients `subscribe()`.
#[derive(Clone)]
pub struct PlanBroadcast {
    tx: broadcast::Sender<PlanEvent>,
}

impl PlanBroadcast {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Emit a step update (non-blocking, drops if no receivers)
    pub fn send_step_update(&self, update: PlanStepUpdate) {
        let _ = self.tx.send(PlanEvent::StepUpdate(update));
    }

    /// Emit a plan completion event
    pub fn send_complete(&self, complete: PlanComplete) {
        let _ = self.tx.send(PlanEvent::Complete(complete));
    }

    /// Subscribe to plan events
    pub fn subscribe(&self) -> broadcast::Receiver<PlanEvent> {
        self.tx.subscribe()
    }
}

/// Events sent through the orchestration broadcast channel.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OrchestrationEvent {
    PhaseChanged {
        session_id: String,
        phase: String,
        data: Value,
    },
    StepStarted {
        session_id: String,
        step_index: usize,
        step_description: String,
        steps_total: usize,
    },
    StepCompleted {
        session_id: String,
        step_index: usize,
        output: String,
        progress_pct: f64,
    },
    StepFailed {
        session_id: String,
        step_index: usize,
        error: String,
    },
    AgentAssigned {
        session_id: String,
        agent_role: String,
        model_tier: String,
    },
    ArtifactCreated {
        session_id: String,
        artifact_name: String,
        artifact_path: String,
    },
    Complete {
        session_id: String,
        status: String,
        summary: String,
        artifact_path: String,
        duration_ms: u64,
    },
    SpawnStarted {
        session_id: String,
        agent_id: String,
        role: String,
        task: String,
        depth: u8,
    },
    SpawnCompleted {
        session_id: String,
        agent_id: String,
        role: String,
        success: bool,
        output: String,
        duration_ms: u64,
    },
    SpawnFailed {
        session_id: String,
        agent_id: String,
        role: String,
        error: String,
    },
}

/// Shared broadcaster for orchestration events.
/// Stored in AppState; orchestration handlers emit events, WebSocket clients subscribe.
#[derive(Clone)]
pub struct OrchestrationBroadcast {
    tx: broadcast::Sender<OrchestrationEvent>,
}

impl OrchestrationBroadcast {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Emit an orchestration event (non-blocking)
    pub fn send(&self, event: OrchestrationEvent) {
        let _ = self.tx.send(event);
    }

    /// Subscribe to orchestration events
    pub fn subscribe(&self) -> broadcast::Receiver<OrchestrationEvent> {
        self.tx.subscribe()
    }
}

// ============================================================================
// Studio → War Room broadcast
// ============================================================================

/// Events emitted during Studio puppet sessions, relayed to linked War Rooms.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum StudioEvent {
    /// Studio session status changed (planning, driving, paused, complete, failed)
    StatusChanged {
        session_id: String,
        status: String,
        goal: String,
    },
    /// A puppet UI action was dispatched
    ActionDispatched {
        session_id: String,
        action_num: u32,
        action_type: String,
        description: String,
    },
    /// A puppet UI action completed
    ActionCompleted {
        session_id: String,
        action_num: u32,
        success: bool,
        elapsed_ms: u64,
    },
    /// Studio session completed
    SessionComplete {
        session_id: String,
        total_actions: u32,
        completed_actions: u32,
        failed_actions: u32,
        summary: String,
    },
    /// Studio session failed
    SessionFailed { session_id: String, error: String },
}

/// Shared broadcaster for Studio puppet session events.
/// Stored in AppState; studio handlers emit events, War Room clients subscribe.
#[derive(Clone)]
pub struct StudioBroadcast {
    tx: broadcast::Sender<StudioEvent>,
}

impl StudioBroadcast {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Emit a studio event (non-blocking, drops if no receivers)
    pub fn send(&self, event: StudioEvent) {
        let _ = self.tx.send(event);
    }

    /// Subscribe to studio events
    pub fn subscribe(&self) -> broadcast::Receiver<StudioEvent> {
        self.tx.subscribe()
    }
}

// ============================================================================
// Protocol types
// ============================================================================

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
enum ClientMessage {
    Chat {
        message: String,
        #[serde(default)]
        session_id: Option<String>,
    },
    Studio {
        message: String,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        team_id: Option<String>,
    },
    PrometheusWatch {
        plan_id: String,
    },
    OrchestrationWatch {
        session_id: String,
    },
    Workflow {
        message: String,
        #[serde(default)]
        #[allow(dead_code)]
        session_id: Option<String>,
        #[serde(default)]
        mode: Option<String>,
    },
    Reaction {
        message_id: String,
        emoji: String,
    },
    Pin {
        message_id: String,
        pinned: bool,
    },
    Edit {
        message_id: String,
        content: String,
    },
    Forward {
        message_id: String,
        target_type: String,
        target_id: String,
    },
    Delete {
        message_id: String,
    },
    ReadReceipt {
        message_id: String,
    },
    AuthResponse {
        nonce: String,
        timestamp: u64,
        signature: String,
    },
    Ping,
    /// Cancel an in-flight streaming chat response
    CancelStream,
    // Pantheon — multi-agent collaboration
    PantheonMission {
        goal: String,
        #[serde(default)]
        constraints: Option<serde_json::Value>,
    },
    PantheonIntervene {
        mission_id: String,
        action: String, // "pause" | "resume" | "cancel" | "redirect"
        #[serde(default)]
        message: Option<String>,
    },
    PantheonApprove {
        mission_id: String,
        task_id: String,
    },
    PantheonReject {
        mission_id: String,
        task_id: String,
        #[serde(default)]
        reason: Option<String>,
    },
    PantheonWatch {
        mission_id: String,
    },
    /// Subscribe to a Studio puppet session's live events
    StudioWatch {
        session_id: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
enum ServerMessage {
    Started,
    TextChunk {
        chunk: String,
    },
    ToolCall {
        name: String,
        args: Value,
    },
    ToolResult {
        name: String,
        success: bool,
        output: String,
    },
    ResponseComplete {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
    Finished {
        iterations: u32,
    },
    Error {
        message: String,
    },
    Compacted {
        messages_removed: usize,
        tokens_before: usize,
        tokens_after: usize,
    },
    Pong,
    StreamCancelled,
    ApprovalPending {
        id: String,
        tool_name: String,
        args: Value,
        agent_id: Option<String>,
    },
    ApprovalResolved {
        id: String,
        status: Value,
    },
    StudioEvent {
        event_type: String,
        data: Value,
    },
    PrometheusUpdate {
        plan_id: String,
        step_id: usize,
        status: String,
        progress_pct: f64,
        output: String,
    },
    PrometheusComplete {
        plan_id: String,
        status: String,
        steps_completed: usize,
        steps_failed: usize,
        duration_ms: u64,
    },
    WorkflowCreated {
        workflow_id: String,
        goal: String,
        steps: Vec<Value>,
        parallel_groups: Vec<Vec<usize>>,
        critical_path: Vec<usize>,
        estimated_total_ms: u64,
        mode: String,
    },
    MessageAck {
        action: String,
        message_id: String,
        success: bool,
    },
    // Orchestration engine events
    OrchestrationWatching {
        session_id: String,
    },
    OrchestrationPhaseChanged {
        session_id: String,
        phase: String,
        data: Value,
    },
    OrchestrationStepStarted {
        session_id: String,
        step_index: usize,
        step_description: String,
        steps_total: usize,
    },
    OrchestrationStepCompleted {
        session_id: String,
        step_index: usize,
        output: String,
        progress_pct: f64,
    },
    OrchestrationStepFailed {
        session_id: String,
        step_index: usize,
        error: String,
    },
    OrchestrationAgentAssigned {
        session_id: String,
        agent_role: String,
        model_tier: String,
    },
    OrchestrationArtifactCreated {
        session_id: String,
        artifact_name: String,
        artifact_path: String,
    },
    OrchestrationComplete {
        session_id: String,
        status: String,
        summary: String,
        artifact_path: String,
        duration_ms: u64,
    },
    SpawnStarted {
        session_id: String,
        agent_id: String,
        role: String,
        task: String,
        depth: u8,
    },
    SpawnCompleted {
        session_id: String,
        agent_id: String,
        role: String,
        success: bool,
        output: String,
        duration_ms: u64,
    },
    SpawnFailed {
        session_id: String,
        agent_id: String,
        role: String,
        error: String,
    },
    // WebSocket v3 auth handshake messages
    AuthChallenge {
        nonce: String,
        server_timestamp: u64,
        version: String,
    },
    AuthOk {
        version: String,
        public_key_hex: String,
    },
    AuthFailed {
        reason: String,
    },
    // Pantheon — multi-agent collaboration events
    PantheonMissionCreated {
        mission_id: String,
        goal: String,
        status: String,
        team: serde_json::Value,
    },
    PantheonTeamAssembled {
        mission_id: String,
        agents: serde_json::Value,
    },
    PantheonTaskAssigned {
        mission_id: String,
        task_id: String,
        agent_id: String,
        description: String,
    },
    PantheonAgentActivity {
        mission_id: String,
        agent_id: String,
        agent_name: String,
        activity: String,
        detail: serde_json::Value,
    },
    PantheonTaskCompleted {
        mission_id: String,
        task_id: String,
        result: String,
    },
    PantheonReviewRequested {
        mission_id: String,
        task_id: String,
        reviewer: String,
    },
    PantheonProgress {
        mission_id: String,
        progress_pct: f64,
        tasks_done: usize,
        tasks_total: usize,
        tokens_used: u64,
    },
    PantheonArtifact {
        mission_id: String,
        name: String,
        path: String,
        artifact_type: String,
    },
    PantheonMissionComplete {
        mission_id: String,
        status: String,
        summary: String,
        artifacts: serde_json::Value,
    },
    PantheonWatching {
        mission_id: String,
    },
    // Room events (generic JSON payload)
    PantheonGenericEvent {
        event_type: String,
        data: serde_json::Value,
    },
    // Studio puppet session events
    StudioUpdate {
        event: StudioEvent,
    },
    StudioWatching {
        session_id: String,
    },
}

// ============================================================================
// Handler
// ============================================================================

/// Default maximum WebSocket message size (1 MB).
/// Overridable via `[gateway].max_ws_message_bytes` in config.toml.
const DEFAULT_MAX_WS_MESSAGE_SIZE: usize = zeus_core::MAX_WS_MESSAGE_BYTES;

/// WebSocket upgrade handler at `GET /v1/ws`.
///
/// Supports optional token auth via `?token=<bearer>` query parameter.
/// When the server has auth configured, this rejects unauthenticated upgrades.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    // Check if auth is configured (via ZEUS_API_TOKEN env var) and validate token.
    // The auth token is not stored in AppState — it's passed to the router layer.
    // WebSocket must check independently since it's upgraded before middleware runs.
    if let Ok(expected_token) = std::env::var("ZEUS_API_TOKEN") {
        let provided = params.get("token").map(|s| s.as_str()).unwrap_or("");

        // #432 WS parity: identity-store tokens authenticate too.
        let store_valid = {
            let s = state.read().await;
            match &s.identity_store {
                Some(store) => matches!(store.resolve_token(provided), Ok(Some(_))),
                None => false,
            }
        };

        // Constant-time comparison against root token
        let root_valid = provided.len() == expected_token.len()
            && provided
                .as_bytes()
                .iter()
                .zip(expected_token.as_bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0;
        if !root_valid && !store_valid {
            return Response::builder()
                .status(axum::http::StatusCode::UNAUTHORIZED)
                .body(axum::body::Body::from("Unauthorized"))
                .unwrap_or_default();
        }
    }

    let max_ws = {
        let s = state.read().await;
        s.config
            .gateway
            .as_ref()
            .map(|g| g.max_ws_message_bytes)
            .unwrap_or(DEFAULT_MAX_WS_MESSAGE_SIZE)
    };
    ws.max_message_size(max_ws)
        .on_upgrade(move |socket| handle_socket(socket, state))
}

/// Run the v3 Ed25519 challenge-response handshake.
///
/// Returns `Ok(())` if authentication succeeds, `Err(())` if it fails
/// (the caller should close the socket on failure).
async fn run_auth_handshake(
    sender: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    receiver: &mut (impl StreamExt<Item = Result<Message, axum::Error>> + Unpin),
    keypair: &crate::ws_auth::WsKeyPair,
    tolerance: u64,
) -> Result<(), ()> {
    // 1. Generate and send challenge
    let challenge = match crate::ws_auth::new_challenge() {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to create auth challenge: {e}");
            let _ = send_server_msg(
                sender,
                &ServerMessage::AuthFailed {
                    reason: "internal error".into(),
                },
            )
            .await;
            return Err(());
        }
    };

    let _ = send_server_msg(
        sender,
        &ServerMessage::AuthChallenge {
            nonce: challenge.nonce_b64.clone(),
            server_timestamp: challenge.issued_at,
            version: "3".into(),
        },
    )
    .await;

    // 2. Wait for response (15s timeout)
    let response = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    return serde_json::from_str::<ClientMessage>(&text).ok();
                }
                Ok(Message::Close(_)) | Err(_) => return None,
                _ => continue,
            }
        }
        None
    })
    .await;

    let client_msg = match response {
        Ok(Some(msg)) => msg,
        _ => {
            let _ = send_server_msg(
                sender,
                &ServerMessage::AuthFailed {
                    reason: "auth timeout".into(),
                },
            )
            .await;
            return Err(());
        }
    };

    // 3. Verify
    match client_msg {
        ClientMessage::AuthResponse {
            nonce,
            timestamp,
            signature,
        } => {
            match crate::ws_auth::verify_response(
                keypair, &challenge, &nonce, timestamp, &signature, tolerance,
            ) {
                Ok(()) => {
                    let _ = send_server_msg(
                        sender,
                        &ServerMessage::AuthOk {
                            version: "3".into(),
                            public_key_hex: crate::ws_auth::public_key_hex(keypair),
                        },
                    )
                    .await;
                    info!("WebSocket v3 auth succeeded");
                    Ok(())
                }
                Err(e) => {
                    warn!("WebSocket v3 auth failed: {e}");
                    let _ = send_server_msg(
                        sender,
                        &ServerMessage::AuthFailed {
                            reason: e.to_string(),
                        },
                    )
                    .await;
                    Err(())
                }
            }
        }
        _ => {
            let _ = send_server_msg(
                sender,
                &ServerMessage::AuthFailed {
                    reason: "expected auth_response".into(),
                },
            )
            .await;
            Err(())
        }
    }
}

/// Public key endpoint handler for `GET /v1/ws/pubkey`.
pub async fn ws_pubkey_handler(State(state): State<SharedState>) -> axum::response::Response {
    let guard = state.read().await;
    match &guard.ws_keypair {
        Some(kp) => {
            let body = serde_json::json!({
                "public_key": crate::ws_auth::public_key_hex(kp),
                "algorithm": "ed25519",
                "version": "3",
            });
            axum::response::IntoResponse::into_response(axum::Json(body))
        }
        None => axum::response::IntoResponse::into_response((
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "ws_auth not enabled"})),
        )),
    }
}

/// Handle an active WebSocket connection.
async fn handle_socket(socket: WebSocket, state: SharedState) {
    let (mut sender, mut receiver) = socket.split();
    info!("WebSocket client connected");

    // Run v3 Ed25519 handshake if configured
    {
        let guard = state.read().await;
        let ws_auth_enabled = guard.config.ws_auth.as_ref().is_some_and(|c| c.enabled);
        if ws_auth_enabled {
            if let Some(ref kp) = guard.ws_keypair {
                let kp = kp.clone();
                let tolerance = guard
                    .config
                    .ws_auth
                    .as_ref()
                    .map_or(30, |c| c.timestamp_tolerance_secs);
                drop(guard); // Release lock before async I/O
                if run_auth_handshake(&mut sender, &mut receiver, &kp, tolerance)
                    .await
                    .is_err()
                {
                    warn!("WebSocket v3 auth handshake failed, closing connection");
                    return;
                }
            } else {
                drop(guard);
                warn!("ws_auth enabled but no keypair loaded, rejecting connection");
                let _ = send_server_msg(
                    &mut sender,
                    &ServerMessage::AuthFailed {
                        reason: "server key not available".into(),
                    },
                )
                .await;
                return;
            }
        }
    }

    // Subscribe to approval broadcast events
    let mut approval_rx = state.read().await.approvals.subscribe();

    // Subscribe to plan execution broadcast events
    let mut plan_rx = state.read().await.plan_broadcast.subscribe();

    // Subscribe to orchestration broadcast events
    let mut orch_rx = state.read().await.orchestration_broadcast.subscribe();

    // Subscribe to Pantheon mission broadcast events
    let mut pantheon_rx = state.read().await.pantheon.subscribe();

    // Subscribe to Studio puppet session broadcast events
    let mut studio_rx = state.read().await.studio_broadcast.subscribe();

    // Track which plan_ids this client is watching
    let watched_plans: Arc<RwLock<HashSet<String>>> = Arc::new(RwLock::new(HashSet::new()));

    // Track which orchestration session_ids this client is watching
    let watched_orchestrations: Arc<RwLock<HashSet<String>>> =
        Arc::new(RwLock::new(HashSet::new()));

    // Track which mission_ids this client is watching
    let watched_missions: Arc<RwLock<HashSet<String>>> = Arc::new(RwLock::new(HashSet::new()));
    // Track which studio session_ids this client is watching
    let watched_studios: Arc<RwLock<HashSet<String>>> = Arc::new(RwLock::new(HashSet::new()));

    // Track current chat agent handle for cancellation
    let current_chat_handle: Arc<Mutex<Option<tokio::task::AbortHandle>>> =
        Arc::new(Mutex::new(None));

    // Channel for spawned chat handler to send messages back
    let (chat_event_tx, mut chat_event_rx) = mpsc::channel::<ServerMessage>(256);

    loop {
        tokio::select! {
            msg = receiver.next() => {
                let msg = match msg {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None => {
                        debug!("WebSocket client disconnected");
                        break;
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => {
                        warn!("WebSocket receive error: {}", e);
                        break;
                    }
                };

                // Reject oversized messages (defense-in-depth)
                if msg.len() > DEFAULT_MAX_WS_MESSAGE_SIZE {
                    let err = ServerMessage::Error {
                        message: "Message too large".to_string(),
                    };
                    let _ = sender
                        .send(Message::Text(serde_json::to_string(&err).unwrap_or_default()))
                        .await;
                    continue;
                }

                let client_msg: ClientMessage = match serde_json::from_str(&msg) {
                    Ok(m) => m,
                    Err(e) => {
                        let err = ServerMessage::Error {
                            message: format!("Invalid message: {}", e),
                        };
                        let _ = sender
                            .send(Message::Text(serde_json::to_string(&err).unwrap_or_default()))
                            .await;
                        continue;
                    }
                };

                match client_msg {
                    ClientMessage::Ping => {
                        let pong = serde_json::to_string(&ServerMessage::Pong).unwrap_or_default();
                        let _ = sender.send(Message::Text(pong)).await;
                    }
                    ClientMessage::CancelStream => {
                        let mut handle = current_chat_handle.lock().await;
                        if let Some(h) = handle.take() {
                            h.abort();
                            info!("Cancelled in-flight stream");
                            let _ = send_server_msg(&mut sender, &ServerMessage::StreamCancelled).await;
                        } else {
                            debug!("CancelStream received but no active stream");
                        }
                    }
                    ClientMessage::Chat {
                        message,
                        session_id,
                    } => {
                        let state_clone = state.clone();
                        let msg_clone = message.clone();
                        let sid_clone = session_id.clone();
                        let tx = chat_event_tx.clone();
                        let handle_clone = current_chat_handle.clone();

                        let join_handle = tokio::spawn(async move {
                            handle_chat_via_channel(&tx, &state_clone, &msg_clone, sid_clone.as_deref()).await;
                        });
                        let abort_handle = join_handle.abort_handle();
                        *handle_clone.lock().await = Some(abort_handle);
                    }
                    ClientMessage::Studio {
                        message,
                        session_id,
                        team_id,
                    } => {
                        handle_studio(&mut sender, &state, &message, session_id.as_deref(), team_id.as_deref()).await;
                    }
                    ClientMessage::PrometheusWatch { plan_id } => {
                        let mut plans = watched_plans.write().await;
                        plans.insert(plan_id.clone());
                        info!("WebSocket client watching plan: {}", plan_id);
                        let _ = send_server_msg(
                            &mut sender,
                            &ServerMessage::StudioEvent {
                                event_type: "prometheus_watching".to_string(),
                                data: serde_json::json!({"plan_id": plan_id}),
                            },
                        )
                        .await;
                    }
                    ClientMessage::OrchestrationWatch { session_id } => {
                        let mut orchs = watched_orchestrations.write().await;
                        orchs.insert(session_id.clone());
                        info!("WebSocket client watching orchestration: {}", session_id);
                        let _ = send_server_msg(
                            &mut sender,
                            &ServerMessage::OrchestrationWatching {
                                session_id,
                            },
                        )
                        .await;
                    }
                    ClientMessage::Workflow { message, session_id: _, mode } => {
                        handle_workflow(&mut sender, &state, &message, mode.as_deref(), &watched_plans).await;
                    }
                    ClientMessage::Reaction { message_id, emoji } => {
                        debug!("Reaction {} on message {}", emoji, message_id);
                        let _ = send_server_msg(&mut sender, &ServerMessage::MessageAck {
                            action: "reaction".into(),
                            message_id,
                            success: true,
                        }).await;
                    }
                    ClientMessage::Pin { message_id, pinned } => {
                        debug!("Pin({}) message {}", pinned, message_id);
                        let _ = send_server_msg(&mut sender, &ServerMessage::MessageAck {
                            action: "pin".into(),
                            message_id,
                            success: true,
                        }).await;
                    }
                    ClientMessage::Edit { message_id, content } => {
                        debug!("Edit message {} -> {} chars", message_id, content.len());
                        let _ = send_server_msg(&mut sender, &ServerMessage::MessageAck {
                            action: "edit".into(),
                            message_id,
                            success: true,
                        }).await;
                    }
                    ClientMessage::Forward { message_id, target_type, target_id } => {
                        debug!("Forward message {} to {}:{}", message_id, target_type, target_id);
                        let _ = send_server_msg(&mut sender, &ServerMessage::MessageAck {
                            action: "forward".into(),
                            message_id,
                            success: true,
                        }).await;
                    }
                    ClientMessage::Delete { message_id } => {
                        debug!("Delete message {}", message_id);
                        let _ = send_server_msg(&mut sender, &ServerMessage::MessageAck {
                            action: "delete".into(),
                            message_id,
                            success: true,
                        }).await;
                    }
                    ClientMessage::ReadReceipt { message_id } => {
                        debug!("Read receipt for message {}", message_id);
                        let _ = send_server_msg(&mut sender, &ServerMessage::MessageAck {
                            action: "read_receipt".into(),
                            message_id,
                            success: true,
                        }).await;
                    }
                    ClientMessage::AuthResponse { .. } => {
                        // Auth responses are only valid during the handshake phase;
                        // ignore if received during normal operation.
                        debug!("Ignoring auth_response outside handshake");
                    }
                    // Pantheon — multi-agent collaboration
                    ClientMessage::PantheonMission { goal, constraints } => {
                        use crate::handlers::PantheonEvent;
                        let store = {
                            let s = state.read().await;
                            s.pantheon.clone()
                        };
                        let constraints_typed = constraints.and_then(|c| {
                            serde_json::from_value(c).ok()
                        }).unwrap_or_default();
                        let mut mission = crate::handlers::pantheon::Mission::new(goal.clone(), constraints_typed);
                        let mission_id = mission.id.clone();
                        let team = crate::handlers::pantheon::assemble_team_heuristic(&goal);
                        mission.team = team.clone();
                        mission.status = crate::handlers::pantheon::MissionStatus::Assembling;
                        store.insert(mission.clone()).await;
                        store.emit(PantheonEvent::MissionCreated {
                            mission_id: mission_id.clone(),
                            goal: goal.clone(),
                            status: "assembling".to_string(),
                        });
                        let _ = send_server_msg(&mut sender, &ServerMessage::PantheonMissionCreated {
                            mission_id: mission_id.clone(),
                            goal,
                            status: "assembling".to_string(),
                            team: serde_json::to_value(&team).unwrap_or_default(),
                        }).await;
                        info!("Pantheon WS mission: {}", mission_id);
                    }
                    ClientMessage::PantheonWatch { mission_id } => {
                        let mut missions = watched_missions.write().await;
                        missions.insert(mission_id.clone());
                        drop(missions);
                        info!("WebSocket client watching mission: {}", mission_id);
                        let _ = send_server_msg(&mut sender, &ServerMessage::PantheonWatching {
                            mission_id,
                        }).await;
                    }
                    ClientMessage::StudioWatch { session_id } => {
                        let mut studios = watched_studios.write().await;
                        studios.insert(session_id.clone());
                        drop(studios);
                        info!("WebSocket client watching studio session: {}", session_id);
                        let _ = send_server_msg(&mut sender, &ServerMessage::StudioWatching {
                            session_id,
                        }).await;
                    }
                    ClientMessage::PantheonIntervene { mission_id, action, message } => {
                        let store = state.read().await.pantheon.clone();
                        let msg_clone = message.clone();
                        store.update(&mission_id, |m| {
                            m.add_activity("user", "User", "intervention",
                                serde_json::json!({"action": action, "message": msg_clone}));
                            if action == "cancel" {
                                m.status = crate::handlers::pantheon::MissionStatus::Cancelled;
                            }
                        }).await;
                        debug!("Pantheon intervention: {} on {}", action, mission_id);
                    }
                    ClientMessage::PantheonApprove { mission_id, task_id } => {
                        let store = state.read().await.pantheon.clone();
                        let tid = task_id.clone();
                        store.update(&mission_id, |m| {
                            if let Some(t) = m.tasks.iter_mut().find(|t| t.id == tid) {
                                t.status = crate::handlers::pantheon::TaskStatus::Approved;
                            }
                            m.add_activity("user", "User", "review",
                                serde_json::json!({"task_id": task_id, "verdict": "approve"}));
                        }).await;
                    }
                    ClientMessage::PantheonReject { mission_id, task_id, reason } => {
                        let store = state.read().await.pantheon.clone();
                        let tid = task_id.clone();
                        let r = reason.clone();
                        store.update(&mission_id, |m| {
                            if let Some(t) = m.tasks.iter_mut().find(|t| t.id == tid) {
                                t.status = crate::handlers::pantheon::TaskStatus::Rejected;
                            }
                            m.add_activity("user", "User", "review",
                                serde_json::json!({"task_id": task_id, "verdict": "reject", "reason": r}));
                        }).await;
                    }
                }
            }
            Ok(event) = approval_rx.recv() => {
                use crate::approvals::ApprovalEvent;
                let server_msg = match event {
                    ApprovalEvent::ApprovalPending { approval } => ServerMessage::ApprovalPending {
                        id: approval.id,
                        tool_name: approval.tool_name,
                        args: approval.args,
                        agent_id: approval.agent_id,
                    },
                    ApprovalEvent::ApprovalResolved { id, status } => ServerMessage::ApprovalResolved {
                        id,
                        status: serde_json::to_value(status).unwrap_or_default(),
                    },
                };
                if send_server_msg(&mut sender, &server_msg).await.is_err() {
                    break;
                }
            }
            // Forward events from spawned chat handler
            Some(server_msg) = chat_event_rx.recv() => {
                if send_server_msg(&mut sender, &server_msg).await.is_err() {
                    break;
                }
            }
            Ok(plan_event) = plan_rx.recv() => {
                let plans = watched_plans.read().await;
                match &plan_event {
                    PlanEvent::StepUpdate(update) => {
                        if plans.contains(&update.plan_id) {
                            let msg = ServerMessage::PrometheusUpdate {
                                plan_id: update.plan_id.clone(),
                                step_id: update.step_id,
                                status: update.status.clone(),
                                progress_pct: update.progress_pct,
                                output: update.output.clone(),
                            };
                            if send_server_msg(&mut sender, &msg).await.is_err() {
                                break;
                            }
                        }
                    }
                    PlanEvent::Complete(complete) => {
                        if plans.contains(&complete.plan_id) {
                            let msg = ServerMessage::PrometheusComplete {
                                plan_id: complete.plan_id.clone(),
                                status: complete.status.clone(),
                                steps_completed: complete.steps_completed,
                                steps_failed: complete.steps_failed,
                                duration_ms: complete.duration_ms,
                            };
                            if send_server_msg(&mut sender, &msg).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
            Ok(orch_event) = orch_rx.recv() => {
                let orchs = watched_orchestrations.read().await;
                let session_id = match &orch_event {
                    OrchestrationEvent::PhaseChanged { session_id, .. }
                    | OrchestrationEvent::StepStarted { session_id, .. }
                    | OrchestrationEvent::StepCompleted { session_id, .. }
                    | OrchestrationEvent::StepFailed { session_id, .. }
                    | OrchestrationEvent::AgentAssigned { session_id, .. }
                    | OrchestrationEvent::ArtifactCreated { session_id, .. }
                    | OrchestrationEvent::Complete { session_id, .. }
                    | OrchestrationEvent::SpawnStarted { session_id, .. }
                    | OrchestrationEvent::SpawnCompleted { session_id, .. }
                    | OrchestrationEvent::SpawnFailed { session_id, .. } => session_id.clone(),
                };
                if orchs.contains(&session_id) {
                    let msg = match orch_event {
                        OrchestrationEvent::PhaseChanged { session_id, phase, data } => {
                            ServerMessage::OrchestrationPhaseChanged { session_id, phase, data }
                        }
                        OrchestrationEvent::StepStarted { session_id, step_index, step_description, steps_total } => {
                            ServerMessage::OrchestrationStepStarted { session_id, step_index, step_description, steps_total }
                        }
                        OrchestrationEvent::StepCompleted { session_id, step_index, output, progress_pct } => {
                            ServerMessage::OrchestrationStepCompleted { session_id, step_index, output, progress_pct }
                        }
                        OrchestrationEvent::StepFailed { session_id, step_index, error } => {
                            ServerMessage::OrchestrationStepFailed { session_id, step_index, error }
                        }
                        OrchestrationEvent::AgentAssigned { session_id, agent_role, model_tier } => {
                            ServerMessage::OrchestrationAgentAssigned { session_id, agent_role, model_tier }
                        }
                        OrchestrationEvent::ArtifactCreated { session_id, artifact_name, artifact_path } => {
                            ServerMessage::OrchestrationArtifactCreated { session_id, artifact_name, artifact_path }
                        }
                        OrchestrationEvent::Complete { session_id, status, summary, artifact_path, duration_ms } => {
                            ServerMessage::OrchestrationComplete { session_id, status, summary, artifact_path, duration_ms }
                        }
                        OrchestrationEvent::SpawnStarted { session_id, agent_id, role, task, depth } => {
                            ServerMessage::SpawnStarted { session_id, agent_id, role, task, depth }
                        }
                        OrchestrationEvent::SpawnCompleted { session_id, agent_id, role, success, output, duration_ms } => {
                            ServerMessage::SpawnCompleted { session_id, agent_id, role, success, output, duration_ms }
                        }
                        OrchestrationEvent::SpawnFailed { session_id, agent_id, role, error } => {
                            ServerMessage::SpawnFailed { session_id, agent_id, role, error }
                        }
                    };
                    if send_server_msg(&mut sender, &msg).await.is_err() {
                        break;
                    }
                }
            }
            Ok(pantheon_event) = pantheon_rx.recv() => {
                use crate::handlers::pantheon::PantheonEvent;
                let missions = watched_missions.read().await;

                // Room events are broadcast to all clients; mission events are filtered
                let is_room_event = matches!(
                    &pantheon_event,
                    PantheonEvent::RoomCreated { .. }
                    | PantheonEvent::RoomMessageSent { .. }
                    | PantheonEvent::AgentJoinedRoom { .. }
                    | PantheonEvent::AgentLeftRoom { .. }
                    | PantheonEvent::PlanCardCreated { .. }
                    | PantheonEvent::PlanApproved { .. }
                    | PantheonEvent::PlanRejected { .. }
                );

                let mission_id = match &pantheon_event {
                    PantheonEvent::MissionCreated { mission_id, .. }
                    | PantheonEvent::TeamAssembled { mission_id, .. }
                    | PantheonEvent::TaskAssigned { mission_id, .. }
                    | PantheonEvent::AgentActivity { mission_id, .. }
                    | PantheonEvent::TaskCompleted { mission_id, .. }
                    | PantheonEvent::ReviewRequested { mission_id, .. }
                    | PantheonEvent::MissionProgress { mission_id, .. }
                    | PantheonEvent::Artifact { mission_id, .. }
                    | PantheonEvent::MissionApproved { mission_id, .. }
                    | PantheonEvent::MissionComplete { mission_id, .. }
                    | PantheonEvent::MissionFailed { mission_id, .. } => Some(mission_id.clone()),
                    _ => None,
                };

                let should_send = is_room_event || mission_id.as_ref().is_some_and(|mid| missions.contains(mid));

                if should_send {
                    let msg_json = serde_json::to_value(&pantheon_event).unwrap_or_default();
                    let event_type = match &pantheon_event {
                        PantheonEvent::MissionCreated { .. } => "mission_created",
                        PantheonEvent::TeamAssembled { .. } => "team_assembled",
                        PantheonEvent::TaskAssigned { .. } => "task_assigned",
                        PantheonEvent::AgentActivity { .. } => "agent_activity",
                        PantheonEvent::TaskCompleted { .. } => "task_completed",
                        PantheonEvent::ReviewRequested { .. } => "review_requested",
                        PantheonEvent::MissionProgress { .. } => "mission_progress",
                        PantheonEvent::Artifact { .. } => "artifact",
                        PantheonEvent::MissionApproved { .. } => "mission_approved",
                        PantheonEvent::MissionComplete { .. } => "mission_complete",
                        PantheonEvent::MissionFailed { .. } => "mission_failed",
                        PantheonEvent::MissionRejected { .. } => "mission_rejected",
                        PantheonEvent::RoomCreated { .. } => "room_created",
                        PantheonEvent::RoomMessageSent { .. } => "room_message",
                        PantheonEvent::AgentJoinedRoom { .. } => "agent_joined_room",
                        PantheonEvent::AgentLeftRoom { .. } => "agent_left_room",
                        PantheonEvent::PlanCardCreated { .. } => "plan_card_created",
                        PantheonEvent::PlanApproved { .. } => "plan_approved",
                        PantheonEvent::PlanRejected { .. } => "plan_rejected",
                    };

                    // For mission events, use the existing typed ServerMessages
                    // For room events, send as generic JSON
                    let msg = match pantheon_event {
                        PantheonEvent::MissionCreated { mission_id, goal, status } => {
                            ServerMessage::PantheonMissionCreated {
                                mission_id, goal, status,
                                team: serde_json::json!([]),
                            }
                        }
                        PantheonEvent::TeamAssembled { mission_id, agents } => {
                            ServerMessage::PantheonTeamAssembled {
                                mission_id,
                                agents: serde_json::to_value(agents).unwrap_or_default(),
                            }
                        }
                        PantheonEvent::TaskAssigned { mission_id, task_id, agent_id, description } => {
                            ServerMessage::PantheonTaskAssigned { mission_id, task_id, agent_id, description }
                        }
                        PantheonEvent::AgentActivity { mission_id, agent_id, agent_name, activity, detail } => {
                            ServerMessage::PantheonAgentActivity { mission_id, agent_id, agent_name, activity, detail }
                        }
                        PantheonEvent::TaskCompleted { mission_id, task_id, result } => {
                            ServerMessage::PantheonTaskCompleted { mission_id, task_id, result }
                        }
                        PantheonEvent::ReviewRequested { mission_id, task_id, reviewer } => {
                            ServerMessage::PantheonReviewRequested { mission_id, task_id, reviewer }
                        }
                        PantheonEvent::MissionProgress { mission_id, progress_pct, tasks_done, tasks_total, tokens_used } => {
                            ServerMessage::PantheonProgress { mission_id, progress_pct, tasks_done, tasks_total, tokens_used }
                        }
                        PantheonEvent::Artifact { mission_id, name, path, artifact_type } => {
                            ServerMessage::PantheonArtifact { mission_id, name, path, artifact_type }
                        }
                        PantheonEvent::MissionApproved { mission_id, approved_by } => {
                            ServerMessage::PantheonGenericEvent {
                                event_type: "mission_approved".to_string(),
                                data: serde_json::json!({
                                    "type": "mission_approved",
                                    "mission_id": mission_id,
                                    "approved_by": approved_by,
                                }),
                            }
                        }
                        PantheonEvent::MissionComplete { mission_id, status, summary, artifacts } => {
                            ServerMessage::PantheonMissionComplete {
                                mission_id, status, summary,
                                artifacts: serde_json::to_value(artifacts).unwrap_or_default(),
                            }
                        }
                        PantheonEvent::MissionFailed { mission_id, reason } => {
                            ServerMessage::PantheonMissionComplete {
                                mission_id,
                                status: "failed".to_string(),
                                summary: reason,
                                artifacts: serde_json::json!([]),
                            }
                        }
                        // Room events: send as generic PantheonEvent JSON
                        _ => {
                            ServerMessage::PantheonGenericEvent {
                                event_type: event_type.to_string(),
                                data: msg_json,
                            }
                        }
                    };
                    if send_server_msg(&mut sender, &msg).await.is_err() {
                        break;
                    }
                }
            }
            // Studio puppet session events → relay to watching clients
            Ok(studio_event) = studio_rx.recv() => {
                let studios = watched_studios.read().await;
                let event_session_id = match &studio_event {
                    StudioEvent::StatusChanged { session_id, .. }
                    | StudioEvent::ActionDispatched { session_id, .. }
                    | StudioEvent::ActionCompleted { session_id, .. }
                    | StudioEvent::SessionComplete { session_id, .. }
                    | StudioEvent::SessionFailed { session_id, .. } => session_id.clone(),
                };
                if studios.contains(&event_session_id) {
                    let msg = ServerMessage::StudioUpdate { event: studio_event };
                    if send_server_msg(&mut sender, &msg).await.is_err() {
                        break;
                    }
                }
            }
        }
    }

    debug!("WebSocket connection closed");
}

/// Process a chat message: run the agent loop and stream events back.
async fn handle_chat(
    sender: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    state: &SharedState,
    message: &str,
    session_id: Option<&str>,
) {
    let state_guard = state.read().await;

    // Create or load session
    let session = match session_id {
        Some(id) => match Session::load(&state_guard.config.sessions, id).await {
            Ok(s) => s,
            Err(e) => {
                let _ = send_server_msg(
                    sender,
                    &ServerMessage::Error {
                        message: format!("Session not found: {}", e),
                    },
                )
                .await;
                return;
            }
        },
        None => {
            let s = Session::new(&state_guard.config.sessions);
            if let Err(e) = s.init().await {
                let _ = send_server_msg(
                    sender,
                    &ServerMessage::Error {
                        message: format!("Failed to create session: {}", e),
                    },
                )
                .await;
                return;
            }
            s
        }
    };

    // Route through unified inbox if available (prevents concurrent session writes).
    // Falls back to fresh-agent path if inbox is not yet wired.
    if let Some(inbox) = state_guard.agent_inbox.as_ref().cloned() {
        drop(state_guard); // Release read lock before awaiting

        let _ = send_server_msg(sender, &ServerMessage::Started).await;

        match inbox.send_and_wait_with_options(
            message.to_string(),
            None, // WebSocket — no ChannelSource needed for response routing
            zeus_core::inbox::InboxSendOptions::new(vec![], 300, false, None)
                .with_session_id(Some(session.id.clone())),
        ).await {
            Ok(response) => {
                let _ = send_server_msg(sender, &ServerMessage::ResponseComplete {
                    content: response,
                    session_id: Some(session.id.clone()),
                }).await;
                let _ = send_server_msg(sender, &ServerMessage::Finished { iterations: 1 }).await;
            }
            Err(e) => {
                let _ = send_server_msg(sender, &ServerMessage::Error {
                    message: e,
                }).await;
            }
        }
        return;
    }

    // Fallback: create LLM client and run fresh agent (used when inbox is None)
    let llm = match LlmClient::from_config(&state_guard.config) {
        Ok(l) => l,
        Err(e) => {
            let _ = send_server_msg(
                sender,
                &ServerMessage::Error {
                    message: format!("LLM init failed: {}", e),
                },
            )
            .await;
            return;
        }
    };

    // Set up the agent with event channel
    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(256);

    let config = state_guard.config.clone();
    let workspace = state_guard.workspace.clone();
    drop(state_guard); // Release the read lock before running the agent

    let ws_session_id = session.id.clone();
    let agent = zeus_agent::Agent::new(config, llm, workspace, session, None);
    let mut agent = agent.with_events(event_tx);

    let msg = message.to_string();

    // Spawn the agent loop
    let agent_handle = tokio::spawn(async move { agent.run(&msg).await });

    // Forward events to WebSocket
    while let Some(event) = event_rx.recv().await {
        let server_msg = match event {
            AgentEvent::Started => ServerMessage::Started,
            AgentEvent::TextChunk(chunk) => ServerMessage::TextChunk { chunk },
            AgentEvent::ToolCall { name, args } => ServerMessage::ToolCall { name, args },
            AgentEvent::ToolResult {
                name,
                success,
                output,
            } => ServerMessage::ToolResult {
                name,
                success,
                output,
            },
            AgentEvent::ResponseComplete(resp) => ServerMessage::ResponseComplete {
                content: resp.content,
                session_id: Some(ws_session_id.clone()),
            },
            AgentEvent::Finished { iterations } => ServerMessage::Finished {
                iterations: iterations as u32,
            },
            AgentEvent::Error(msg) => ServerMessage::Error { message: msg },
            AgentEvent::Compacted { messages_removed, tokens_before, tokens_after } => {
                ServerMessage::Compacted { messages_removed, tokens_before, tokens_after }
            }
            AgentEvent::OAuthComplete(_) => continue, // TUI-only event, skip in API
        };

        if send_server_msg(sender, &server_msg).await.is_err() {
            break; // Client disconnected
        }
    }

    // Wait for agent to finish
    match agent_handle.await {
        Ok(Ok(_response)) => {}
        Ok(Err(e)) => {
            let _ = send_server_msg(
                sender,
                &ServerMessage::Error {
                    message: e.to_string(),
                },
            )
            .await;
        }
        Err(e) => {
            error!("Agent task panicked: {}", e);
            let _ = send_server_msg(
                sender,
                &ServerMessage::Error {
                    message: "Internal error".to_string(),
                },
            )
            .await;
        }
    }
}

/// Process a chat message via channel: run the agent loop and stream events back through channel.
async fn handle_chat_via_channel(
    tx: &mpsc::Sender<ServerMessage>,
    state: &SharedState,
    message: &str,
    session_id: Option<&str>,
) {
    let state_guard = state.read().await;

    // Create or load session
    let session = match session_id {
        Some(id) => match Session::load(&state_guard.config.sessions, id).await {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(ServerMessage::Error {
                    message: format!("Session not found: {}", e),
                }).await;
                return;
            }
        },
        None => {
            let s = Session::new(&state_guard.config.sessions);
            if let Err(e) = s.init().await {
                let _ = tx.send(ServerMessage::Error {
                    message: format!("Failed to create session: {}", e),
                }).await;
                return;
            }
            s
        }
    };

    // Route through unified inbox if available (prevents concurrent session writes).
    // Falls back to fresh-agent path if inbox is not yet wired.
    if let Some(inbox) = state_guard.agent_inbox.as_ref().cloned() {
        drop(state_guard); // Release read lock before awaiting

        let _ = tx.send(ServerMessage::Started).await;

        match inbox.send_and_wait_with_options(
            message.to_string(),
            None, // WebSocket — no ChannelSource needed for response routing
            zeus_core::inbox::InboxSendOptions::new(vec![], 300, false, None)
                .with_session_id(Some(session.id.clone())),
        ).await {
            Ok(response) => {
                let _ = tx.send(ServerMessage::ResponseComplete {
                    content: response,
                    session_id: Some(session.id.clone()),
                }).await;
                let _ = tx.send(ServerMessage::Finished { iterations: 1 }).await;
            }
            Err(e) => {
                let _ = tx.send(ServerMessage::Error {
                    message: e,
                }).await;
            }
        }
        return;
    }

    // Fallback: create LLM client and run fresh agent
    let llm = match LlmClient::from_config(&state_guard.config) {
        Ok(l) => l,
        Err(e) => {
            let _ = tx.send(ServerMessage::Error {
                message: format!("LLM init failed: {}", e),
            }).await;
            return;
        }
    };

    // Set up the agent with event channel
    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(256);

    let config = state_guard.config.clone();
    let workspace = state_guard.workspace.clone();
    drop(state_guard);

    let agent = zeus_agent::Agent::new(config, llm, workspace, session, None);
    let mut agent = agent.with_events(event_tx);

    let msg = message.to_string();

    // Spawn the agent loop
    let agent_handle = tokio::spawn(async move { agent.run(&msg).await });

    // Forward events to channel
    while let Some(event) = event_rx.recv().await {
        let server_msg = match event {
            AgentEvent::Started => ServerMessage::Started,
            AgentEvent::TextChunk(chunk) => ServerMessage::TextChunk { chunk },
            AgentEvent::ToolCall { name, args } => ServerMessage::ToolCall { name, args },
            AgentEvent::ToolResult {
                name,
                success,
                output,
            } => ServerMessage::ToolResult {
                name,
                success,
                output,
            },
            AgentEvent::ResponseComplete(resp) => ServerMessage::ResponseComplete {
                content: resp.content,
                session_id: None,
            },
            AgentEvent::Finished { iterations } => ServerMessage::Finished {
                iterations: iterations as u32,
            },
            AgentEvent::Error(msg) => ServerMessage::Error { message: msg },
            AgentEvent::Compacted { messages_removed, tokens_before, tokens_after } => {
                ServerMessage::Compacted { messages_removed, tokens_before, tokens_after }
            }
            AgentEvent::OAuthComplete(_) => continue,
        };

        if tx.send(server_msg).await.is_err() {
            break; // Channel closed
        }
    }

    // Wait for agent to finish
    match agent_handle.await {
        Ok(Ok(_response)) => {}
        Ok(Err(e)) => {
            let _ = tx.send(ServerMessage::Error {
                message: e.to_string(),
            }).await;
        }
        Err(e) => {
            error!("Agent task panicked: {}", e);
            let _ = tx.send(ServerMessage::Error {
                message: "Internal error".to_string(),
            }).await;
        }
    }
}

/// Process a Studio message: route through the orchestra system via the AgentRegistry.
///
/// Both paths spawn an ephemeral agent through `AgentRegistry::spawn_dynamic()`,
/// making the studio agent visible in the registry during execution and enabling
/// real orchestration from the Agent Studio web frontend.
///
/// When `team_id` is provided, the agent receives team-aware context (members,
/// supervisor, policy) injected into its system prompt.
///
/// When `team_id` is None, the agent receives the full studio capability prompt
/// for general orchestration (agent/team management, tools, channels, etc.).
async fn handle_studio(
    sender: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    state: &SharedState,
    message: &str,
    session_id: Option<&str>,
    team_id: Option<&str>,
) {
    // Notify client that studio processing has started
    let _ = send_server_msg(
        sender,
        &ServerMessage::StudioEvent {
            event_type: "studio_started".to_string(),
            data: serde_json::json!({
                "message": message,
                "team_id": team_id,
            }),
        },
    )
    .await;

    if let Some(tid) = team_id {
        handle_studio_team(sender, state, message, session_id, tid).await;
    } else {
        handle_studio_default(sender, state, message, session_id).await;
    }
}

/// Studio with team_id: spawn a dynamic agent via the AgentRegistry, run the
/// message with event streaming, then clean up the ephemeral agent.
async fn handle_studio_team(
    sender: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    state: &SharedState,
    message: &str,
    _session_id: Option<&str>,
    team_id: &str,
) {
    // 1. Look up the team to build context
    let team_context = {
        let state_guard = state.read().await;
        match state_guard.orchestra().get_team(team_id).await {
            Ok(team) => {
                let members = if team.agent_ids.is_empty() {
                    "none".to_string()
                } else {
                    team.agent_ids.join(", ")
                };
                let supervisor = team.supervisor_id.as_deref().unwrap_or("none");
                format!(
                    "You are a studio agent operating within team \"{team_name}\" (id: {team_id}).\n\
                     Supervisor: {supervisor}\n\
                     Team members: {members}\n\
                     Policy: max_depth={max_depth}, budget_tokens={budget}, timeout={timeout}s, \
                     loop_detection={loop_det}, quality_threshold={quality}, require_verification={verify}\n\n\
                     You have full access to all Zeus tools. Execute the user's request using the \
                     team's capabilities. Coordinate with team members when appropriate.",
                    team_name = team.name,
                    team_id = team.id,
                    max_depth = team.policy.max_depth,
                    budget = team.policy.budget_tokens,
                    timeout = team.policy.timeout_seconds,
                    loop_det = team.policy.loop_detection,
                    quality = team.policy.quality_threshold,
                    verify = team.policy.require_verification,
                )
            }
            Err(e) => {
                let _ = send_server_msg(
                    sender,
                    &ServerMessage::Error {
                        message: format!("Team '{}' not found: {}", team_id, e),
                    },
                )
                .await;
                return;
            }
        }
    };

    // 2. Spawn a dynamic agent via the registry
    let agent_id = format!("studio-{}", uuid::Uuid::new_v4());
    {
        let mut state_guard = state.write().await;
        if let Err(e) = state_guard
            .agent_registry
            .spawn_dynamic(
                &agent_id,
                &format!("Studio ({})", team_id),
                Some(team_context),
            )
            .await
        {
            let _ = send_server_msg(
                sender,
                &ServerMessage::Error {
                    message: format!("Failed to spawn studio agent: {}", e),
                },
            )
            .await;
            return;
        }
        // Track compute quota for the ephemeral studio agent (default priority).
        state_guard.register_agent_compute(&agent_id, 1.0).await;
    }

    // 3. Get the agent Arc, attach event channel
    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(256);
    let agent_arc = {
        let mut state_guard = state.write().await;
        state_guard.agent_registry.update_activity(&agent_id);
        match state_guard.agent_registry.get(&agent_id) {
            Some(instance) => instance.agent.clone(),
            None => {
                let _ = send_server_msg(
                    sender,
                    &ServerMessage::Error {
                        message: format!("Studio agent '{}' disappeared after spawn", agent_id),
                    },
                )
                .await;
                return;
            }
        }
    };

    // Set events on the agent while we hold the write lock
    {
        let mut agent = agent_arc.write().await;
        agent.set_events(event_tx);
    }

    // 4. Run the message in a background task
    let msg = message.to_string();
    let agent_arc_clone = agent_arc.clone();
    let agent_handle = tokio::spawn(async move {
        let mut agent = agent_arc_clone.write().await;
        agent.run(&msg).await
    });

    // 5. Stream events to WebSocket
    stream_agent_events(sender, &mut event_rx).await;

    // 6. Wait for agent to finish
    match agent_handle.await {
        Ok(Ok(_response)) => {
            let _ = send_server_msg(
                sender,
                &ServerMessage::StudioEvent {
                    event_type: "studio_complete".to_string(),
                    data: serde_json::json!({"agent_id": agent_id, "team_id": team_id}),
                },
            )
            .await;
        }
        Ok(Err(e)) => {
            let _ = send_server_msg(
                sender,
                &ServerMessage::Error {
                    message: e.to_string(),
                },
            )
            .await;
        }
        Err(e) => {
            error!("Studio team agent panicked: {}", e);
            let _ = send_server_msg(
                sender,
                &ServerMessage::Error {
                    message: "Internal error".to_string(),
                },
            )
            .await;
        }
    }

    // 7. Clean up the ephemeral agent + release its compute quota.
    {
        let mut state_guard = state.write().await;
        state_guard.agent_registry.unregister(&agent_id);
        state_guard.deregister_agent_compute(&agent_id).await;
    }
}

/// Studio system prompt used for all studio agents (team and default).
const STUDIO_PROMPT: &str = concat!(
    "You are Zeus, a powerful AI assistant with full access to all capabilities listed below.\n",
    "You can do ANYTHING the user asks by using the appropriate action.\n\n",
    "## Available Capabilities\n\n",
    "Use <action type=\"ACTION_TYPE\"> tags to trigger these tools:\n\n",
    "### Creative\n",
    "- **generate_image** — Generate images from text descriptions. ",
    "When the user asks you to create, draw, generate, or make an image/picture/photo, ",
    "use <action type=\"generate_image\">{\"prompt\": \"...\", \"style\": \"...\", \"width\": 1024, \"height\": 1024}</action>\n\n",
    "### Search & Knowledge\n",
    "- **search_web** — Search the internet for current information\n",
    "- **read_memory** — Read from Zeus memory/knowledge base\n",
    "- **write_memory** — Store facts and notes to memory\n",
    "- **search_memory** — Search stored memories semantically\n\n",
    "### Agent Management\n",
    "- **create_agent** — Create a new agent with specific capabilities\n",
    "- **list_agents** — List all registered agents\n",
    "- **delete_agent** — Remove an agent\n",
    "- **create_team** — Create an agent team for collaborative work\n",
    "- **manage_teams** — Manage existing agent teams and delegations\n\n",
    "### Projects\n",
    "- **create_project** — Create a new project\n",
    "- **list_projects** — List all projects\n",
    "- **manage_projects** — Update, assign agents, or delete projects\n\n",
    "### Channels & Communication\n",
    "- **manage_channels** — Configure Telegram, Discord, Slack, Email, and other channels\n",
    "- **send_message** — Send messages through configured channels\n\n",
    "### Tools & Execution\n",
    "- **list_tools** — List all available tools\n",
    "- **execute_tool** — Execute any registered tool by name\n",
    "- **shell** — Run shell commands\n",
    "- **read_file** / **write_file** / **edit_file** — File operations\n\n",
    "### Sessions\n",
    "- **list_sessions** — List conversation sessions\n",
    "- **manage_sessions** — Create, delete, or review sessions\n\n",
    "### System\n",
    "- **update_config** — Update Zeus configuration\n",
    "- **get_status** — Get system status and health\n\n",
    "## Important Rules\n",
    "1. NEVER say you cannot generate images — you CAN via the generate_image action\n",
    "2. NEVER say you cannot search the web — you CAN via search_web\n",
    "3. When a user asks for something creative (image, picture, art), use generate_image\n",
    "4. Respond naturally and conversationally while using actions as needed\n",
    "5. Always confirm what action you took and show results\n"
);

/// Default studio behavior (no team): spawn a studio agent via the registry
/// with the full orchestration prompt, stream events, then clean up.
async fn handle_studio_default(
    sender: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    state: &SharedState,
    message: &str,
    _session_id: Option<&str>,
) {
    // 1. Spawn a dynamic agent via the registry with the studio prompt
    let agent_id = format!("studio-{}", uuid::Uuid::new_v4());
    {
        let mut state_guard = state.write().await;
        if let Err(e) = state_guard
            .agent_registry
            .spawn_dynamic(
                &agent_id,
                "Studio (default)",
                Some(STUDIO_PROMPT.to_string()),
            )
            .await
        {
            let _ = send_server_msg(
                sender,
                &ServerMessage::Error {
                    message: format!("Failed to spawn studio agent: {}", e),
                },
            )
            .await;
            return;
        }
        // Track compute quota for the ephemeral studio agent (default priority).
        state_guard.register_agent_compute(&agent_id, 1.0).await;
    }

    // 2. Get the agent Arc, attach event channel
    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(256);
    let agent_arc = {
        let mut state_guard = state.write().await;
        state_guard.agent_registry.update_activity(&agent_id);
        match state_guard.agent_registry.get(&agent_id) {
            Some(instance) => instance.agent.clone(),
            None => {
                let _ = send_server_msg(
                    sender,
                    &ServerMessage::Error {
                        message: format!("Studio agent '{}' disappeared after spawn", agent_id),
                    },
                )
                .await;
                return;
            }
        }
    };

    // Set events on the agent
    {
        let mut agent = agent_arc.write().await;
        agent.set_events(event_tx);
    }

    // 3. Run the message in a background task
    let msg = message.to_string();
    let agent_arc_clone = agent_arc.clone();
    let agent_handle = tokio::spawn(async move {
        let mut agent = agent_arc_clone.write().await;
        agent.run(&msg).await
    });

    // 4. Stream events to WebSocket
    stream_agent_events(sender, &mut event_rx).await;

    // 5. Wait for agent to finish
    match agent_handle.await {
        Ok(Ok(_response)) => {
            let _ = send_server_msg(
                sender,
                &ServerMessage::StudioEvent {
                    event_type: "studio_complete".to_string(),
                    data: serde_json::json!({"agent_id": agent_id}),
                },
            )
            .await;
        }
        Ok(Err(e)) => {
            let _ = send_server_msg(
                sender,
                &ServerMessage::Error {
                    message: e.to_string(),
                },
            )
            .await;
        }
        Err(e) => {
            error!("Studio agent task panicked: {}", e);
            let _ = send_server_msg(
                sender,
                &ServerMessage::Error {
                    message: "Internal error".to_string(),
                },
            )
            .await;
        }
    }

    // 6. Clean up the ephemeral agent + release its compute quota.
    {
        let mut state_guard = state.write().await;
        state_guard.agent_registry.unregister(&agent_id);
        state_guard.deregister_agent_compute(&agent_id).await;
    }
}

/// Stream agent events to WebSocket, emitting extra StudioEvent for orchestration tool calls.
async fn stream_agent_events(
    sender: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    event_rx: &mut mpsc::Receiver<AgentEvent>,
) {
    while let Some(event) = event_rx.recv().await {
        let server_msg = match event {
            AgentEvent::Started => ServerMessage::Started,
            AgentEvent::TextChunk(chunk) => ServerMessage::TextChunk { chunk },
            AgentEvent::ToolCall { name, args } => {
                // Also emit studio events for orchestration-related tool calls
                if name.starts_with("team_")
                    || name.starts_with("agent_")
                    || name.starts_with("delegate")
                {
                    let _ = send_server_msg(
                        sender,
                        &ServerMessage::StudioEvent {
                            event_type: format!("orchestration_{}", name),
                            data: args.clone(),
                        },
                    )
                    .await;
                }
                ServerMessage::ToolCall { name, args }
            }
            AgentEvent::ToolResult {
                name,
                success,
                output,
            } => ServerMessage::ToolResult {
                name,
                success,
                output,
            },
            AgentEvent::ResponseComplete(resp) => ServerMessage::ResponseComplete {
                content: resp.content,
                session_id: None,
            },
            AgentEvent::Finished { iterations } => ServerMessage::Finished {
                iterations: iterations as u32,
            },
            AgentEvent::Error(msg) => ServerMessage::Error { message: msg },
            AgentEvent::Compacted { messages_removed, tokens_before, tokens_after } => {
                ServerMessage::Compacted { messages_removed, tokens_before, tokens_after }
            }
            AgentEvent::OAuthComplete(_) => continue,
        };

        if send_server_msg(sender, &server_msg).await.is_err() {
            break; // Client disconnected
        }
    }
}

/// Process a workflow message: use LLM planner to decompose, then execute as DAG.
async fn handle_workflow(
    sender: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    state: &SharedState,
    message: &str,
    mode: Option<&str>,
    watched_plans: &Arc<RwLock<HashSet<String>>>,
) {
    let state_guard = state.read().await;

    // 1. Build LLM client (required for planning)
    let llm = match LlmClient::from_config(&state_guard.config) {
        Ok(l) => l,
        Err(e) => {
            let _ = send_server_msg(
                sender,
                &ServerMessage::Error {
                    message: format!("LLM required for workflow planning: {}", e),
                },
            )
            .await;
            return;
        }
    };

    // 2. Get tool schemas for the planner
    let tool_schemas = state_guard.tools.schemas();

    // 3. Call Planner to decompose message into a Plan
    let planner = zeus_prometheus::planner::Planner::new();
    let plan = match planner.create_plan(message, &llm, &tool_schemas).await {
        Ok(p) => p,
        Err(e) => {
            let _ = send_server_msg(
                sender,
                &ServerMessage::Error {
                    message: format!("Planning failed: {}", e),
                },
            )
            .await;
            return;
        }
    };

    let goal = plan.task.clone();

    // 4. Analyze plan into a TaskDAG
    let dag = match state_guard.strategic_planner().analyze(&plan) {
        Ok(d) => d,
        Err(e) => {
            let _ = send_server_msg(
                sender,
                &ServerMessage::Error {
                    message: format!("DAG analysis failed: {}", e),
                },
            )
            .await;
            return;
        }
    };

    // 5. Generate workflow ID and extract DAG metadata
    let workflow_id = format!("wf-{}", uuid::Uuid::new_v4());
    let parallel_groups = dag.parallel_groups();
    let critical_path = dag.critical_path();
    let estimated_total_ms = dag.estimated_total_ms();

    // 6. Determine execution mode
    let step_delay_ms = 200u64;
    let mode_str = mode.unwrap_or("agent");
    let exec_mode = if mode_str == "simulate" {
        crate::handlers::ExecutionMode::Simulated { step_delay_ms }
    } else if mode_str == "llm" {
        crate::handlers::ExecutionMode::Llm(Box::new(state_guard.config.clone()))
    } else {
        crate::handlers::ExecutionMode::Agent(state.clone())
    };
    let mode_label = match &exec_mode {
        crate::handlers::ExecutionMode::Agent(_) => "agent",
        crate::handlers::ExecutionMode::Llm(_) => "llm",
        crate::handlers::ExecutionMode::Simulated { .. } => "simulated",
    };

    // 7. Build step summaries for the WorkflowCreated message
    let steps: Vec<Value> = dag
        .nodes
        .iter()
        .map(|(id, node)| {
            serde_json::json!({
                "id": id,
                "description": node.description,
                "tool": node.tool,
                "status": format!("{:?}", node.status),
            })
        })
        .collect();

    let broadcast = state_guard.plan_broadcast.clone();
    drop(state_guard);

    // 8. Send WorkflowCreated to client
    let _ = send_server_msg(
        sender,
        &ServerMessage::WorkflowCreated {
            workflow_id: workflow_id.clone(),
            goal: goal.clone(),
            steps,
            parallel_groups,
            critical_path,
            estimated_total_ms,
            mode: mode_label.to_string(),
        },
    )
    .await;

    // 9. Auto-subscribe this client to the workflow's plan events
    {
        let mut plans = watched_plans.write().await;
        plans.insert(workflow_id.clone());
    }

    // 10. Spawn background DAG executor
    let exec_id = workflow_id.clone();
    let exec_goal = goal.clone();
    tokio::spawn(async move {
        crate::handlers::execute_plan_steps(dag, &exec_id, &exec_goal, exec_mode, &broadcast).await;
    });

    info!("Workflow {} created from chat: {}", workflow_id, message);
}

/// Send a server message as JSON text.
async fn send_server_msg(
    sender: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    msg: &ServerMessage,
) -> Result<(), ()> {
    let json = serde_json::to_string(msg).unwrap_or_default();
    sender.send(Message::Text(json)).await.map_err(|e| {
        warn!("WebSocket send error: {}", e);
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_client_message_parse_chat() {
        let json = r#"{"type":"chat","message":"hello"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::Chat { .. }));
    }

    #[test]
    fn test_client_message_parse_chat_with_session() {
        let json = r#"{"type":"chat","message":"hello","session_id":"abc123"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Chat {
                message,
                session_id,
            } => {
                assert_eq!(message, "hello");
                assert_eq!(session_id.unwrap(), "abc123");
            }
            _ => panic!("Expected Chat"),
        }
    }

    #[test]
    fn test_client_message_parse_studio() {
        let json = r#"{"type":"studio","message":"create team alpha"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Studio {
                message,
                session_id,
                team_id,
            } => {
                assert_eq!(message, "create team alpha");
                assert!(session_id.is_none());
                assert!(team_id.is_none());
            }
            _ => panic!("Expected Studio"),
        }
    }

    #[test]
    fn test_client_message_parse_studio_with_session() {
        let json = r#"{"type":"studio","message":"delegate task","session_id":"s1"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Studio {
                message,
                session_id,
                team_id,
            } => {
                assert_eq!(message, "delegate task");
                assert_eq!(session_id.unwrap(), "s1");
                assert!(team_id.is_none());
            }
            _ => panic!("Expected Studio"),
        }
    }

    #[test]
    fn test_client_message_parse_studio_with_team_id() {
        let json = r#"{"type":"studio","message":"run analysis","team_id":"team-abc"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Studio {
                message,
                session_id,
                team_id,
            } => {
                assert_eq!(message, "run analysis");
                assert!(session_id.is_none());
                assert_eq!(team_id.unwrap(), "team-abc");
            }
            _ => panic!("Expected Studio"),
        }
    }

    #[test]
    fn test_client_message_parse_studio_with_all_fields() {
        let json = r#"{"type":"studio","message":"deploy","session_id":"s2","team_id":"team-xyz"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Studio {
                message,
                session_id,
                team_id,
            } => {
                assert_eq!(message, "deploy");
                assert_eq!(session_id.unwrap(), "s2");
                assert_eq!(team_id.unwrap(), "team-xyz");
            }
            _ => panic!("Expected Studio"),
        }
    }

    #[test]
    fn test_client_message_parse_ping() {
        let json = r#"{"type":"ping"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::Ping));
    }

    #[test]
    fn test_server_message_serialize_started() {
        let msg = ServerMessage::Started;
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"started"}"#);
    }

    #[test]
    fn test_server_message_serialize_text_chunk() {
        let msg = ServerMessage::TextChunk {
            chunk: "Hello".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "text_chunk");
        assert_eq!(parsed["chunk"], "Hello");
    }

    #[test]
    fn test_server_message_serialize_tool_call() {
        let msg = ServerMessage::ToolCall {
            name: "shell".to_string(),
            args: json!({"command": "ls"}),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "tool_call");
        assert_eq!(parsed["name"], "shell");
        assert_eq!(parsed["args"]["command"], "ls");
    }

    #[test]
    fn test_server_message_serialize_pong() {
        let msg = ServerMessage::Pong;
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"pong"}"#);
    }

    #[test]
    fn test_server_message_serialize_error() {
        let msg = ServerMessage::Error {
            message: "something broke".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "error");
        assert_eq!(parsed["message"], "something broke");
    }

    #[test]
    fn test_server_message_serialize_finished() {
        let msg = ServerMessage::Finished { iterations: 3 };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "finished");
        assert_eq!(parsed["iterations"], 3);
    }

    #[test]
    fn test_server_message_serialize_studio_event() {
        let msg = ServerMessage::StudioEvent {
            event_type: "team_created".to_string(),
            data: json!({"team_id": "t1", "name": "alpha"}),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "studio_event");
        assert_eq!(parsed["event_type"], "team_created");
        assert_eq!(parsed["data"]["name"], "alpha");
    }

    // ========================================================================
    // Prometheus WebSocket protocol tests
    // ========================================================================

    #[test]
    fn test_client_message_parse_prometheus_watch() {
        let json = r#"{"type":"prometheus_watch","plan_id":"plan-abc123"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::PrometheusWatch { plan_id } => {
                assert_eq!(plan_id, "plan-abc123");
            }
            _ => panic!("Expected PrometheusWatch"),
        }
    }

    #[test]
    fn test_server_message_serialize_prometheus_update() {
        let msg = ServerMessage::PrometheusUpdate {
            plan_id: "plan-1".to_string(),
            step_id: 2,
            status: "completed".to_string(),
            progress_pct: 66.7,
            output: "Step 2 done".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "prometheus_update");
        assert_eq!(parsed["plan_id"], "plan-1");
        assert_eq!(parsed["step_id"], 2);
        assert_eq!(parsed["status"], "completed");
        assert_eq!(parsed["progress_pct"], 66.7);
        assert_eq!(parsed["output"], "Step 2 done");
    }

    #[test]
    fn test_server_message_serialize_prometheus_complete() {
        let msg = ServerMessage::PrometheusComplete {
            plan_id: "plan-1".to_string(),
            status: "completed".to_string(),
            steps_completed: 5,
            steps_failed: 0,
            duration_ms: 1234,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "prometheus_complete");
        assert_eq!(parsed["plan_id"], "plan-1");
        assert_eq!(parsed["status"], "completed");
        assert_eq!(parsed["steps_completed"], 5);
        assert_eq!(parsed["steps_failed"], 0);
        assert_eq!(parsed["duration_ms"], 1234);
    }

    #[test]
    fn test_plan_step_update_serde() {
        let update = PlanStepUpdate {
            plan_id: "p1".to_string(),
            step_id: 0,
            status: "running".to_string(),
            progress_pct: 25.0,
            output: "Working...".to_string(),
        };
        let json = serde_json::to_string(&update).unwrap();
        let parsed: PlanStepUpdate = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.plan_id, "p1");
        assert_eq!(parsed.step_id, 0);
        assert_eq!(parsed.status, "running");
        assert_eq!(parsed.progress_pct, 25.0);
        assert_eq!(parsed.output, "Working...");
    }

    #[test]
    fn test_plan_complete_serde() {
        let complete = PlanComplete {
            plan_id: "p2".to_string(),
            status: "partial".to_string(),
            steps_completed: 3,
            steps_failed: 1,
            duration_ms: 5678,
        };
        let json = serde_json::to_string(&complete).unwrap();
        let parsed: PlanComplete = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.plan_id, "p2");
        assert_eq!(parsed.status, "partial");
        assert_eq!(parsed.steps_completed, 3);
        assert_eq!(parsed.steps_failed, 1);
        assert_eq!(parsed.duration_ms, 5678);
    }

    #[test]
    fn test_plan_broadcast_send_and_subscribe() {
        let broadcast = PlanBroadcast::new(16);
        let mut rx = broadcast.subscribe();

        broadcast.send_step_update(PlanStepUpdate {
            plan_id: "p1".to_string(),
            step_id: 0,
            status: "completed".to_string(),
            progress_pct: 50.0,
            output: "Done".to_string(),
        });

        let event = rx.try_recv().unwrap();
        match event {
            PlanEvent::StepUpdate(u) => {
                assert_eq!(u.plan_id, "p1");
                assert_eq!(u.step_id, 0);
                assert_eq!(u.progress_pct, 50.0);
            }
            _ => panic!("Expected StepUpdate"),
        }
    }

    #[test]
    fn test_plan_broadcast_complete_event() {
        let broadcast = PlanBroadcast::new(16);
        let mut rx = broadcast.subscribe();

        broadcast.send_complete(PlanComplete {
            plan_id: "p1".to_string(),
            status: "completed".to_string(),
            steps_completed: 2,
            steps_failed: 0,
            duration_ms: 100,
        });

        let event = rx.try_recv().unwrap();
        match event {
            PlanEvent::Complete(c) => {
                assert_eq!(c.plan_id, "p1");
                assert_eq!(c.steps_completed, 2);
                assert_eq!(c.duration_ms, 100);
            }
            _ => panic!("Expected Complete"),
        }
    }

    #[test]
    fn test_plan_broadcast_no_receivers_doesnt_panic() {
        let broadcast = PlanBroadcast::new(16);
        // No subscribers — send should silently drop
        broadcast.send_step_update(PlanStepUpdate {
            plan_id: "p1".to_string(),
            step_id: 0,
            status: "completed".to_string(),
            progress_pct: 100.0,
            output: "ok".to_string(),
        });
        broadcast.send_complete(PlanComplete {
            plan_id: "p1".to_string(),
            status: "completed".to_string(),
            steps_completed: 1,
            steps_failed: 0,
            duration_ms: 0,
        });
        // If we get here without panic, the test passes
    }

    #[test]
    fn test_plan_broadcast_multiple_subscribers() {
        let broadcast = PlanBroadcast::new(16);
        let mut rx1 = broadcast.subscribe();
        let mut rx2 = broadcast.subscribe();

        broadcast.send_step_update(PlanStepUpdate {
            plan_id: "shared".to_string(),
            step_id: 1,
            status: "completed".to_string(),
            progress_pct: 100.0,
            output: "done".to_string(),
        });

        // Both subscribers should receive the same event
        let e1 = rx1.try_recv().unwrap();
        let e2 = rx2.try_recv().unwrap();
        match (e1, e2) {
            (PlanEvent::StepUpdate(a), PlanEvent::StepUpdate(b)) => {
                assert_eq!(a.plan_id, "shared");
                assert_eq!(b.plan_id, "shared");
                assert_eq!(a.step_id, b.step_id);
            }
            _ => panic!("Expected StepUpdate on both"),
        }
    }

    #[test]
    fn test_prometheus_watch_invalid_missing_plan_id() {
        let json = r#"{"type":"prometheus_watch"}"#;
        let result: Result<ClientMessage, _> = serde_json::from_str(json);
        assert!(result.is_err(), "Should fail without plan_id");
    }

    #[test]
    fn test_prometheus_update_partial_status() {
        let msg = ServerMessage::PrometheusComplete {
            plan_id: "p1".to_string(),
            status: "partial".to_string(),
            steps_completed: 2,
            steps_failed: 1,
            duration_ms: 999,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["status"], "partial");
        assert_eq!(parsed["steps_failed"], 1);
    }

    // ========================================================================
    // Workflow WebSocket protocol tests
    // ========================================================================

    #[test]
    fn test_client_message_parse_workflow() {
        let json = r#"{"type":"workflow","message":"Build and deploy my app"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Workflow {
                message,
                session_id,
                mode,
            } => {
                assert_eq!(message, "Build and deploy my app");
                assert!(session_id.is_none());
                assert!(mode.is_none());
            }
            _ => panic!("Expected Workflow"),
        }
    }

    #[test]
    fn test_client_message_parse_workflow_with_mode() {
        let json = r#"{"type":"workflow","message":"List files","mode":"simulate"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Workflow { message, mode, .. } => {
                assert_eq!(message, "List files");
                assert_eq!(mode.unwrap(), "simulate");
            }
            _ => panic!("Expected Workflow"),
        }
    }

    #[test]
    fn test_client_message_parse_workflow_with_session() {
        let json = r#"{"type":"workflow","message":"deploy","session_id":"s1","mode":"agent"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::Workflow {
                message,
                session_id,
                mode,
            } => {
                assert_eq!(message, "deploy");
                assert_eq!(session_id.unwrap(), "s1");
                assert_eq!(mode.unwrap(), "agent");
            }
            _ => panic!("Expected Workflow"),
        }
    }

    #[test]
    fn test_client_message_parse_workflow_missing_message() {
        let json = r#"{"type":"workflow"}"#;
        let result: Result<ClientMessage, _> = serde_json::from_str(json);
        assert!(result.is_err(), "Should fail without message field");
    }

    #[test]
    fn test_server_message_serialize_workflow_created() {
        let msg = ServerMessage::WorkflowCreated {
            workflow_id: "wf-123".to_string(),
            goal: "Deploy app".to_string(),
            steps: vec![json!({"id": 0, "description": "Build", "tool": "shell"})],
            parallel_groups: vec![vec![0]],
            critical_path: vec![0],
            estimated_total_ms: 5000,
            mode: "agent".to_string(),
        };
        let json_str = serde_json::to_string(&msg).unwrap();
        let parsed: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["type"], "workflow_created");
        assert_eq!(parsed["workflow_id"], "wf-123");
        assert_eq!(parsed["goal"], "Deploy app");
        assert_eq!(parsed["steps"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["parallel_groups"][0][0], 0);
        assert_eq!(parsed["critical_path"][0], 0);
        assert_eq!(parsed["estimated_total_ms"], 5000);
        assert_eq!(parsed["mode"], "agent");
    }

    #[test]
    fn test_workflow_created_empty_steps() {
        let msg = ServerMessage::WorkflowCreated {
            workflow_id: "wf-empty".to_string(),
            goal: "Nothing".to_string(),
            steps: vec![],
            parallel_groups: vec![],
            critical_path: vec![],
            estimated_total_ms: 0,
            mode: "simulated".to_string(),
        };
        let json_str = serde_json::to_string(&msg).unwrap();
        let parsed: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["type"], "workflow_created");
        assert!(parsed["steps"].as_array().unwrap().is_empty());
        assert_eq!(parsed["estimated_total_ms"], 0);
    }

    #[test]
    fn test_workflow_uses_plan_broadcast() {
        // Verify that workflow_id format works with PlanBroadcast (same channel)
        let broadcast = PlanBroadcast::new(16);
        let mut rx = broadcast.subscribe();

        let workflow_id = "wf-test-123".to_string();
        broadcast.send_step_update(PlanStepUpdate {
            plan_id: workflow_id.clone(),
            step_id: 0,
            status: "running".to_string(),
            progress_pct: 0.0,
            output: "Starting".to_string(),
        });

        let event = rx.try_recv().unwrap();
        match event {
            PlanEvent::StepUpdate(u) => {
                assert_eq!(u.plan_id, "wf-test-123");
                assert!(u.plan_id.starts_with("wf-"));
            }
            _ => panic!("Expected StepUpdate"),
        }

        broadcast.send_complete(PlanComplete {
            plan_id: workflow_id,
            status: "completed".to_string(),
            steps_completed: 1,
            steps_failed: 0,
            duration_ms: 100,
        });

        let event = rx.try_recv().unwrap();
        match event {
            PlanEvent::Complete(c) => {
                assert_eq!(c.plan_id, "wf-test-123");
                assert_eq!(c.steps_completed, 1);
            }
            _ => panic!("Expected Complete"),
        }
    }

    #[test]
    fn test_client_message_parse_cancel_stream() {
        let json = r#"{"type":"cancel_stream"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::CancelStream));
    }

    #[test]
    fn test_server_message_stream_cancelled_serializes() {
        let msg = ServerMessage::StreamCancelled;
        let json_str = serde_json::to_string(&msg).unwrap();
        let parsed: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["type"], "stream_cancelled");
    }

    #[test]
    fn test_cancel_stream_roundtrip() {
        // Verify cancel_stream can be parsed from client and serialized as server response
        let client_json = r#"{"type":"cancel_stream"}"#;
        let client_msg: ClientMessage = serde_json::from_str(client_json).unwrap();
        assert!(matches!(client_msg, ClientMessage::CancelStream));

        let server_msg = ServerMessage::StreamCancelled;
        let server_json = serde_json::to_string(&server_msg).unwrap();
        let parsed: Value = serde_json::from_str(&server_json).unwrap();
        assert_eq!(parsed["type"], "stream_cancelled");
    }
}

// ═══════════════════════════════════════════════════
// S63: Office Broadcast — streams channel messages to Office UI
// ═══════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfficeMessage {
    pub sender_id: String,
    pub sender_name: String,
    pub channel_type: String,
    pub content: String,
    pub timestamp: String,
}

pub struct OfficeBroadcast {
    tx: broadcast::Sender<OfficeMessage>,
}

impl OfficeBroadcast {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn send(&self, msg: OfficeMessage) {
        let _ = self.tx.send(msg);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<OfficeMessage> {
        self.tx.subscribe()
    }
}
