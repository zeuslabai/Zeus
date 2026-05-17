//! BlueBubbles channel adapter
//!
//! Provides iMessage access via BlueBubbles server (alternative to direct AppleScript).
//! Uses REST API and WebSocket for real-time updates.

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, RwLock, mpsc};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use zeus_core::{Error, Result};

/// BlueBubbles channel adapter
pub struct BlueBubblesAdapter {
    connected: Arc<AtomicBool>,
    config: BlueBubblesConfig,
    shutdown: Arc<Notify>,
    /// Shared HTTP client (connection pooling)
    client: reqwest::Client,
    /// Handle to the receive task
    task_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
}

impl BlueBubblesAdapter {
    /// Create a new BlueBubbles adapter
    pub async fn new(config: BlueBubblesConfig) -> Result<Self> {
        if config.server_url.is_empty() {
            return Err(Error::Config("BlueBubbles server_url is required".into()));
        }
        if config.password.is_empty() {
            return Err(Error::Config("BlueBubbles password is required".into()));
        }

        tracing::info!(server = %config.server_url, "BlueBubbles adapter created");

        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            config,
            shutdown: Arc::new(Notify::new()),
            client: reqwest::Client::new(),
            task_handle: RwLock::new(None),
        })
    }

    /// Send a message
    pub async fn send_message(&self, chat_guid: &str, text: &str) -> Result<()> {
        let client = &self.client;
        let url = format!(
            "{}/api/v1/message/text",
            self.config.server_url.trim_end_matches('/')
        );

        let response = client
            .post(&url)
            .query(&[("password", &self.config.password)])
            .json(&serde_json::json!({
                "chatGuid": chat_guid,
                "message": text
            }))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to send BlueBubbles message: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "BlueBubbles API error {}: {}",
                status, body
            )));
        }

        tracing::info!(chat = %chat_guid, "BlueBubbles message sent");
        Ok(())
    }

    /// Start the WebSocket connection for real-time updates
    async fn start_websocket(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        let base_ws = self
            .config
            .server_url
            .trim_end_matches('/')
            .replace("http://", "ws://")
            .replace("https://", "wss://");
        let ws_url = format!(
            "{}/socket.io/?password={}&transport=websocket",
            base_ws, self.config.password
        );
        // Log sanitized URL without the password
        tracing::info!(url = %format!("{}/socket.io/?password=<redacted>&transport=websocket", base_ws), "Connecting to BlueBubbles WebSocket");

        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .map_err(|e| Error::Channel(format!("WebSocket connection failed: {}", e)))?;

        let (_, mut read) = ws_stream.split();
        let connected = self.connected.clone();
        let shutdown = self.shutdown.clone();

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.notified() => {
                        tracing::info!("BlueBubbles WebSocket shutting down");
                        break;
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(WsMessage::Text(text))) => {
                                if let Err(e) = Self::handle_ws_message(&text, &tx).await {
                                    tracing::error!(error = %e, "Error handling BlueBubbles message");
                                }
                            }
                            Some(Ok(WsMessage::Close(_))) => {
                                tracing::info!("BlueBubbles WebSocket closed");
                                break;
                            }
                            Some(Err(e)) => {
                                tracing::error!(error = %e, "WebSocket error");
                                break;
                            }
                            None => break,
                            _ => {}
                        }
                    }
                }
            }
            connected.store(false, Ordering::SeqCst);
        });

        *self.task_handle.write().await = Some(handle);
        self.connected.store(true, Ordering::SeqCst);

        Ok(())
    }

    /// Handle a WebSocket message
    async fn handle_ws_message(text: &str, tx: &mpsc::Sender<ChannelMessage>) -> Result<()> {
        // Socket.IO format: 42["event", data]
        if !text.starts_with("42[") {
            return Ok(());
        }

        let json_part = &text[2..];
        let arr: serde_json::Value = serde_json::from_str(json_part)
            .map_err(|e| Error::Channel(format!("Failed to parse message: {}", e)))?;

        let event = arr.get(0).and_then(|v| v.as_str());
        let data = arr.get(1);

        if event == Some("new-message")
            && let Some(msg_data) = data
        {
            let text = msg_data.get("text").and_then(|t| t.as_str()).unwrap_or("");
            let chat_guid = msg_data
                .get("chats")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("guid"))
                .and_then(|g| g.as_str())
                .unwrap_or("");
            let handle = msg_data
                .get("handle")
                .and_then(|h| h.get("address"))
                .and_then(|a| a.as_str())
                .unwrap_or("");
            let is_from_me = msg_data
                .get("isFromMe")
                .and_then(|f| f.as_bool())
                .unwrap_or(false);

            // Skip messages from ourselves
            if is_from_me {
                return Ok(());
            }

            let source = ChannelSource::with_chat("bluebubbles", handle, chat_guid);
            let message = ChannelMessage::new(source, text.to_string());

            tx.send(message)
                .await
                .map_err(|e| Error::Channel(format!("Failed to forward message: {}", e)))?;
        }

        Ok(())
    }

    /// Test the connection
    pub async fn test_connection(&self) -> Result<bool> {
        let client = &self.client;
        let url = format!(
            "{}/api/v1/server/info",
            self.config.server_url.trim_end_matches('/')
        );

        let response = client
            .get(&url)
            .query(&[("password", &self.config.password)])
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Connection test failed: {}", e)))?;

        if response.status().is_success() {
            tracing::info!("BlueBubbles connection verified");
            Ok(true)
        } else {
            Err(Error::Channel("BlueBubbles authentication failed".into()))
        }
    }
}

#[async_trait]
impl ChannelAdapter for BlueBubblesAdapter {
    fn channel_type(&self) -> &'static str {
        "bluebubbles"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::WebSocket
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        self.start_websocket(tx).await
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.shutdown.notify_one();

        if let Some(handle) = self.task_handle.write().await.take() {
            let _ = handle.await;
        }

        tracing::info!("BlueBubbles adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "bluebubbles" {
            return Err(Error::channel("Invalid channel source for BlueBubbles"));
        }

        let chat_guid = to
            .chat_id
            .as_deref()
            .ok_or_else(|| Error::channel("BlueBubbles send requires a chat_guid"))?;

        self.send_message(chat_guid, content).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

/// BlueBubbles configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlueBubblesConfig {
    /// Server URL (e.g., http://localhost:1234)
    #[serde(default)]
    pub server_url: String,
    /// Server password
    #[serde(default, skip_serializing)]
    pub password: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bluebubbles_config_default() {
        let config = BlueBubblesConfig::default();
        assert!(config.server_url.is_empty());
        assert!(config.password.is_empty());
    }

    #[tokio::test]
    async fn test_bluebubbles_adapter_validation() {
        // Empty config should fail
        let config = BlueBubblesConfig::default();
        assert!(BlueBubblesAdapter::new(config).await.is_err());

        // Missing password should fail
        let config = BlueBubblesConfig {
            server_url: "http://localhost:1234".to_string(),
            ..Default::default()
        };
        assert!(BlueBubblesAdapter::new(config).await.is_err());

        // Valid config should succeed
        let config = BlueBubblesConfig {
            server_url: "http://localhost:1234".to_string(),
            password: "test-password".to_string(),
        };
        assert!(BlueBubblesAdapter::new(config).await.is_ok());
    }

    #[tokio::test]
    async fn test_bluebubbles_adapter_lifecycle() {
        let config = BlueBubblesConfig {
            server_url: "http://localhost:1234".to_string(),
            password: "test-password".to_string(),
        };

        let adapter = BlueBubblesAdapter::new(config)
            .await
            .expect("BlueBubblesAdapter::new should succeed");
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "bluebubbles");
    }
}
