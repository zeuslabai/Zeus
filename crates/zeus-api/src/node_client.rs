//! Node Client — auto-connect client for fleet agents.
//!
//! Fleet agents run this to connect TO the coordinator's `/v1/ws/nodes` endpoint.
//! The client:
//! 1. Connects to hub via WebSocket
//! 2. Sends Register message with node identity
//! 3. Handles incoming invokes from hub (forwards to local tmux)
//! 4. Auto-reconnects on disconnect
//!
//! Usage: `zeus gateway --connect-hub ws://192.168.1.112:8080/v1/ws/nodes`

use futures::{SinkExt, StreamExt};
use serde_json::Value;
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::node_registry::{NodeClientMessage, NodeServerMessage};

/// Configuration for the node client.
#[derive(Debug, Clone)]
pub struct NodeClientConfig {
    /// WebSocket URL of the hub (e.g. ws://192.168.1.112:8080/v1/ws/nodes)
    pub hub_url: String,
    /// This agent's identity (e.g. "@zeus100")
    pub node_id: String,
    /// This agent's host IP (e.g. "192.168.1.100")
    pub host: String,
    /// Capabilities to advertise
    pub capabilities: Vec<String>,
    /// How long to wait between reconnect attempts
    pub reconnect_interval: Duration,
    /// Target tmux session for delivering hub invokes
    pub tmux_target: Option<String>,
}

impl NodeClientConfig {
    pub fn new(hub_url: String, node_id: String, host: String) -> Self {
        Self {
            hub_url,
            node_id,
            host,
            capabilities: vec![],
            reconnect_interval: Duration::from_secs(5),
            tmux_target: None,
        }
    }
}

/// Run the node client with auto-reconnect.
///
/// This blocks indefinitely, reconnecting on disconnect.
pub async fn run_node_client(config: NodeClientConfig) {
    info!(
        "Node client starting: {} connecting to {}",
        config.node_id, config.hub_url
    );

    loop {
        match connect_to_hub(&config).await {
            Ok(()) => {
                info!("Node client: connection to hub ended cleanly, reconnecting...");
            }
            Err(e) => {
                warn!(
                    "Node client: connection failed: {}, reconnecting in {}s...",
                    e,
                    config.reconnect_interval.as_secs()
                );
            }
        }

        tokio::time::sleep(config.reconnect_interval).await;
    }
}

/// Single connection attempt to the hub.
async fn connect_to_hub(config: &NodeClientConfig) -> Result<(), String> {
    let (ws_stream, _response) = connect_async(&config.hub_url)
        .await
        .map_err(|e| format!("WebSocket connect failed: {}", e))?;

    info!("Node client: connected to hub at {}", config.hub_url);

    let (mut sender, mut receiver) = ws_stream.split();

    // ── Step 1: Send Register ─────────────────────────────────────────────
    let register = NodeClientMessage::Register {
        node_id: config.node_id.clone(),
        host: config.host.clone(),
        capabilities: config.capabilities.clone(),
    };
    let register_json = serde_json::to_string(&register)
        .map_err(|e| format!("Failed to serialize Register: {}", e))?;
    sender
        .send(Message::Text(register_json))
        .await
        .map_err(|e| format!("Failed to send Register: {}", e))?;

    // ── Step 2: Wait for Registered ack ───────────────────────────────────
    let ack = tokio::time::timeout(Duration::from_secs(10), receiver.next())
        .await
        .map_err(|_| "Timeout waiting for Registered ack".to_string())?
        .ok_or("Connection closed before Registered ack")?
        .map_err(|e| format!("WS error waiting for ack: {}", e))?;

    if let Message::Text(text) = ack {
        match serde_json::from_str::<NodeServerMessage>(&text) {
            Ok(NodeServerMessage::Registered { node_id }) => {
                info!("Node client: registered as {} on hub", node_id);
            }
            Ok(other) => {
                warn!("Node client: expected Registered, got {:?}", other);
            }
            Err(e) => {
                warn!("Node client: failed to parse ack: {}", e);
            }
        }
    }

    // ── Step 3: Main message loop ─────────────────────────────────────────
    while let Some(msg) = receiver.next().await {
        let text = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Ping(data)) => {
                let _ = sender.send(Message::Pong(data)).await;
                continue;
            }
            Ok(Message::Close(_)) => {
                info!("Node client: hub sent close frame");
                break;
            }
            Ok(_) => continue,
            Err(e) => {
                error!("Node client: WS receive error: {}", e);
                break;
            }
        };

        match serde_json::from_str::<NodeServerMessage>(&text) {
            Ok(NodeServerMessage::Invoke { id, method, params }) => {
                info!("Node client: hub invoke '{}' (id={})", method, id);
                let result = handle_hub_invoke(config, &method, &params).await;
                let response = match result {
                    Ok(val) => NodeClientMessage::InvokeResult {
                        id,
                        result: val,
                        error: None,
                    },
                    Err(e) => NodeClientMessage::InvokeResult {
                        id,
                        result: Value::Null,
                        error: Some(e),
                    },
                };
                let json = serde_json::to_string(&response).unwrap_or_default();
                if sender.send(Message::Text(json)).await.is_err() {
                    error!("Node client: failed to send invoke result");
                    break;
                }
            }

            Ok(NodeServerMessage::InvokeResult { id, result, error }) => {
                // Hub responding to our invoke (if we sent one)
                info!(
                    "Node client: got invoke result id={}, error={:?}",
                    id, error
                );
                // For now, log it. In future: match pending invokes.
                if let Some(ref err) = error {
                    warn!("Node client: invoke {} failed: {}", id, err);
                } else {
                    info!("Node client: invoke {} result: {}", id, result);
                }
            }

            Ok(NodeServerMessage::Event { event_type, data }) => {
                info!("Node client: event '{}': {}", event_type, data);
                // Deliver to tmux if configured
                if let Some(ref target) = config.tmux_target {
                    let text = data
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&data.to_string())
                        .to_string();
                    let formatted = format!("[Hub event: {}] {}", event_type, text);
                    crate::handlers::forward_to_tmux(target, &formatted).await;
                }
            }

            Ok(NodeServerMessage::Ping) => {
                let pong = serde_json::to_string(&NodeClientMessage::Pong).unwrap_or_default();
                if sender.send(Message::Text(pong)).await.is_err() {
                    break;
                }
            }

            Ok(NodeServerMessage::Registered { .. }) => {
                // Duplicate ack, ignore
            }

            Err(e) => {
                warn!("Node client: failed to parse hub message: {}", e);
            }
        }
    }

    Ok(())
}

/// Handle an invoke request from the hub.
///
/// Supported methods:
/// - `ping` — returns pong
/// - `tmux_send` — sends text to local tmux (params.text, params.session)
/// - `shell` — runs a shell command (params.command)
/// - `status` — returns local gateway status
async fn handle_hub_invoke(
    config: &NodeClientConfig,
    method: &str,
    params: &Value,
) -> Result<Value, String> {
    match method {
        "ping" => Ok(serde_json::json!({"pong": true, "node_id": config.node_id})),

        "tmux_send" => {
            let text = params
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or("Missing params.text")?;
            let session = params
                .get("session")
                .and_then(|v| v.as_str())
                .or(config.tmux_target.as_deref())
                .ok_or("No tmux session specified")?;
            crate::handlers::forward_to_tmux(session, text).await;
            Ok(serde_json::json!({"sent": true, "session": session}))
        }

        "shell" => {
            let command = params
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or("Missing params.command")?;
            let output = tokio::process::Command::new("sh")
                .args(["-c", command])
                .output()
                .await
                .map_err(|e| format!("Shell exec failed: {}", e))?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            Ok(serde_json::json!({
                "exit_code": output.status.code(),
                "stdout": stdout.chars().take(4096).collect::<String>(),
                "stderr": stderr.chars().take(1024).collect::<String>(),
            }))
        }

        "status" => Ok(serde_json::json!({
            "node_id": config.node_id,
            "host": config.host,
            "status": "online",
            "tmux_target": config.tmux_target,
        })),

        _ => Err(format!("Unknown method: {}", method)),
    }
}
