//! Agent Studio handlers (Phase 5 — Super Cursor)

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde_json::{Value, json};

use crate::SharedState;
use crate::handlers::studio_store;
use crate::handlers::pantheon;

// ============================================================================
// Agent Studio (Phase 5 — Super Cursor)
// ============================================================================

/// POST /v1/studio/sessions — create a new studio session
pub async fn studio_create_session(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let goal = body
        .get("goal")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'goal'".to_string()))?;

    let id = format!(
        "studio-{}",
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("x")
    );
    let user_id = body
        .get("user_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let session = studio_store::StudioSessionRow {
        id: id.clone(),
        user_id: user_id.to_string(),
        goal: goal.to_string(),
        status: "idle".to_string(),
        plan_id: None,
        room_id: body
            .get("room_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        agent_id: None,
        session_id: None,
        total_actions: 0,
        completed_actions: 0,
        failed_actions: 0,
        error_message: String::new(),
        metadata_json: body
            .get("metadata")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "{}".to_string()),
        created_at: String::new(),
        updated_at: String::new(),
        completed_at: None,
    };

    let state_guard = state.read().await;
    if state_guard.studio_store.create_session(&session).await {
        // Broadcast studio session creation to War Room observers
        state_guard
            .studio_broadcast
            .send(crate::websocket::StudioEvent::StatusChanged {
                session_id: id.clone(),
                status: "idle".to_string(),
                goal: goal.to_string(),
            });
        // If linked to a War Room, inject a room message
        if let Some(ref room_id) = session.room_id {
            let msg = pantheon::RoomMessage {
                id: format!("rmsg-{}", uuid::Uuid::new_v4()),
                room_id: room_id.clone(),
                sender_id: "system".to_string(),
                sender_name: "Studio".to_string(),
                content: format!("Studio session started: {}", goal),
                message_type: "system".to_string(),
                metadata: Some(serde_json::json!({"studio_session_id": id})),
                reply_to: None,
                edited: false,
                attachments: vec![],
                timestamp: chrono::Utc::now(),
            };
            state_guard.pantheon.insert_room_message(&msg).await;
        }
        Ok((
            StatusCode::CREATED,
            Json(json!({
                "id": id,
                "status": "created",
                "session": session,
            })),
        ))
    } else {
        Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create studio session".to_string(),
        ))
    }
}

/// GET /v1/studio/sessions — list studio sessions
pub async fn studio_list_sessions(
    State(state): State<SharedState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let user_id = params
        .get("user_id")
        .map(|s| s.as_str())
        .unwrap_or("default");
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(20);
    let state_guard = state.read().await;
    let sessions = state_guard.studio_store.list_sessions(user_id, limit).await;
    Json(json!({ "sessions": sessions, "total": sessions.len() }))
}

/// GET /v1/studio/sessions/active — list active studio sessions
pub async fn studio_active_sessions(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let sessions = state_guard.studio_store.list_active_sessions().await;
    Json(json!({ "sessions": sessions, "total": sessions.len() }))
}

/// GET /v1/studio/sessions/:id — get studio session state + action log
pub async fn studio_get_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let session = state_guard
        .studio_store
        .get_session(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Session {} not found", id)))?;
    let actions = state_guard.studio_store.all_actions(&id).await;
    let artifacts = state_guard.studio_store.get_artifacts(&id).await;
    Ok(Json(json!({
        "session": session,
        "actions": actions,
        "artifacts": artifacts,
    })))
}

/// DELETE /v1/studio/sessions/:id — end and delete a session
pub async fn studio_delete_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    if state_guard.studio_store.delete_session(&id).await {
        Ok(Json(json!({ "status": "deleted", "id": id })))
    } else {
        Err((StatusCode::NOT_FOUND, format!("Session {} not found", id)))
    }
}

/// POST /v1/studio/sessions/:id/pause — pause autopilot
pub async fn studio_pause(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let session = state_guard
        .studio_store
        .get_session(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Session {} not found", id)))?;

    if session.status != "driving" {
        return Err((
            StatusCode::CONFLICT,
            format!("Session is {}, can only pause when driving", session.status),
        ));
    }

    state_guard
        .studio_store
        .update_session_status(&id, "paused", None)
        .await;
    state_guard.agent_director.pause_session(&id).await;
    state_guard
        .studio_broadcast
        .send(crate::websocket::StudioEvent::StatusChanged {
            session_id: id.clone(),
            status: "paused".to_string(),
            goal: session.goal.clone(),
        });
    Ok(Json(json!({ "id": id, "status": "paused" })))
}

/// POST /v1/studio/sessions/:id/resume — resume autopilot
pub async fn studio_resume(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let session = state_guard
        .studio_store
        .get_session(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Session {} not found", id)))?;

    if session.status != "paused" {
        return Err((
            StatusCode::CONFLICT,
            format!("Session is {}, can only resume when paused", session.status),
        ));
    }

    state_guard
        .studio_store
        .update_session_status(&id, "driving", None)
        .await;
    state_guard.agent_director.resume_session(&id).await;
    state_guard
        .studio_broadcast
        .send(crate::websocket::StudioEvent::StatusChanged {
            session_id: id.clone(),
            status: "driving".to_string(),
            goal: session.goal.clone(),
        });
    Ok(Json(json!({ "id": id, "status": "driving" })))
}

/// POST /v1/studio/sessions/:id/intervene — inject user instruction mid-run
pub async fn studio_intervene(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let message = body
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'message'".to_string()))?;

    let state_guard = state.read().await;
    let session = state_guard
        .studio_store
        .get_session(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Session {} not found", id)))?;

    if !["driving", "paused", "awaiting_approval"].contains(&session.status.as_str()) {
        return Err((
            StatusCode::CONFLICT,
            format!("Session is {}, cannot intervene", session.status),
        ));
    }

    // Queue the intervention as a special action
    let action = studio_store::StudioActionRow {
        id: format!(
            "act-{}",
            uuid::Uuid::new_v4()
                .to_string()
                .split('-')
                .next()
                .unwrap_or("x")
        ),
        session_id: id.clone(),
        action_type: "intervene".to_string(),
        target: String::new(),
        value: message.to_string(),
        description: format!("User intervention: {}", &message[..zeus_core::floor_char_boundary(message, 80)]),
        delay_ms: 0,
        status: "pending".to_string(),
        error_message: String::new(),
        sequence_num: session.total_actions + 1,
        elapsed_ms: 0,
        created_at: String::new(),
        executed_at: None,
    };
    state_guard.studio_store.queue_action(&action).await;

    Ok(Json(
        json!({ "id": id, "status": "intervention_queued", "action_id": action.id }),
    ))
}

/// GET /v1/studio/sessions/:id/replay — full action log for playback
pub async fn studio_replay(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let session = state_guard
        .studio_store
        .get_session(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Session {} not found", id)))?;
    let actions = state_guard.studio_store.all_actions(&id).await;
    let artifacts = state_guard.studio_store.get_artifacts(&id).await;

    Ok(Json(json!({
        "session_id": id,
        "goal": session.goal,
        "status": session.status,
        "total_actions": actions.len(),
        "actions": actions,
        "artifacts": artifacts,
        "created_at": session.created_at,
        "completed_at": session.completed_at,
    })))
}

/// GET /v1/studio/stats — studio overview stats
pub async fn studio_stats(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let stats = state_guard.studio_store.stats().await;
    Json(serde_json::to_value(stats).unwrap_or_default())
}

/// POST /v1/studio/sessions/:id/link-room — bridge Studio session to a War Room
///
/// Links a studio session to a Pantheon room so that:
/// - Studio action events are broadcast to War Room observers
/// - Session lifecycle events (start, complete, fail) appear as room messages
/// - Room members can watch the session live via `StudioWatch` WebSocket
pub async fn studio_link_room(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let room_id = body
        .get("room_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'room_id'".to_string()))?;

    let state_guard = state.read().await;
    let session = state_guard
        .studio_store
        .get_session(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Session {} not found", id)))?;

    // Verify room exists
    let _room = state_guard
        .pantheon
        .get_room(room_id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Room {} not found", room_id)))?;

    // Link the session to the room
    state_guard.studio_store.link_room(&id, room_id).await;

    // Inject a room message announcing the link
    let msg = pantheon::RoomMessage {
        id: format!("rmsg-{}", uuid::Uuid::new_v4()),
        room_id: room_id.to_string(),
        sender_id: "system".to_string(),
        sender_name: "Studio".to_string(),
        content: format!(
            "Studio session `{}` linked to this room. Goal: {}. Watch live with `studio_watch`.",
            id, session.goal
        ),
        message_type: "system".to_string(),
        metadata: Some(serde_json::json!({
            "studio_session_id": id,
            "event": "session_linked",
        })),
        reply_to: None,
        edited: false,
        attachments: vec![],
        timestamp: chrono::Utc::now(),
    };
    state_guard.pantheon.insert_room_message(&msg).await;

    // Broadcast the link event
    state_guard
        .studio_broadcast
        .send(crate::websocket::StudioEvent::StatusChanged {
            session_id: id.clone(),
            status: format!("linked:{}", room_id),
            goal: session.goal,
        });

    Ok(Json(json!({
        "id": id,
        "room_id": room_id,
        "status": "linked",
    })))
}

// ============================================================================
// Studio Drive — Phase 8 Autonomous Driving Loop
// ============================================================================

/// Heuristic step estimation from a goal string for ComplexityAnalyzer.
///
/// Parses the goal for action keywords and produces rough step/tool pairs.
/// Real step planning happens inside DrivingLoop — this just gates the
/// approval flow before driving starts.
pub fn estimate_drive_steps(goal: &str) -> Vec<(String, Option<String>)> {
    let lower = goal.to_lowercase();
    let mut steps = Vec::new();

    // Navigation detection
    if lower.contains("navigate") || lower.contains("go to") || lower.contains("open") {
        steps.push(("Navigate to target page".to_string(), None));
    }

    // Form/input detection
    if lower.contains("type")
        || lower.contains("fill")
        || lower.contains("enter")
        || lower.contains("input")
        || lower.contains("form")
    {
        steps.push(("Fill form fields".to_string(), None));
    }

    // Click/interaction detection
    if lower.contains("click")
        || lower.contains("press")
        || lower.contains("select")
        || lower.contains("submit")
        || lower.contains("button")
    {
        steps.push(("Interact with UI elements".to_string(), None));
    }

    // File/write detection
    if lower.contains("create")
        || lower.contains("write")
        || lower.contains("save")
        || lower.contains("generate")
    {
        steps.push((
            "Create or write content".to_string(),
            Some("write_file".to_string()),
        ));
    }

    // Shell/build detection
    if lower.contains("build")
        || lower.contains("deploy")
        || lower.contains("run")
        || lower.contains("install")
        || lower.contains("execute")
    {
        steps.push((
            "Execute system commands".to_string(),
            Some("shell".to_string()),
        ));
    }

    // Multi-step indicators
    if lower.contains("and then")
        || lower.contains("after that")
        || lower.contains("finally")
        || lower.contains("step")
        || lower.contains("pipeline")
    {
        steps.push((
            "Execute multi-step pipeline".to_string(),
            Some("shell".to_string()),
        ));
    }

    // Design/visual detection
    if lower.contains("design")
        || lower.contains("logo")
        || lower.contains("landing page")
        || lower.contains("layout")
        || lower.contains("mockup")
    {
        steps.push((
            "Design visual elements".to_string(),
            Some("write_file".to_string()),
        ));
        steps.push((
            "Iterate on design".to_string(),
            Some("edit_file".to_string()),
        ));
    }

    // Minimum: always at least one step
    if steps.is_empty() {
        steps.push(("Analyze and execute goal".to_string(), None));
    }

    // Verification step for multi-step goals
    if steps.len() > 2 {
        steps.push(("Verify results".to_string(), None));
    }

    steps
}

/// POST /v1/studio/sessions/:id/drive — Start the autonomous driving loop.
///
/// The DrivingLoop runs in a background task: LLM → plan actions → dispatch
/// via puppet WS → collect results → re-plan → repeat until goal complete.
/// Returns immediately with the session status.
pub async fn studio_drive(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;

    // Load the studio session
    let session = state_guard
        .studio_store
        .get_session(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Session {} not found", id)))?;

    if session.goal.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Session has no goal".to_string()));
    }

    // Check session isn't already driving
    let director = state_guard.agent_director.clone();
    if let Some(status) = director.get_status(&id).await
        && matches!(status, zeus_prometheus::DirectorStatus::Driving)
    {
        return Err((
            StatusCode::CONFLICT,
            "Session is already being driven".to_string(),
        ));
    }

    // Complexity analysis → PlanCard approval gate
    // If the goal is complex, return a PlanCard for user approval instead of driving immediately.
    // Pass `approved: true` or session status `approved` to skip this gate on re-drive.
    let skip_approval = body
        .get("approved")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || session.status == "approved";

    if !skip_approval {
        let analyzer = zeus_nous::ComplexityAnalyzer::new();
        // Estimate steps from goal keywords (heuristic — real step planning happens in DrivingLoop)
        let estimated_steps = estimate_drive_steps(&session.goal);
        let plan_card = analyzer.analyze(
            &format!("studio-plan-{}", id),
            &session.goal,
            &estimated_steps,
            &[], // available tools resolved at drive time
        );

        if plan_card.requires_approval {
            // Store plan card and set session to awaiting_approval
            state_guard
                .studio_store
                .update_session_status(&id, "awaiting_approval", None)
                .await;
            state_guard
                .studio_broadcast
                .send(crate::websocket::StudioEvent::StatusChanged {
                    session_id: id.clone(),
                    status: "awaiting_approval".to_string(),
                    goal: session.goal.clone(),
                });
            return Ok(Json(json!({
                "id": id,
                "status": "awaiting_approval",
                "plan_card": plan_card,
                "message": "Complex goal detected — approve to start driving",
            })));
        }
    }

    // Parse optional config overrides
    let config = {
        let mut c = zeus_prometheus::DrivingConfig::default();
        if let Some(v) = body.get("max_iterations").and_then(|v| v.as_u64()) {
            c.max_iterations = v as u32;
        }
        if let Some(v) = body.get("max_total_actions").and_then(|v| v.as_u64()) {
            c.max_total_actions = v as u32;
        }
        if let Some(v) = body.get("batch_delay_ms").and_then(|v| v.as_u64()) {
            c.batch_delay_ms = v;
        }
        c
    };

    // Create LLM client
    let llm = LlmClient::from_config(&state_guard.config).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("LLM init failed: {}", e),
        )
    })?;

    // Start the director session — gives us the puppet channels
    let (_cmd_rx, _result_tx) = director
        .start_session(id.clone(), session.goal.clone())
        .await;

    // Take the result receiver (can only be taken once — DrivingLoop owns it)
    let mut result_rx = director.take_result_rx(&id).await.ok_or_else(|| {
        (
            StatusCode::CONFLICT,
            "Result receiver already taken (session in use)".to_string(),
        )
    })?;

    // Mark session as driving in the store
    state_guard
        .studio_store
        .update_session_status(&id, "driving", None)
        .await;

    // Broadcast drive started event
    state_guard
        .studio_broadcast
        .send(crate::websocket::StudioEvent::StatusChanged {
            session_id: id.clone(),
            status: "driving".to_string(),
            goal: session.goal.clone(),
        });

    // War Room notification if linked
    if let Some(ref room_id) = session.room_id {
        let msg = pantheon::RoomMessage {
            id: format!("rmsg-{}", uuid::Uuid::new_v4()),
            room_id: room_id.clone(),
            sender_id: "system".to_string(),
            sender_name: "Studio".to_string(),
            content: format!("Driving started for: {}", session.goal),
            message_type: "system".to_string(),
            metadata: Some(serde_json::json!({
                "studio_session_id": id,
                "event": "drive_started",
            })),
            reply_to: None,
            edited: false,
            attachments: vec![],
            timestamp: chrono::Utc::now(),
        };
        state_guard.pantheon.insert_room_message(&msg).await;
    }

    let goal = session.goal.clone();
    let session_id = id.clone();
    let studio_store = state_guard.studio_store.clone();
    let studio_broadcast = state_guard.studio_broadcast.clone();
    let room_id = session.room_id.clone();
    let pantheon = state_guard.pantheon.clone();
    drop(state_guard); // Release read lock before spawning

    // Snapshot values needed for the JSON response (before move into spawn)
    let resp_goal = goal.clone();
    let resp_max_iter = config.max_iterations;
    let resp_max_actions = config.max_total_actions;
    let resp_delay = config.batch_delay_ms;

    // Spawn the driving loop as a background task
    tokio::spawn(async move {
        let driving_loop = zeus_prometheus::DrivingLoop::new(config);

        let result = driving_loop
            .run(&director, &llm, &session_id, &goal, &mut result_rx)
            .await;

        // Persist final state
        let final_status = if result.success { "complete" } else { "failed" };
        let error = if result.success {
            None
        } else {
            Some(result.summary.as_str())
        };
        studio_store
            .update_session_status(&session_id, final_status, error)
            .await;

        // Broadcast completion
        studio_broadcast.send(crate::websocket::StudioEvent::StatusChanged {
            session_id: session_id.clone(),
            status: final_status.to_string(),
            goal: result.summary.clone(),
        });

        // War Room completion message
        if let Some(room_id) = room_id {
            let emoji = if result.success { "✅" } else { "❌" };
            let msg = pantheon::RoomMessage {
                id: format!("rmsg-{}", uuid::Uuid::new_v4()),
                room_id,
                sender_id: "system".to_string(),
                sender_name: "Studio".to_string(),
                content: format!(
                    "{} Drive {}: {} ({} actions, {}ms)",
                    emoji,
                    final_status,
                    result.summary,
                    result.actions_executed,
                    result.duration_ms
                ),
                message_type: "system".to_string(),
                metadata: Some(serde_json::json!({
                    "studio_session_id": session_id,
                    "event": "drive_complete",
                    "result": result,
                })),
                reply_to: None,
                edited: false,
                attachments: vec![],
                timestamp: chrono::Utc::now(),
            };
            pantheon.insert_room_message(&msg).await;
        }

        // Session cleanup — remove from director after 5 minutes (Zeus107 item D)
        let dir = director.clone();
        let sid = session_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;
            dir.remove_session(&sid).await;
            tracing::debug!("Cleaned up director session {} after 5min", sid);
        });

        tracing::info!(
            "DrivingLoop finished session={} success={} actions={} duration={}ms",
            session_id,
            result.success,
            result.actions_executed,
            result.duration_ms
        );
    });

    Ok(Json(json!({
        "id": id,
        "status": "driving",
        "goal": resp_goal,
        "config": {
            "max_iterations": resp_max_iter,
            "max_total_actions": resp_max_actions,
            "batch_delay_ms": resp_delay,
        },
    })))
}

// ============================================================================
// Studio Puppet WebSocket
// ============================================================================

/// WebSocket upgrade handler for the puppet protocol.
///
/// `GET /v1/studio/sessions/:id/puppet` upgrades to a bidirectional WebSocket:
///   → Backend sends `PuppetCommand` (actions, thinking, status, complete, error)
///   ← Frontend sends `PuppetResponse` (action results, pause, resume, intervene)
pub async fn studio_puppet_ws(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<SharedState>,
    Path(session_id): Path<String>,
) -> axum::response::Response {
    ws.on_upgrade(move |socket| handle_puppet_socket(socket, state, session_id))
}

async fn handle_puppet_socket(
    socket: axum::extract::ws::WebSocket,
    state: SharedState,
    session_id: String,
) {
    use futures::{SinkExt, StreamExt};
    use zeus_prometheus::{PuppetCommand, PuppetResponse};

    let (mut ws_tx, mut ws_rx) = socket.split();

    // Get the director from state
    let director = {
        let state_guard = state.read().await;
        state_guard.agent_director.clone()
    };

    // Subscribe to commands for this session
    let status = director.get_status(&session_id).await;
    if status.is_none() {
        let _ = ws_tx
            .send(axum::extract::ws::Message::Text(
                serde_json::to_string(&PuppetCommand::Error {
                    message: format!("Session {} not found", session_id),
                })
                .unwrap_or_default(),
            ))
            .await;
        return;
    }

    // Get the command broadcast receiver and result sender
    let sessions = &director;
    let (mut cmd_rx, result_tx) = {
        // We need to subscribe to the session's command channel
        // The session was started earlier via start_session — we subscribe here
        let inner_sessions = sessions.active_sessions().await;
        if !inner_sessions.contains(&session_id) {
            let _ = ws_tx
                .send(axum::extract::ws::Message::Text(
                    serde_json::to_string(&PuppetCommand::Error {
                        message: "Session not active".to_string(),
                    })
                    .unwrap_or_default(),
                ))
                .await;
            return;
        }
        // Re-subscribe to the broadcast channel
        match director
            .send_command(
                &session_id,
                PuppetCommand::StatusChange {
                    status: "connected".to_string(),
                    reason: "Puppet WebSocket connected".to_string(),
                },
            )
            .await
        {
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("Failed to send connect notification: {}", e);
            }
        }

        // We need the broadcast receiver — start a fresh subscription
        let (cmd_rx, result_tx) = director
            .start_session(
                session_id.clone(),
                String::new(), // goal already set
            )
            .await;
        (cmd_rx, result_tx)
    };

    // Spawn task to forward commands → WebSocket
    let session_id_clone = session_id.clone();
    let cmd_forward = tokio::spawn(async move {
        loop {
            match cmd_rx.recv().await {
                Ok(cmd) => {
                    let json = match serde_json::to_string(&cmd) {
                        Ok(j) => j,
                        Err(e) => {
                            tracing::warn!("Failed to serialize puppet command: {}", e);
                            continue;
                        }
                    };
                    if ws_tx
                        .send(axum::extract::ws::Message::Text(json))
                        .await
                        .is_err()
                    {
                        tracing::debug!(
                            "Puppet WS send failed for {}, client disconnected",
                            session_id_clone
                        );
                        break;
                    }
                    // If this was a Complete or Error, we're done
                    if matches!(
                        cmd,
                        PuppetCommand::Complete { .. } | PuppetCommand::Error { .. }
                    ) {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        "Puppet command stream lagged by {} for {}",
                        n,
                        session_id_clone
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::debug!("Puppet command channel closed for {}", session_id_clone);
                    break;
                }
            }
        }
    });

    // Forward WebSocket messages → result channel
    let session_id_clone2 = session_id.clone();
    let result_forward = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                axum::extract::ws::Message::Text(text) => {
                    match serde_json::from_str::<PuppetResponse>(&text) {
                        Ok(response) => {
                            if result_tx.send(response).await.is_err() {
                                tracing::debug!("Result channel closed for {}", session_id_clone2);
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Invalid puppet response from {}: {}",
                                session_id_clone2,
                                e
                            );
                        }
                    }
                }
                axum::extract::ws::Message::Close(_) => {
                    tracing::debug!("Puppet WS closed for {}", session_id_clone2);
                    break;
                }
                _ => {} // ignore binary, ping, pong
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = cmd_forward => {},
        _ = result_forward => {},
    }

    tracing::info!("Puppet WebSocket closed for session {}", session_id);
}


// ============================================================================
// Studio Chat (ZeusWeb Studio page)
// ============================================================================

use zeus_llm::LlmClient;
use zeus_session::Session;
use tracing::debug;

/// POST /v1/studio — studio chat endpoint (ZeusWeb Studio page)
///
/// Accepts: `{ "type": "studio", "message": "...", "session_id": "...", "attachments": [...], "system_prompt": "..." }`
/// Returns: `{ "response": "...", "session_id": "..." }`
#[allow(dead_code)]
#[allow(dead_code)]
pub async fn studio_chat(
    State(state): State<SharedState>,
    Json(req): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let message = req
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if message.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Missing 'message' field".to_string(),
        ));
    }

    let session_id = req
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let custom_system_prompt = req
        .get("system_prompt")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let state = state.read().await;

    // Create or load session
    let mut session = if let Some(id) = &session_id {
        Session::load(&state.config.sessions, id)
            .await
            .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?
    } else {
        let s = Session::new(&state.config.sessions);
        s.init()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        s
    };

    let sid = session.id.clone();

    // Create LLM client
    let llm = LlmClient::from_config(&state.config)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Add user message
    let user_msg = zeus_core::Message::user(&message);
    session
        .add(user_msg)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Build system prompt: use custom if provided, else workspace default
    let system_prompt = if let Some(custom) = custom_system_prompt {
        custom
    } else {
        state
            .workspace
            .get_context()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    let tool_schemas = state.tools.schemas();

    // Call LLM
    let response = llm
        .complete(&session.messages, &tool_schemas, Some(&system_prompt))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Record LLM spend
    let cost = super::model_tier_cost(&state.config.model);
    if let Err(e) = state.ledger.spend(
        "default",
        cost,
        zeus_economy::TransactionReason::LlmCall,
        format!("studio: {}", &state.config.model),
    ) {
        debug!("Economy spend failed (non-fatal): {e}");
    }

    // Save assistant response
    let assistant_text = if response.content.is_empty() {
        "[no response]".to_string()
    } else {
        response.content.clone()
    };
    let assistant_msg = zeus_core::Message::assistant(&assistant_text);
    let _ = session.add(assistant_msg).await;

    Ok(Json(json!({
        "response": assistant_text,
        "session_id": sid,
    })))
}
