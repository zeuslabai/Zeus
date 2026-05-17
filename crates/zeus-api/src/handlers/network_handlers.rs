//! Network, inter-agent messaging, and node management handlers.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{info, warn};

use zeus_core::ChannelSource;
use crate::SharedState;
use crate::AgentMessage;
use super::agents_dir;

// ============================================================================
// Network agent discovery
// ============================================================================

/// GET /v1/network/agents — List known agents (local + managed)
pub async fn network_agents(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let _ = &state.config; // acknowledge state

    // Always include the local Zeus instance
    let agent_name = std::env::var("ZEUS_AGENT_ID")
        .ok()
        .or_else(|| state.config.agent.as_ref().and_then(|a| a.name.clone()))
        .or_else(|| state.config.name.clone())
        .or_else(|| state.config.network.as_ref().and_then(|n| n.agent_name.clone()))
        .unwrap_or_else(|| "Zeus".to_string());
    let mut agents = vec![json!({
        "id": "local",
        "name": agent_name,
        "type": "local",
        "status": "online",
        "address": "127.0.0.1:8080"
    })];

    // Also include managed agents from the configured agents directory
    let dir = agents_dir(&state.config.workspace);
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

    // S93: Include agents discovered from Discord channel presence
    {
        let now = chrono::Utc::now().timestamp();
        let presence = state.channel_presence.read().await;
        for (id, agent) in presence.iter() {
            // Skip self (local agent already included)
            if id == "local" { continue; }
            let age_secs = now - agent.last_seen;
            let status = if age_secs < 300 { "active" } else if age_secs < 1800 { "idle" } else { "offline" };
            agents.push(serde_json::json!({
                "id": id,
                "name": agent.name,
                "type": if agent.agent_type == "human" { "human" } else { "channel" },
                "status": status,
                "last_seen": agent.last_seen,
                "current_task": agent.last_message,
                "agent_type": agent.agent_type,
            }));
        }
    }

    Json(json!({ "agents": agents }))
}

/// GET /v1/network/discover — mDNS discovery results
///
/// Returns known Zeus peers discovered via mDNS on the local network.
/// Also triggers a 2-second active scan for new peers.
pub async fn network_discover(State(state): State<SharedState>) -> Json<Value> {
    let st = state.read().await;
    // Return already-known peers
    let peers = st.mdns_discovery().known_peers().await;
    let broadcasting = st.mdns_discovery().is_broadcasting().await;
    Json(json!({
        "implemented": true,
        "mdns": peers,
        "broadcasting": broadcasting,
        "peer_count": peers.len(),
    }))
}

// ============================================================================
// Inter-agent messaging
// ============================================================================

/// Request body for receiving a message from a peer agent.
#[derive(Debug, Deserialize)]
pub struct IncomingAgentMessage {
    /// Sender agent name (e.g. "@zeus_bot")
    pub from_agent: String,
    /// Sender host (e.g. "192.168.1.112")
    pub from_host: String,
    /// Target agent (optional — None means "to whoever is on this gateway")
    pub to_agent: Option<String>,
    /// Message content
    pub content: String,
}

/// Request body for sending a message to a specific peer.
#[derive(Debug, Deserialize)]
pub struct SendToPeerRequest {
    /// Target peer host IP or hostname (e.g. "192.168.1.100")
    pub host: String,
    /// Target peer port (default: 8080)
    pub port: Option<u16>,
    /// Sender agent name
    pub from_agent: String,
    /// Target agent name (optional)
    pub to_agent: Option<String>,
    /// Message content
    pub content: String,
}

/// Request body for broadcasting a message to all known mDNS peers.
#[derive(Debug, Deserialize)]
pub struct BroadcastRequest {
    /// Sender agent name
    pub from_agent: String,
    /// Message content
    pub content: String,
}

/// Forward a message to a tmux session by typing it via send-keys.
///
/// Replicates the Telegram relay mechanism: the formatted message is typed
/// literally into the target tmux pane and Enter is pressed.
pub async fn forward_to_tmux(session: &str, text: &str) {
    let tmux = if std::path::Path::new("/opt/homebrew/bin/tmux").exists() {
        "/opt/homebrew/bin/tmux"
    } else if std::path::Path::new("/usr/local/bin/tmux").exists() {
        "/usr/local/bin/tmux"
    } else {
        "tmux"
    };

    // Resolve tmux socket — launchd services don't inherit TMUX env var
    let uid = std::process::Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "501".to_string());
    let socket_path = format!("/private/tmp/tmux-{}/default", uid);

    let escaped = text.replace('\'', "'\\''");

    // Type the message literally into the tmux pane
    let result = tokio::process::Command::new(tmux)
        .args([
            "-S",
            &socket_path,
            "send-keys",
            "-t",
            session,
            "-l",
            &escaped,
        ])
        .output()
        .await;
    match &result {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("tmux send-keys failed (exit {}): {}", output.status, stderr);
            return;
        }
        Err(e) => {
            warn!("tmux send-keys spawn failed: {}", e);
            return;
        }
        _ => {}
    }

    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    // Press Enter
    let result = tokio::process::Command::new(tmux)
        .args(["-S", &socket_path, "send-keys", "-t", session, "C-m"])
        .output()
        .await;
    match &result {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("tmux Enter failed (exit {}): {}", output.status, stderr);
        }
        Err(e) => {
            warn!("tmux Enter spawn failed: {}", e);
        }
        _ => {}
    }

    info!("Forwarded to tmux {}: {} chars", session, text.len());
}

/// Auto-detect an active tmux session (prefers sessions starting with "zeus-").
pub async fn detect_active_tmux_session() -> Option<String> {
    let tmux = if std::path::Path::new("/opt/homebrew/bin/tmux").exists() {
        "/opt/homebrew/bin/tmux"
    } else if std::path::Path::new("/usr/local/bin/tmux").exists() {
        "/usr/local/bin/tmux"
    } else {
        "tmux"
    };

    let uid = std::process::Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "501".to_string());
    let socket_path = format!("/private/tmp/tmux-{}/default", uid);

    let output = tokio::process::Command::new(tmux)
        .args(["-S", &socket_path, "list-sessions", "-F", "#{session_name}"])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let sessions: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Prefer sessions starting with "zeus-"
    sessions
        .iter()
        .find(|s| s.starts_with("zeus-"))
        .cloned()
        .or_else(|| sessions.into_iter().next())
}

/// GET /v1/network/messages — List inbox messages
///
/// Returns the in-memory ring buffer of recent inter-agent messages.
pub async fn network_messages(State(state): State<SharedState>) -> Json<Value> {
    let st = state.read().await;
    let inbox = st.message_inbox.lock().await;
    let messages: Vec<_> = inbox.messages.iter().cloned().collect();
    let total = messages.len();
    Json(json!({
        "implemented": true,
        "messages": messages,
        "total": total,
    }))
}

/// POST /v1/network/messages — Receive a message from a peer agent
///
/// Routes to the unified agent inbox when available (standalone mode),
/// falls back to tmux session forwarding, always stores in message_inbox.
pub async fn receive_network_message(
    State(state): State<SharedState>,
    Json(body): Json<IncomingAgentMessage>,
) -> (StatusCode, Json<Value>) {
    let msg_id = uuid::Uuid::new_v4().to_string();
    let timestamp = chrono::Utc::now().to_rfc3339();

    info!(
        "Received agent message from {} ({}): {} chars",
        body.from_agent,
        body.from_host,
        body.content.len()
    );

    let formatted = format!(
        "(Agent {} from {}) {}",
        body.from_agent, body.from_host, body.content
    );

    // Route: unified inbox first (standalone agents), then tmux, then stored-only
    let mut delivered = false;
    {
        let st = state.read().await;
        if let Some(ref inbox) = st.agent_inbox {
            let source = ChannelSource {
                channel_type: "network".to_string(),
                channel_id: Some(body.from_host.clone()),
                sender_name: Some(body.from_agent.clone()),
                sender_id: None,
                channel_name: None,
            };
            delivered = inbox.try_send(formatted.clone(), Some(source), 120, true);
            if !delivered {
                warn!("Agent inbox full — network message from {} queued in ring buffer only", body.from_agent);
            }
        }
    }
    if !delivered {
        if let Some(session) = detect_active_tmux_session().await {
            forward_to_tmux(&session, &formatted).await;
            delivered = true;
        } else {
            warn!("No agent inbox or tmux session — network message stored in ring buffer only");
        }
    }

    // Always store in ring buffer for polling / history
    let agent_msg = AgentMessage {
        id: msg_id.clone(),
        from_agent: body.from_agent,
        from_host: body.from_host,
        to_agent: body.to_agent,
        content: body.content,
        timestamp,
        delivered,
    };
    {
        let st = state.read().await;
        let mut inbox = st.message_inbox.lock().await;
        inbox.push(agent_msg);
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "message_id": msg_id,
            "delivered": delivered,
        })),
    )
}

/// POST /v1/network/send — Send a message to a specific peer by IP/hostname
///
/// Forwards the message to the target peer's `/v1/network/messages` endpoint.
pub async fn network_send(
    State(state): State<SharedState>,
    Json(body): Json<SendToPeerRequest>,
) -> (StatusCode, Json<Value>) {
    let port = body.port.unwrap_or(8080);
    let url = format!("http://{}:{}/v1/network/messages", body.host, port);

    let payload = serde_json::json!({
        "from_agent": body.from_agent,
        "from_host": get_local_ip(),
        "to_agent": body.to_agent,
        "content": body.content,
    });

    let st = state.read().await;
    let result = st
        .http_client
        .post(&url)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    match result {
        Ok(resp) => {
            let status = resp.status();
            let body_val: Value = resp
                .json()
                .await
                .unwrap_or(json!({"error": "invalid response"}));
            info!("Sent message to {} — status {}", url, status);
            (
                StatusCode::OK,
                Json(json!({
                    "ok": status.is_success(),
                    "target": format!("{}:{}", body.host, port),
                    "status": status.as_u16(),
                    "response": body_val,
                })),
            )
        }
        Err(e) => {
            warn!("Failed to send message to {}: {}", url, e);
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "ok": false,
                    "error": format!("Failed to reach peer: {}", e),
                    "target": format!("{}:{}", body.host, port),
                })),
            )
        }
    }
}

/// POST /v1/network/broadcast — Broadcast a message to ALL known mDNS peers
///
/// Fans out the message to every discovered peer's `/v1/network/messages`.
pub async fn network_broadcast(
    State(state): State<SharedState>,
    Json(body): Json<BroadcastRequest>,
) -> Json<Value> {
    let st = state.read().await;
    let peers = st.mdns_discovery().known_peers().await;
    let local_ip = get_local_ip();

    if peers.is_empty() {
        return Json(json!({
            "ok": true,
            "broadcast_count": 0,
            "results": [],
            "note": "No mDNS peers discovered. Run GET /v1/network/discover first.",
        }));
    }

    let client = st.http_client.clone();
    drop(st); // release read lock before async fan-out

    let mut results = Vec::new();

    for peer in &peers {
        let url = format!("http://{}:{}/v1/network/messages", peer.address, peer.port);
        let payload = serde_json::json!({
            "from_agent": body.from_agent,
            "from_host": &local_ip,
            "content": body.content,
        });

        let resp = client
            .post(&url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        match resp {
            Ok(r) => {
                let status = r.status();
                info!(
                    "Broadcast to {} ({}:{}) — {}",
                    peer.instance_name, peer.address, peer.port, status
                );
                results.push(json!({
                    "peer": peer.instance_name,
                    "address": format!("{}:{}", peer.address, peer.port),
                    "ok": status.is_success(),
                    "status": status.as_u16(),
                }));
            }
            Err(e) => {
                warn!("Broadcast to {} failed: {}", peer.instance_name, e);
                results.push(json!({
                    "peer": peer.instance_name,
                    "address": format!("{}:{}", peer.address, peer.port),
                    "ok": false,
                    "error": e.to_string(),
                }));
            }
        }
    }

    let success_count = results
        .iter()
        .filter(|r| r["ok"].as_bool().unwrap_or(false))
        .count();

    Json(json!({
        "ok": success_count > 0,
        "broadcast_count": results.len(),
        "success_count": success_count,
        "results": results,
    }))
}

/// Get local IP address for outbound message tagging.
fn get_local_ip() -> String {
    std::env::var("ZEUS_HOST_IP").unwrap_or_else(|_| {
        hostname::get()
            .ok()
            .and_then(|h| h.to_str().map(String::from))
            .unwrap_or_else(|| "127.0.0.1".to_string())
    })
}

// ============================================================================
// Node management endpoints (hub-spoke WebSocket fleet)
// ============================================================================

/// GET /v1/nodes — List all connected WebSocket nodes
pub async fn list_nodes(State(state): State<SharedState>) -> Json<Value> {
    let st = state.read().await;
    let nodes = st.node_registry.list_nodes();
    Json(json!({
        "nodes": nodes,
        "count": nodes.len(),
    }))
}

/// GET /v1/nodes/:id — Get info for a specific connected node
pub async fn get_node(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let st = state.read().await;
    match st.node_registry.get_node_info(&id) {
        Some(info) => (StatusCode::OK, Json(json!(info))),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Node '{}' not connected", id)})),
        ),
    }
}

/// Request body for invoking a method on a connected node.
#[derive(Debug, serde::Deserialize)]
pub struct InvokeNodeRequest {
    pub method: String,
    #[serde(default)]
    pub params: Value,
    /// Timeout in seconds (default: 30)
    #[serde(default = "default_invoke_timeout")]
    pub timeout: u64,
}

fn default_invoke_timeout() -> u64 {
    30
}

/// POST /v1/nodes/:id/invoke — Invoke a method on a connected node (request-response)
pub async fn invoke_node(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<InvokeNodeRequest>,
) -> (StatusCode, Json<Value>) {
    let st = state.read().await;
    let timeout = std::time::Duration::from_secs(body.timeout);
    let registry = st.node_registry.clone();
    drop(st); // release read lock before async invoke

    match registry
        .invoke(&id, &body.method, body.params, timeout)
        .await
    {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "node_id": id,
                "method": body.method,
                "result": result,
            })),
        ),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "ok": false,
                "node_id": id,
                "method": body.method,
                "error": e,
            })),
        ),
    }
}

/// Request body for sending an event to a connected node.
#[derive(Debug, serde::Deserialize)]
pub struct NodeEventRequest {
    pub event_type: String,
    #[serde(default)]
    pub data: Value,
}

/// POST /v1/nodes/:id/event — Send a one-way event to a connected node
pub async fn send_node_event(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<NodeEventRequest>,
) -> (StatusCode, Json<Value>) {
    let st = state.read().await;
    let sent = st
        .node_registry
        .send_event(&id, &body.event_type, body.data);
    if sent {
        (
            StatusCode::OK,
            Json(json!({"ok": true, "node_id": id, "event_type": body.event_type})),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"ok": false, "error": format!("Node '{}' not connected", id)})),
        )
    }
}

/// Request body for broadcasting an event to all connected nodes.
#[derive(Debug, serde::Deserialize)]
pub struct BroadcastNodesRequest {
    pub event_type: String,
    #[serde(default)]
    pub data: Value,
}

/// POST /v1/nodes/broadcast — Broadcast an event to ALL connected nodes
pub async fn broadcast_nodes(
    State(state): State<SharedState>,
    Json(body): Json<BroadcastNodesRequest>,
) -> Json<Value> {
    let st = state.read().await;
    let count = st
        .node_registry
        .broadcast_event(&body.event_type, body.data);
    Json(json!({
        "ok": true,
        "event_type": body.event_type,
        "delivered": count,
        "total_nodes": st.node_registry.node_count(),
    }))
}
