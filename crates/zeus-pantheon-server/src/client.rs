//! Pantheon WebSocket client — connects to a standalone Pantheon server.
//!
//! Used by the gateway for auto-connect on boot and by the TUI for real-time chat.

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::auth::compute_token;
use crate::protocol::{ClientMessage, ServerMessage};

/// A connected Pantheon client that can send/receive messages.
pub struct PantheonClient {
    /// Send client messages to the server.
    tx: mpsc::Sender<ClientMessage>,
    /// Receive server messages.
    rx: mpsc::Receiver<ServerMessage>,
}

/// Configuration for connecting to a Pantheon server.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub server_url: String,
    pub user_id: String,
    pub display_name: String,
    pub channel_key: String,
    pub is_agent: bool,
    pub auto_join: Vec<String>,
}

impl PantheonClient {
    /// Connect to a Pantheon server and authenticate.
    ///
    /// Returns a connected client with send/receive channels.
    /// Spawns a background task to handle the WebSocket connection.
    pub async fn connect(config: ClientConfig) -> anyhow::Result<Self> {
        let url = if config.server_url.starts_with("ws") {
            config.server_url.clone()
        } else {
            format!("ws://{}", config.server_url)
        };

        let (ws, _) = connect_async(&url).await
            .map_err(|e| anyhow::anyhow!("Failed to connect to Pantheon at {}: {}", url, e))?;

        info!("Connected to Pantheon server at {}", url);

        let (mut ws_sink, mut ws_stream) = ws.split();

        // Channels for communication with the caller
        let (outbound_tx, mut outbound_rx) = mpsc::channel::<ClientMessage>(64);
        let (inbound_tx, inbound_rx) = mpsc::channel::<ServerMessage>(256);

        // Send AUTH immediately
        let nonce = uuid::Uuid::new_v4().to_string();
        let token = compute_token(&config.channel_key, &config.user_id, &nonce);
        let auth_msg = ClientMessage::Auth {
            user_id: config.user_id.clone(),
            display_name: config.display_name.clone(),
            token,
            nonce,
            agent: config.is_agent,
        };
        let auth_json = serde_json::to_string(&auth_msg)?;
        ws_sink.send(Message::Text(auth_json.into())).await?;

        // Background task: read from WS → inbound_tx, read from outbound_rx → WS
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Server → client
                    msg = ws_stream.next() => {
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                match serde_json::from_str::<ServerMessage>(&text) {
                                    Ok(server_msg) => {
                                        if inbound_tx.send(server_msg).await.is_err() {
                                            break; // receiver dropped
                                        }
                                    }
                                    Err(e) => warn!("Pantheon: failed to parse server message: {}", e),
                                }
                            }
                            Some(Ok(Message::Close(_))) => {
                                info!("Pantheon server closed connection");
                                break;
                            }
                            Some(Ok(Message::Ping(d))) => {
                                let _ = ws_sink.send(Message::Pong(d)).await;
                            }
                            Some(Err(e)) => {
                                error!("Pantheon WS error: {}", e);
                                break;
                            }
                            None => break, // stream ended
                            _ => {}
                        }
                    }
                    // Client → server
                    outbound = outbound_rx.recv() => {
                        match outbound {
                            Some(client_msg) => {
                                if let Ok(json) = serde_json::to_string(&client_msg) {
                                    if let Err(e) = ws_sink.send(Message::Text(json.into())).await {
                                        error!("Pantheon: failed to send: {}", e);
                                        break;
                                    }
                                }
                            }
                            None => break, // sender dropped
                        }
                    }
                }
            }
        });

        Ok(Self {
            tx: outbound_tx,
            rx: inbound_rx,
        })
    }

    /// Send a message to a channel.
    pub async fn send_msg(&self, channel: &str, content: &str) -> anyhow::Result<()> {
        self.tx.send(ClientMessage::Msg {
            channel: channel.into(),
            content: content.into(),
            message_type: Default::default(),
        }).await.map_err(|e| anyhow::anyhow!("Send failed: {}", e))
    }

    /// Join a channel.
    pub async fn join(&self, channel: &str) -> anyhow::Result<()> {
        self.tx.send(ClientMessage::Join {
            channel: channel.into(),
        }).await.map_err(|e| anyhow::anyhow!("Join failed: {}", e))
    }

    /// Part (leave) a channel.
    pub async fn part(&self, channel: &str) -> anyhow::Result<()> {
        self.tx.send(ClientMessage::Part {
            channel: channel.into(),
        }).await.map_err(|e| anyhow::anyhow!("Part failed: {}", e))
    }

    /// Send a raw client message.
    pub async fn send_raw(&self, msg: ClientMessage) -> anyhow::Result<()> {
        self.tx.send(msg).await.map_err(|e| anyhow::anyhow!("Send failed: {}", e))
    }

    /// Receive the next server message (blocks until available or disconnected).
    pub async fn recv(&mut self) -> Option<ServerMessage> {
        self.rx.recv().await
    }

    /// Try to receive a server message without blocking.
    pub fn try_recv(&mut self) -> Option<ServerMessage> {
        self.rx.try_recv().ok()
    }
}

/// Connect to Pantheon with auto-reconnect. Returns a handle for sending messages.
/// Spawns a background task that reconnects on disconnect.
pub fn spawn_auto_connect(
    config: ClientConfig,
    inbound_tx: mpsc::Sender<ServerMessage>,
) -> mpsc::Sender<ClientMessage> {
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<ClientMessage>(64);

    tokio::spawn(async move {
        loop {
            info!("Pantheon: connecting to {}...", config.server_url);
            match PantheonClient::connect(config.clone()).await {
                Ok(mut client) => {
                    info!("Pantheon: connected and authenticated");

                    // Auto-join configured channels
                    for ch in &config.auto_join {
                        if let Err(e) = client.join(ch).await {
                            warn!("Pantheon: failed to join {}: {}", ch, e);
                        }
                    }

                    // Relay loop
                    loop {
                        tokio::select! {
                            server_msg = client.recv() => {
                                match server_msg {
                                    Some(msg) => {
                                        if inbound_tx.send(msg).await.is_err() {
                                            return; // caller dropped
                                        }
                                    }
                                    None => {
                                        warn!("Pantheon: server disconnected");
                                        break; // reconnect
                                    }
                                }
                            }
                            outbound = outbound_rx.recv() => {
                                match outbound {
                                    Some(msg) => {
                                        if let Err(e) = client.send_raw(msg).await {
                                            warn!("Pantheon: send failed: {}", e);
                                            break; // reconnect
                                        }
                                    }
                                    None => return, // caller dropped
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Pantheon: connection failed: {}", e);
                }
            }

            // Reconnect delay
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });

    outbound_tx
}
