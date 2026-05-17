//! WebSocket handler for fleet node connections at `GET /v1/ws/nodes`.
//!
//! Hub-spoke architecture:
//! - Coordinator runs this endpoint, accepting persistent WS from fleet agents
//! - Each agent sends a Register message on connect
//! - Hub can invoke (request-response), push events, and broadcast
//! - Agents auto-reconnect on disconnect
//!
//! Protocol:
//! - Client → Hub: NodeClientMessage (JSON, serde-tagged)
//! - Hub → Client: NodeServerMessage (JSON, serde-tagged)

use axum::{
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::SharedState;
use crate::node_registry::{NodeClientMessage, NodeServerMessage};

/// Default WS message size for node connections (1 MB).
/// Overridable via `[gateway].max_ws_message_bytes` in config.toml.
const DEFAULT_MAX_NODE_WS_SIZE: usize = 1_048_576;

/// Timeout for initial Register message.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(10);

/// Ping interval for keepalive.
const PING_INTERVAL: Duration = Duration::from_secs(30);

/// WebSocket upgrade handler at `GET /v1/ws/nodes`.
pub async fn node_ws_handler(ws: WebSocketUpgrade, State(state): State<SharedState>) -> Response {
    let max_ws = {
        let s = state.read().await;
        s.config
            .gateway
            .as_ref()
            .map(|g| g.max_ws_message_bytes)
            .unwrap_or(DEFAULT_MAX_NODE_WS_SIZE)
    };
    ws.max_message_size(max_ws)
        .on_upgrade(move |socket| handle_node_socket(socket, state))
}

/// Handle an active node WebSocket connection.
async fn handle_node_socket(socket: WebSocket, state: SharedState) {
    let (mut sender, mut receiver) = socket.split();
    info!("Node WebSocket: new connection, waiting for Register...");

    // ── Step 1: Wait for Register message ─────────────────────────────────
    let (node_id, host, capabilities) = match wait_for_register(&mut receiver).await {
        Some(reg) => reg,
        None => {
            warn!("Node WebSocket: no Register received within timeout, closing");
            let _ = sender.close().await;
            return;
        }
    };

    info!("Node WebSocket: {} ({}) registering", node_id, host);

    // ── Step 2: Create channel and register in NodeRegistry ───────────────
    let (tx, mut rx) = mpsc::unbounded_channel::<NodeServerMessage>();

    {
        let st = state.read().await;
        st.node_registry
            .register(node_id.clone(), host.clone(), capabilities, tx);
    }

    // Send Registered acknowledgement
    let ack = NodeServerMessage::Registered {
        node_id: node_id.clone(),
    };
    if send_node_msg(&mut sender, &ack).await.is_err() {
        warn!(
            "Node WebSocket: failed to send Registered ack to {}",
            node_id
        );
        state.read().await.node_registry.unregister(&node_id);
        return;
    }

    info!("Node WebSocket: {} registered successfully", node_id);

    // ── Step 3: Main select loop ──────────────────────────────────────────
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    ping_interval.tick().await; // skip immediate first tick

    loop {
        tokio::select! {
            // Hub -> Node: messages from NodeRegistry (invokes, events, etc.)
            hub_msg = rx.recv() => {
                match hub_msg {
                    Some(msg) => {
                        if send_node_msg(&mut sender, &msg).await.is_err() {
                            warn!("Node WebSocket: send to {} failed, disconnecting", node_id);
                            break;
                        }
                    }
                    None => {
                        // Channel closed (registry dropped our sender)
                        info!("Node WebSocket: channel for {} closed", node_id);
                        break;
                    }
                }
            }

            // Node -> Hub: messages from the node's WebSocket
            ws_msg = receiver.next() => {
                let text = match ws_msg {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None => {
                        info!("Node WebSocket: {} disconnected", node_id);
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sender.send(Message::Pong(data)).await;
                        continue;
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => {
                        warn!("Node WebSocket: {} receive error: {}", node_id, e);
                        break;
                    }
                };

                // Parse and handle client message
                match serde_json::from_str::<NodeClientMessage>(&text) {
                    Ok(client_msg) => {
                        handle_node_message(&state, &node_id, client_msg, &mut sender).await;
                    }
                    Err(e) => {
                        warn!("Node WebSocket: {} sent invalid JSON: {}", node_id, e);
                        let err = NodeServerMessage::Event {
                            event_type: "error".to_string(),
                            data: serde_json::json!({"message": format!("Invalid message: {}", e)}),
                        };
                        let _ = send_node_msg(&mut sender, &err).await;
                    }
                }
            }

            // Keepalive ping
            _ = ping_interval.tick() => {
                if send_node_msg(&mut sender, &NodeServerMessage::Ping).await.is_err() {
                    warn!("Node WebSocket: ping to {} failed, disconnecting", node_id);
                    break;
                }
            }
        }
    }

    // ── Step 4: Cleanup ───────────────────────────────────────────────────
    state.read().await.node_registry.unregister(&node_id);
    info!("Node WebSocket: {} session ended", node_id);
}

/// Wait for the Register message from a new node connection.
async fn wait_for_register(
    receiver: &mut (impl StreamExt<Item = Result<Message, axum::Error>> + Unpin),
) -> Option<(String, String, Vec<String>)> {
    let result = tokio::time::timeout(REGISTER_TIMEOUT, async {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(NodeClientMessage::Register {
                        node_id,
                        host,
                        capabilities,
                    }) = serde_json::from_str(&text)
                    {
                        return Some((node_id, host, capabilities));
                    }
                    // Not a Register message — ignore and keep waiting
                    debug!("Node WebSocket: received non-Register message while waiting");
                }
                Ok(Message::Close(_)) | Err(_) => return None,
                _ => continue,
            }
        }
        None
    })
    .await;

    match result {
        Ok(Some(reg)) => Some(reg),
        _ => None,
    }
}

/// Handle an incoming message from a connected node.
async fn handle_node_message(
    state: &SharedState,
    node_id: &str,
    msg: NodeClientMessage,
    sender: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
) {
    match msg {
        NodeClientMessage::Register { .. } => {
            // Already registered — ignore duplicate register
            debug!("Node {}: duplicate Register ignored", node_id);
        }

        NodeClientMessage::InvokeResult { id, result, error } => {
            // Node is responding to a hub invoke
            debug!("Node {}: invoke result for {}", node_id, id);
            let st = state.read().await;
            st.node_registry.handle_invoke_result(&id, result, error);
        }

        NodeClientMessage::Invoke { id, method, params } => {
            // Node is invoking something on the hub (reverse invoke)
            info!("Node {}: reverse invoke '{}' (id={})", node_id, method, id);
            let result = handle_reverse_invoke(state, node_id, &method, &params).await;
            let response = match result {
                Ok(val) => NodeServerMessage::InvokeResult {
                    id,
                    result: val,
                    error: None,
                },
                Err(e) => NodeServerMessage::InvokeResult {
                    id,
                    result: serde_json::Value::Null,
                    error: Some(e),
                },
            };
            let _ = send_node_msg(sender, &response).await;
        }

        NodeClientMessage::Event { event_type, data } => {
            info!("Node {}: event '{}': {}", node_id, event_type, data);
            // Forward events to tmux if they're "message" type
            if event_type == "message" || event_type == "report" {
                let text = data
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&data.to_string())
                    .to_string();
                let formatted = format!("[Node {}] {}", node_id, text);
                if let Some(session) = crate::handlers::detect_active_tmux_session().await {
                    crate::handlers::forward_to_tmux(&session, &formatted).await;
                }
            }
        }

        NodeClientMessage::Pong => {
            debug!("Node {}: pong", node_id);
        }
    }
}

/// Handle a reverse invoke from a node (node invoking the hub).
///
/// Supported methods:
/// - `list_nodes` — returns connected node list
/// - `hub_status` — returns hub status info
async fn handle_reverse_invoke(
    state: &SharedState,
    _from_node: &str,
    method: &str,
    _params: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let st = state.read().await;
    match method {
        "list_nodes" => {
            let nodes = st.node_registry.list_nodes();
            Ok(serde_json::json!({"nodes": nodes}))
        }
        "hub_status" => Ok(serde_json::json!({
            "connected_nodes": st.node_registry.node_count(),
            "uptime": "ok",
        })),
        "ping" => Ok(serde_json::json!({"pong": true})),
        _ => Err(format!("Unknown hub method: {}", method)),
    }
}

/// Serialize and send a NodeServerMessage over WebSocket.
async fn send_node_msg(
    sender: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
    msg: &NodeServerMessage,
) -> Result<(), ()> {
    let json = serde_json::to_string(msg).unwrap_or_default();
    sender.send(Message::Text(json)).await.map_err(|e| {
        error!("Failed to send WS message: {}", e);
    })
}
