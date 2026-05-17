//! Agent handler endpoints extracted from mod.rs (A3 handlers split)

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
use super::{agents_dir, find_agent_by_name, CreateAgentRequest, UpdateAgentRequest};
use crate::handlers::personas_handlers::CreateAgentFromPersonaRequest;
use zeus_session::Session;

pub async fn list_agents(State(state): State<SharedState>) -> Json<Value> {
    let dir = agents_dir(&state.read().await.config.workspace);
    let mut agents = Vec::new();

    if dir.exists()
        && let Ok(mut rd) = tokio::fs::read_dir(&dir).await
    {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json")
                && let Ok(content) = tokio::fs::read_to_string(&path).await
                && let Ok(agent) = serde_json::from_str::<Value>(&content)
            {
                agents.push(agent);
            }
        }
    }

    Json(json!({ "agents": agents }))
}

pub async fn create_agent(
    State(state): State<SharedState>,
    Json(req): Json<CreateAgentRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let dir = agents_dir(&state.read().await.config.workspace);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Upsert: reuse existing ID+path if an agent with this name already exists,
    // preventing stale accumulation on repeated POST /v1/agents with the same name.
    let (id, path) = match find_agent_by_name(&dir, &req.name).await {
        Some((existing_id, existing_path)) => {
            info!("Upserting existing agent '{}' ({})", req.name, existing_id);
            (existing_id, existing_path)
        }
        None => {
            let new_id = uuid::Uuid::new_v4().to_string();
            let new_path = dir.join(format!("{}.json", new_id));
            (new_id, new_path)
        }
    };

    let now = chrono::Utc::now().to_rfc3339();

    let autonomy = req
        .autonomy_level
        .or(req.autonomy)
        .unwrap_or_else(|| "supervised".to_string());

    let bindings_val = req.bindings.as_ref().map_or(json!(null), |b| json!(b));
    let tool_policy_val = req.tool_policy.as_ref().map_or(json!(null), |tp| json!(tp));
    let priority_val = req.priority.unwrap_or(0);

    let agent = json!({
        "id": id,
        "name": req.name,
        "role": req.role.unwrap_or_default(),
        "model": req.model.unwrap_or_default(),
        "autonomy": autonomy,
        "autonomy_level": autonomy,
        "persona": req.persona.unwrap_or_default(),
        "soul": req.soul.unwrap_or_default(),
        "tools": req.tools.unwrap_or_default(),
        "heartbeat": req.heartbeat.unwrap_or(json!(null)),
        "bindings": bindings_val,
        "tool_policy": tool_policy_val,
        "priority": priority_val,
        "status": "active",
        "type": "managed",
        "address": "local",
        "created": now
    });

    let content = serde_json::to_string_pretty(&agent)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!("Created agent: {} ({})", req.name, id);

    // Auto-supervisor: create a supervisor agent + team + war room
    if req.auto_supervisor {
        let sup_id = uuid::Uuid::new_v4().to_string();
        let sup_name = format!("{}-supervisor", req.name);
        let sup_path = dir.join(format!("{}.json", sup_id));

        let supervisor = json!({
            "id": sup_id,
            "name": sup_name,
            "role": "supervisor",
            "model": agent["model"],
            "autonomy": "full",
            "autonomy_level": "full",
            "persona": format!(
                "You are the supervisor for agent '{}'. Your role is to coordinate, review outputs, \
                 and ensure quality. Intervene when the worker agent needs guidance or correction. \
                 Communicate via Pantheon war room.",
                req.name
            ),
            "soul": "",
            "tools": [],
            "status": "active",
            "type": "supervisor",
            "address": "local",
            "supervised_agent": id,
            "created": now
        });

        let sup_content = serde_json::to_string_pretty(&supervisor)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        tokio::fs::write(&sup_path, sup_content)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        // Create team linking supervisor + worker
        let team_id = uuid::Uuid::new_v4().to_string();
        let team = json!({
            "id": team_id,
            "name": format!("{}-team", req.name),
            "description": format!("Auto-created team for {} with supervisor", req.name),
            "supervisor_id": sup_id,
            "agents": [id, sup_id],
            "created": now
        });
        let teams_dir = dir.parent().unwrap_or(&dir).join("teams");
        let _ = tokio::fs::create_dir_all(&teams_dir).await;
        let team_path = teams_dir.join(format!("{}.json", team_id));
        let team_content = serde_json::to_string_pretty(&team)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let _ = tokio::fs::write(&team_path, team_content).await;

        info!("Auto-created supervisor '{}' ({}) and team '{}'", sup_name, sup_id, team_id);

        return Ok(Json(json!({
            "message": "Agent created with supervisor",
            "id": id,
            "supervisor_id": sup_id,
            "team_id": team_id
        })));
    }

    Ok(Json(json!({
        "message": "Agent created",
        "id": id
    })))
}

pub async fn get_agent(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let path = agents_dir(&state.read().await.config.workspace).join(format!("{}.json", id));

    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Agent not found: {}", id)));
    }

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let agent: Value = serde_json::from_str(&content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(agent))
}

pub async fn update_agent(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAgentRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let path = agents_dir(&state.read().await.config.workspace).join(format!("{}.json", id));

    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Agent not found: {}", id)));
    }

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut agent: Value = serde_json::from_str(&content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(status) = req.status {
        agent["status"] = Value::String(status);
    }
    if let Some(autonomy) = req.autonomy {
        agent["autonomy"] = Value::String(autonomy.clone());
        agent["autonomy_level"] = Value::String(autonomy);
    }
    if let Some(autonomy_level) = req.autonomy_level {
        agent["autonomy_level"] = Value::String(autonomy_level.clone());
        agent["autonomy"] = Value::String(autonomy_level);
    }
    if let Some(model) = req.model {
        agent["model"] = Value::String(model);
    }
    if let Some(name) = req.name {
        agent["name"] = Value::String(name);
    }
    if let Some(role) = req.role {
        agent["role"] = Value::String(role);
    }
    if let Some(persona) = req.persona {
        agent["persona"] = Value::String(persona);
    }
    if let Some(soul) = req.soul {
        agent["soul"] = Value::String(soul);
    }
    if let Some(tools) = req.tools {
        agent["tools"] = json!(tools);
    }
    if let Some(heartbeat) = req.heartbeat {
        agent["heartbeat"] = heartbeat;
    }
    if let Some(bindings) = &req.bindings {
        agent["bindings"] = json!(bindings);
    }
    if let Some(tool_policy) = &req.tool_policy {
        agent["tool_policy"] = json!(tool_policy);
    }
    if let Some(priority) = req.priority {
        agent["priority"] = json!(priority);
    }

    tokio::fs::write(
        &path,
        serde_json::to_string_pretty(&agent).unwrap_or_default(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "message": "Agent updated",
        "id": id
    })))
}

pub async fn delete_agent(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let path = agents_dir(&state.read().await.config.workspace).join(format!("{}.json", id));

    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Agent not found: {}", id)));
    }

    tokio::fs::remove_file(&path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!("Deleted agent: {}", id);

    Ok(Json(json!({
        "success": true,
        "id": id,
        "message": "Agent deleted"
    })))
}

pub async fn create_agent_from_persona(
    State(state): State<crate::SharedState>,
    Path(persona_name): Path<String>,
    Json(req): Json<CreateAgentFromPersonaRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let workspace = state_guard.config.workspace_path().to_path_buf();
    drop(state_guard);

    let template =
        zeus_core::PersonaTemplate::find(&persona_name, &workspace).ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Persona not found: {}", persona_name),
            )
        })?;

    let dir = agents_dir(&workspace);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let agent_name = req.name.unwrap_or_else(|| template.name.clone());
    let agent_model = req.model.unwrap_or_else(|| template.model.clone());
    let autonomy = req.autonomy.unwrap_or_else(|| "supervised".to_string());

    let agent = json!({
        "id": id,
        "name": agent_name,
        "role": "assistant",
        "model": agent_model,
        "autonomy": &autonomy,
        "autonomy_level": &autonomy,
        "persona": template.persona_text,
        "soul": "",
        "tools": template.tools,
        "heartbeat": null,
        "bindings": null,
        "tool_policy": null,
        "priority": 0,
        "status": "active",
        "type": "managed",
        "persona_source": template.name,
        "address": "local",
        "created": now,
    });

    let path = dir.join(format!("{}.json", id));
    let content = serde_json::to_string_pretty(&agent)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!(
        "Created agent '{}' from persona '{}': {}",
        agent_name, persona_name, id
    );

    Ok(Json(agent))
}

pub async fn agent_status(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    match state.agent_registry.get(&id) {
        Some(instance) => Ok(Json(json!({
            "agent_id": instance.agent_id,
            "name": instance.name,
            "spawned_at": instance.spawned_at.to_rfc3339(),
            "last_active": instance.last_active.to_rfc3339(),
            "message_count": instance.message_count,
            "bindings": instance.binding.bindings,
            "tool_policy": instance.binding.tool_policy,
            "priority": instance.binding.priority
        }))),
        None => Err((
            StatusCode::NOT_FOUND,
            format!("Agent '{}' is not spawned", id),
        )),
    }
}

#[derive(Debug, Deserialize)]
pub struct AgentChatRequest {
    pub message: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

pub async fn agent_chat(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<AgentChatRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    info!(
        "Agent chat request: agent={}, {} chars",
        id,
        req.message.len()
    );

    // Get agent Arc, session config, and update activity — then release state lock
    let (agent_arc, sessions_dir) = {
        let mut st = state.write().await;
        st.agent_registry.update_activity(&id);
        match st.agent_registry.get(&id) {
            Some(instance) => (instance.agent.clone(), st.config.sessions.clone()),
            None => {
                return Err((
                    StatusCode::NOT_FOUND,
                    format!(
                        "Agent '{}' is not spawned. Spawn it first via POST /v1/agents/spawn",
                        id
                    ),
                ));
            }
        }
    };

    // Create or load session keyed by agent + optional session_id
    let mut session = if let Some(ref sid) = req.session_id {
        Session::load(&sessions_dir, sid)
            .await
            .map_err(|e| (StatusCode::NOT_FOUND, format!("Session not found: {}", e)))?
    } else {
        let s = Session::new(&sessions_dir);
        s.init()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        s
    };

    let session_id = session.id.clone();

    // Add user message to session
    let user_msg = zeus_core::Message::user(&req.message);
    session
        .add(user_msg)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Run through the agent loop
    let mut agent = agent_arc.write().await;
    match agent.run(&req.message).await {
        Ok(response) => {
            // Persist assistant response to session
            let assistant_msg = zeus_core::Message::assistant(&response);
            let _ = session.add(assistant_msg).await;

            // Fire webhook event if WebhookManager is available
            {
                let st = state.read().await;
                let data = serde_json::json!({
                    "agent_id": id,
                    "session_id": session_id,
                    "message": req.message,
                    "response_length": response.len(),
                });
                st.webhook_manager
                    .fire_event(crate::webhook_outbound::WebhookEventType::Message, data)
                    .await;
            }

            Ok(Json(json!({
                "agent_id": id,
                "session_id": session_id,
                "response": response,
            })))
        }
        Err(e) => {
            // Fire error webhook
            {
                let st = state.read().await;
                let data = serde_json::json!({
                    "agent_id": id,
                    "session_id": session_id,
                    "error": e.to_string(),
                });
                st.webhook_manager
                    .fire_event(crate::webhook_outbound::WebhookEventType::Error, data)
                    .await;
            }

            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Agent error: {}", e),
            ))
        }
    }
}

pub async fn agent_status_stream(
    State(state): State<SharedState>,
) -> axum::response::sse::Sse<impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    let state = state.read().await;
    let mut rx = state.agent_status_tx.subscribe();
    // Send current snapshot of all zone assignments as initial event
    let zones: serde_json::Value = serde_json::json!({
        "type": "snapshot",
        "zones": state.agent_zones.iter().map(|e| {
            serde_json::json!({"agent_id": e.key(), "zone": e.value()})
        }).collect::<Vec<_>>()
    });
    drop(state);

    let stream = async_stream::stream! {
        // Emit snapshot first
        if let Ok(json) = serde_json::to_string(&zones) {
            yield Ok(axum::response::sse::Event::default().event("snapshot").data(json));
        }
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Ok(json) = serde_json::to_string(&event) {
                        yield Ok(axum::response::sse::Event::default().event("agent_update").data(json));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!("Agent status stream lagged {} messages", n);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    axum::response::sse::Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
}

pub async fn assign_agent_zone(
    State(state): State<SharedState>,
    Path(agent_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let zone = match body.get("zone").and_then(|v| v.as_str()) {
        Some(z) if !z.trim().is_empty() => z.trim().to_string(),
        _ => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({"error": "zone field is required and must be non-empty"})),
            ).into_response();
        }
    };

    let state_guard = state.read().await;

    // Validate agent exists
    let agent_exists = state_guard.agent_registry.get(&agent_id).is_some();
    if !agent_exists {
        return (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": format!("agent '{}' not found", agent_id)})),
        ).into_response();
    }

    // Update zone registry (DashMap + SQLite persistence)
    state_guard.agent_zones.insert(agent_id.clone(), zone.clone());
    state_guard.pantheon.save_agent_zone(&agent_id, &zone).await;

    // Broadcast zone change event
    let event = serde_json::json!({
        "type": "zone_change",
        "agent_id": agent_id,
        "zone": zone,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    let _ = state_guard.agent_status_tx.send(event);

    tracing::info!("Agent '{}' assigned to zone '{}'", agent_id, zone);

    (
        axum::http::StatusCode::OK,
        axum::Json(serde_json::json!({
            "agent_id": agent_id,
            "zone": zone,
            "status": "assigned"
        })),
    ).into_response()
}


/// GET /v1/office/state — current office state for the Star Office game
pub async fn office_state(
    State(state): State<SharedState>,
) -> impl IntoResponse {
    let state = state.read().await;
    let now = chrono::Utc::now();

    let agents: Vec<serde_json::Value> = state.agent_zones.iter().map(|e| {
        let agent_id = e.key().clone();
        let zone = e.value().clone();

        // Derive real state from agent registry
        let (agent_state, detail) = match state.agent_registry.get(&agent_id) {
            Some(instance) => {
                let secs_since_active = (now - instance.last_active).num_seconds();
                if secs_since_active < 30 {
                    ("working".to_string(), format!("{} messages processed", instance.message_count))
                } else {
                    ("idle".to_string(), format!("last active {}s ago", secs_since_active))
                }
            }
            None => ("offline".to_string(), "not spawned".to_string()),
        };

        serde_json::json!({
            "agentId": agent_id,
            "name": agent_id,
            "state": agent_state,
            "area": zone,
            "detail": detail
        })
    }).collect();

    // Overall office state: working if any agent is working, idle if all idle, offline if empty
    let overall = if agents.iter().any(|a| a["state"] == "working") {
        "working"
    } else if agents.is_empty() || agents.iter().all(|a| a["state"] == "offline") {
        "offline"
    } else {
        "idle"
    };

    axum::Json(serde_json::json!({
        "state": overall,
        "detail": format!("{} agents in office", agents.len()),
        "agents": agents
    }))
}

/// POST /v1/office/join — Agent joins the Star Office
pub async fn office_join(
    State(state): State<SharedState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = body.get("agentId").and_then(|v| v.as_str()).unwrap_or("unknown");
    let area = body.get("area").and_then(|v| v.as_str()).unwrap_or("breakroom");

    let state = state.read().await;
    state.agent_zones.insert(agent_id.to_string(), area.to_string());

    axum::Json(serde_json::json!({
        "success": true,
        "agentId": agent_id,
        "area": area
    }))
}

/// POST /v1/office/leave — Agent leaves the Star Office
pub async fn office_leave(
    State(state): State<SharedState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = body.get("agentId").and_then(|v| v.as_str()).unwrap_or("unknown");

    let state = state.read().await;
    state.agent_zones.remove(agent_id);

    axum::Json(serde_json::json!({
        "success": true,
        "agentId": agent_id
    }))
}
