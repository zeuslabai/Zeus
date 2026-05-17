//! client.rs — Pantheon WebSocket client for the TUI.
//!
//! Spawns a Tokio task that maintains the WebSocket connection to the
//! Pantheon server and bridges messages to/from the TUI via mpsc channels.
//!
//! # Usage
//! ```rust,ignore
//! let (client, rx) = PantheonClient::connect("ws://localhost:7777", creds).await?;
//! // rx: mpsc::UnboundedReceiver<ServerEvent> — poll in your TUI event loop
//! // client.send(ClientAction::Msg { ... }) — send from input_bar
//! ```

use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

// Re-export protocol types so callers don't need to import zeus-pantheon-server directly.
pub use zeus_pantheon_server::protocol::{
    ClientMessage, ErrorCode, MessageKind, PermissionTier, PresenceStatus, ServerMessage, UserInfo,
};

// ── Public event type (TUI receives these) ────────────────────────────────────

/// Events emitted by the background client task to the TUI event loop.
#[derive(Debug, Clone)]
pub enum ServerEvent {
    /// Successfully authenticated; auto-joined channels list.
    Connected {
        user_id: String,
        tier: PermissionTier,
        channels: Vec<String>,
    },
    /// Auth rejected.
    AuthFailed(String),
    /// Joined a channel — includes current member list and topic.
    Joined {
        channel: String,
        members: Vec<UserInfo>,
        topic: String,
    },
    /// Left a channel.
    Parted { channel: String },
    /// New message arrived in a channel.
    Message {
        channel: String,
        from: String,
        content: String,
        kind: MessageKind,
        ts: chrono::DateTime<chrono::Utc>,
    },
    /// Someone joined a channel we're in.
    UserJoined { channel: String, user: UserInfo },
    /// Someone left a channel we're in.
    UserParted { channel: String, user_id: String },
    /// Topic changed.
    TopicChanged { channel: String, topic: String, set_by: String },
    /// WHO reply — full member list for a channel.
    WhoReply { channel: String, members: Vec<UserInfo> },
    /// Server-side error.
    ServerError { code: ErrorCode, message: String },
    /// WebSocket connection dropped — TUI should show reconnecting state.
    Disconnected,
}

// ── Actions the TUI sends to the client task ─────────────────────────────────

/// Commands the TUI sends to the background client task.
#[derive(Debug)]
pub enum ClientAction {
    Join(String),
    Part(String),
    Msg { channel: String, content: String, kind: MessageKind },
    SetTopic { channel: String, topic: String },
    Who(String),
    Disconnect,
}

// ── Credentials ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PantheonCredentials {
    pub user_id: String,
    pub display_name: String,
    pub channel_key: String,
    pub agent: bool,
}

// ── Client handle ─────────────────────────────────────────────────────────────

/// Handle to the background WebSocket task.  Clone-safe.
#[derive(Clone)]
pub struct PantheonClient {
    tx: mpsc::UnboundedSender<ClientAction>,
}

impl PantheonClient {
    /// Connect to the Pantheon server and spawn the background task.
    ///
    /// Returns `(client_handle, event_receiver)`.
    /// Poll `event_receiver` each TUI tick to process incoming events.
    pub async fn connect(
        url: &str,
        creds: PantheonCredentials,
    ) -> anyhow::Result<(Self, mpsc::UnboundedReceiver<ServerEvent>)> {
        use tokio_tungstenite::connect_async;
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let (ws_stream, _) = connect_async(url).await?;
        let (mut ws_tx, mut ws_rx) = ws_stream.split();

        // Send Auth immediately
        let nonce = uuid::Uuid::new_v4().to_string();
        let token = compute_token(&creds.channel_key, &creds.user_id, &nonce);
        let auth_msg = ClientMessage::Auth {
            user_id: creds.user_id.clone(),
            display_name: creds.display_name.clone(),
            token,
            nonce,
            agent: creds.agent,
        };
        ws_tx
            .send(Message::Text(serde_json::to_string(&auth_msg)?.into()))
            .await?;

        let (action_tx, mut action_rx) = mpsc::unbounded_channel::<ClientAction>();
        let (event_tx, event_rx) = mpsc::unbounded_channel::<ServerEvent>();

        // ── Background task ───────────────────────────────────────────────────
        tokio::spawn(async move {
            // Wrap ws_tx so we can share it across select branches
            let ws_tx = Arc::new(Mutex::new(ws_tx));

            loop {
                tokio::select! {
                    // Incoming WS message
                    msg = ws_rx.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                match serde_json::from_str::<ServerMessage>(&text) {
                                    Ok(server_msg) => {
                                        if let Some(event) = translate(server_msg) {
                                            let _ = event_tx.send(event);
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!("pantheon: failed to parse server message: {e}");
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(data))) => {
                                let _ = ws_tx.lock().await.send(Message::Pong(data)).await;
                            }
                            Some(Ok(Message::Close(_))) | None => {
                                let _ = event_tx.send(ServerEvent::Disconnected);
                                break;
                            }
                            Some(Ok(_)) => {} // binary / pong — ignore
                            Some(Err(e)) => {
                                tracing::error!("pantheon ws error: {e}");
                                let _ = event_tx.send(ServerEvent::Disconnected);
                                break;
                            }
                        }
                    }

                    // Outgoing action from TUI
                    action = action_rx.recv() => {
                        match action {
                            None | Some(ClientAction::Disconnect) => {
                                let _ = ws_tx.lock().await
                                    .send(Message::Close(None)).await;
                                break;
                            }
                            Some(a) => {
                                if let Some(client_msg) = action_to_client_msg(a) {
                                    if let Ok(json) = serde_json::to_string(&client_msg) {
                                        let _ = ws_tx.lock().await
                                            .send(Message::Text(json.into())).await;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        Ok((Self { tx: action_tx }, event_rx))
    }

    /// Send a TUI action to the background task (non-blocking).
    pub fn send(&self, action: ClientAction) {
        let _ = self.tx.send(action);
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Translate a `ServerMessage` into a `ServerEvent` the TUI cares about.
fn translate(msg: ServerMessage) -> Option<ServerEvent> {
    Some(match msg {
        ServerMessage::AuthOk { user_id, tier, channels } => {
            ServerEvent::Connected { user_id, tier, channels }
        }
        ServerMessage::AuthErr { reason } => ServerEvent::AuthFailed(reason),
        ServerMessage::Joined { channel, members, topic } => {
            ServerEvent::Joined { channel, members, topic }
        }
        ServerMessage::Parted { channel } => ServerEvent::Parted { channel },
        ServerMessage::Msg { channel, from, content, message_type, ts, .. } => {
            ServerEvent::Message {
                channel,
                from: from.display_name,
                content,
                kind: message_type,
                ts,
            }
        }
        ServerMessage::PresenceJoin { channel, user } => {
            ServerEvent::UserJoined { channel, user }
        }
        ServerMessage::PresencePart { channel, user_id } => {
            ServerEvent::UserParted { channel, user_id }
        }
        ServerMessage::TopicUpdate { channel, topic, set_by } => {
            ServerEvent::TopicChanged { channel, topic, set_by }
        }
        ServerMessage::WhoReply { channel, members } => {
            ServerEvent::WhoReply { channel, members }
        }
        ServerMessage::Err { code, message } => {
            ServerEvent::ServerError { code, message }
        }
        // Pong and Presence status — no TUI action needed
        ServerMessage::Motd { text } => ServerEvent::Message {
            channel: "#system".into(),
            from: "server".into(),
            content: text,
            kind: MessageKind::System,
            ts: chrono::Utc::now(),
        },
        ServerMessage::Pong { .. } | ServerMessage::Presence { .. } => return None,
    })
}

/// Map a `ClientAction` to a `ClientMessage` for the wire.
fn action_to_client_msg(action: ClientAction) -> Option<ClientMessage> {
    Some(match action {
        ClientAction::Join(channel) => ClientMessage::Join { channel },
        ClientAction::Part(channel) => ClientMessage::Part { channel },
        ClientAction::Msg { channel, content, kind } => {
            ClientMessage::Msg { channel, content, message_type: kind }
        }
        ClientAction::SetTopic { channel, topic } => {
            ClientMessage::Topic { channel, topic }
        }
        ClientAction::Who(channel) => ClientMessage::Who { channel },
        ClientAction::Disconnect => return None,
    })
}

/// HMAC-SHA256(key, user_id + ":" + nonce) — must match server's `auth.rs`.
fn compute_token(channel_key: &str, user_id: &str, nonce: &str) -> String {
    use sha2::Sha256;
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(channel_key.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(format!("{}:{}", user_id, nonce).as_bytes());
    hex::encode(mac.finalize().into_bytes())
}
