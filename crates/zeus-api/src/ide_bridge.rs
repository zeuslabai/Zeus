//! IDE Bridge — WebSocket server for VS Code / JetBrains remote control.
//!
//! Exposes `GET /v1/ide/ws` — IDEs connect here and can:
//! - Send commands to the active agent session
//! - Receive streaming output in real time
//! - Query session state (current file context, active tools, etc.)
//!
//! ## Protocol (JSON over WebSocket)
//!
//! ### IDE → Zeus
//! ```json
//! {"type": "ide_hello", "client": "vscode", "version": "1.0", "workspace": "/path/to/project"}
//! {"type": "ide_command", "session_id": "optional", "message": "refactor this function"}
//! {"type": "ide_context", "files": [{"path": "src/main.rs", "content": "..."}]}
//! {"type": "ide_interrupt"}
//! {"type": "ping"}
//! ```
//!
//! ### Zeus → IDE
//! ```json
//! {"type": "ide_welcome", "agent_id": "zeus107", "session_id": "sess-abc"}
//! {"type": "ide_text_chunk", "chunk": "Here's my analysis..."}
//! {"type": "ide_tool_call", "name": "read_file", "args": {"path": "src/main.rs"}}
//! {"type": "ide_tool_result", "name": "read_file", "success": true, "output": "..."}
//! {"type": "ide_done", "session_id": "sess-abc"}
//! {"type": "ide_error", "message": "..."}
//! {"type": "pong"}
//! ```

use axum::{
    extract::{State, WebSocketUpgrade, ws::{Message, WebSocket}},
    response::Response,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::SharedState;

// ─────────────────────────────────────────────────────────────────────────────
// Protocol types
// ─────────────────────────────────────────────────────────────────────────────

/// Messages sent from the IDE client to Zeus.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IdeInbound {
    /// Initial handshake — IDE identifies itself and its workspace root.
    IdeHello {
        client: String,          // "vscode" | "jetbrains" | "neovim" | ...
        version: String,
        #[serde(default)]
        workspace: Option<String>,
    },
    /// Send a message/command into an agent session.
    IdeCommand {
        message: String,
        #[serde(default)]
        session_id: Option<String>,
    },
    /// Push file context — injected as a system message before next command.
    IdeContext {
        files: Vec<IdeFile>,
    },
    /// Cancel in-flight agent work.
    IdeInterrupt,
    /// Keepalive.
    Ping,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct IdeFile {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub language: Option<String>,
}

/// Messages sent from Zeus to the IDE client.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IdeOutbound {
    IdeWelcome {
        agent_id: String,
        session_id: String,
        capabilities: Vec<String>,
    },
    IdeTextChunk {
        chunk: String,
        session_id: String,
    },
    IdeToolCall {
        name: String,
        args: Value,
        session_id: String,
    },
    IdeToolResult {
        name: String,
        success: bool,
        output: String,
        session_id: String,
    },
    IdeDone {
        session_id: String,
    },
    IdeError {
        message: String,
    },
    Pong,
}

// ─────────────────────────────────────────────────────────────────────────────
// Connection state
// ─────────────────────────────────────────────────────────────────────────────

pub struct IdeBridgeConn {
    /// IDE client identifier ("vscode", "jetbrains", etc.)
    pub client: String,
    /// Workspace root path sent by the IDE on hello.
    pub workspace: Option<String>,
    /// Active session ID (resolved at first command if not supplied).
    pub session_id: Option<String>,
    /// Pending file context to inject before next command.
    pub pending_context: Vec<IdeFile>,
}

impl IdeBridgeConn {
    fn new() -> Self {
        Self {
            client: "unknown".to_string(),
            workspace: None,
            session_id: None,
            pending_context: Vec::new(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Axum handler — upgrade to WebSocket
// ─────────────────────────────────────────────────────────────────────────────

pub async fn ide_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
) -> Response {
    ws.on_upgrade(move |socket| handle_ide_socket(socket, state))
}

async fn handle_ide_socket(socket: WebSocket, state: SharedState) {
    let (mut sink, mut stream) = socket.split();

    // Channel for sending outbound messages from async tasks back to the sink.
    let (tx, mut rx) = mpsc::channel::<IdeOutbound>(64);

    // Spawn a writer task so we can push from multiple places.
    let write_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let json = match serde_json::to_string(&msg) {
                Ok(s) => s,
                Err(e) => {
                    error!("[ide_bridge] serialize error: {e}");
                    continue;
                }
            };
            if sink.send(Message::Text(json.into())).await.is_err() {
                break; // client disconnected
            }
        }
    });

    let mut conn = IdeBridgeConn::new();

    while let Some(Ok(msg)) = stream.next().await {
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => break,
            _ => continue,
        };

        let inbound: IdeInbound = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                warn!("[ide_bridge] bad message: {e}");
                let _ = tx.send(IdeOutbound::IdeError {
                    message: format!("parse error: {e}"),
                }).await;
                continue;
            }
        };

        handle_inbound(inbound, &mut conn, &tx, &state).await;
    }

    // Clean up writer task.
    write_task.abort();
    info!("[ide_bridge] client '{}' disconnected", conn.client);
}

// ─────────────────────────────────────────────────────────────────────────────
// Message dispatch
// ─────────────────────────────────────────────────────────────────────────────

async fn handle_inbound(
    msg: IdeInbound,
    conn: &mut IdeBridgeConn,
    tx: &mpsc::Sender<IdeOutbound>,
    state: &SharedState,
) {
    match msg {
        IdeInbound::IdeHello { client, version, workspace } => {
            info!("[ide_bridge] hello from {client} v{version} @ {:?}", workspace);
            conn.client = client;
            conn.workspace = workspace;

            // Resolve or create a session ID.
            let session_id = conn.session_id
                .clone()
                .unwrap_or_else(|| format!("ide-{}", uuid_short()));

            conn.session_id = Some(session_id.clone());

            let _ = tx.send(IdeOutbound::IdeWelcome {
                agent_id: { let sg = state.read().await; sg.config.agents.first().map(|a| a.id.clone()).unwrap_or_else(|| "<unnamed agent>".to_string()) },
                session_id,
                capabilities: vec![
                    "stream".to_string(),
                    "context_injection".to_string(),
                    "interrupt".to_string(),
                ],
            }).await;
        }

        IdeInbound::IdeContext { files } => {
            debug!("[ide_bridge] context push: {} file(s)", files.len());
            conn.pending_context = files;
        }

        IdeInbound::IdeCommand { message, session_id } => {
            // Use supplied session ID or fall back to conn default.
            let sid = session_id
                .or_else(|| conn.session_id.clone())
                .unwrap_or_else(|| format!("ide-{}", uuid_short()));

            conn.session_id = Some(sid.clone());

            // Build the full message, prepending any pending file context.
            let full_message = if conn.pending_context.is_empty() {
                message
            } else {
                let context_block = conn.pending_context
                    .iter()
                    .map(|f| {
                        let lang = f.language.as_deref().unwrap_or("");
                        format!("```{lang}\n// {}\n{}\n```", f.path, f.content)
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                conn.pending_context.clear();
                format!("{context_block}\n\n{message}")
            };

            info!("[ide_bridge] command in session {sid}: {}", &full_message[..zeus_core::floor_char_boundary(&full_message, 80)]);

            // Forward to the agent via the gateway inbox.
            dispatch_to_agent(full_message, sid.clone(), tx, state).await;
        }

        IdeInbound::IdeInterrupt => {
            info!("[ide_bridge] interrupt requested");
            // Signal cancellation via the shared state's cancel broadcast if available.
            // For now, log and ack — full interrupt wiring requires a cancel token per session.
            let _ = tx.send(IdeOutbound::IdeError {
                message: "interrupt acknowledged (not yet wired to cancel token)".to_string(),
            }).await;
        }

        IdeInbound::Ping => {
            let _ = tx.send(IdeOutbound::Pong).await;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Agent dispatch — send message to gateway agent loop and stream back results
// ─────────────────────────────────────────────────────────────────────────────

async fn dispatch_to_agent(
    message: String,
    session_id: String,
    tx: &mpsc::Sender<IdeOutbound>,
    state: &SharedState,
) {
    use zeus_agent::AgentEvent;

    // Build an agent for the IDE session
    let state_guard = state.read().await;
    let llm = match zeus_llm::LlmClient::from_config(&state_guard.config) {
        Ok(l) => l,
        Err(e) => {
            let _ = tx.send(IdeOutbound::IdeError {
                message: format!("LLM init failed: {e}"),
            }).await;
            return;
        }
    };
    let workspace = state_guard.workspace.clone();
    let config = state_guard.config.clone();
    drop(state_guard);
    let session = zeus_session::Session::new(&session_id);
    let agent = zeus_agent::Agent::new(config, llm, workspace, session, None);

    let (_event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(128);
    let tx_clone = tx.clone();
    let sid_clone = session_id.clone();

    // Spawn event forwarder.
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let outbound = match event {
                AgentEvent::TextChunk(chunk) => IdeOutbound::IdeTextChunk {
                    chunk,
                    session_id: sid_clone.clone(),
                },
                AgentEvent::ToolCall { name, args } => IdeOutbound::IdeToolCall {
                    name,
                    args,
                    session_id: sid_clone.clone(),
                },
                AgentEvent::ToolResult { name, success, output } => IdeOutbound::IdeToolResult {
                    name,
                    success,
                    output,
                    session_id: sid_clone.clone(),
                },
                AgentEvent::Finished { iterations: 0 } => IdeOutbound::IdeDone {
                    session_id: sid_clone.clone(),
                },
                AgentEvent::Error(e) => IdeOutbound::IdeError { message: e },
                _ => continue,
            };
            if tx_clone.send(outbound).await.is_err() {
                break;
            }
        }
    });

    // Run the agent — this blocks until done.
    let mut agent = agent;
    match agent.run(&message).await {
        Ok(response) => {
            let _ = tx.send(IdeOutbound::IdeTextChunk { chunk: response, session_id: session_id.clone() }).await;
            let _ = tx.send(IdeOutbound::IdeDone { session_id }).await;
        }
        Err(e) => {
            let _ = tx.send(IdeOutbound::IdeError {
                message: format!("agent error: {e}"),
            }).await;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn uuid_short() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{:08x}", nanos)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ide_file_serializes() {
        let f = IdeFile {
            path: "src/main.rs".to_string(),
            content: "fn main() {}".to_string(),
            language: Some("rust".to_string()),
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("src/main.rs"));
    }

    #[test]
    fn test_inbound_parse_hello() {
        let raw = r#"{"type":"ide_hello","client":"vscode","version":"1.2.3"}"#;
        let msg: IdeInbound = serde_json::from_str(raw).unwrap();
        match msg {
            IdeInbound::IdeHello { client, .. } => assert_eq!(client, "vscode"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_inbound_parse_command() {
        let raw = r#"{"type":"ide_command","message":"refactor this","session_id":"sess-123"}"#;
        let msg: IdeInbound = serde_json::from_str(raw).unwrap();
        match msg {
            IdeInbound::IdeCommand { message, session_id } => {
                assert_eq!(message, "refactor this");
                assert_eq!(session_id, Some("sess-123".to_string()));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_inbound_parse_context() {
        let raw = r#"{"type":"ide_context","files":[{"path":"a.rs","content":"x"}]}"#;
        let msg: IdeInbound = serde_json::from_str(raw).unwrap();
        match msg {
            IdeInbound::IdeContext { files } => {
                assert_eq!(files.len(), 1);
                assert_eq!(files[0].path, "a.rs");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_outbound_welcome_serializes() {
        let msg = IdeOutbound::IdeWelcome {
            agent_id: "zeus107".to_string(),
            session_id: "ide-abc".to_string(),
            capabilities: vec!["stream".to_string()],
        };
        let s = serde_json::to_string(&msg).unwrap();
        assert!(s.contains("ide_welcome"));
        assert!(s.contains("zeus107"));
    }

    #[test]
    fn test_uuid_short_nonempty() {
        let id = uuid_short();
        assert!(!id.is_empty());
        assert_eq!(id.len(), 8);
    }
}
