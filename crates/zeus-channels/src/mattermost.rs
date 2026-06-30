//! Mattermost channel adapter
//!
//! Provides Mattermost messaging support via REST API and WebSocket.
//! Supports both personal access tokens and bot accounts.

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, RwLock, mpsc};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use zeus_core::{Error, Result};

/// Mattermost channel adapter
pub struct MattermostAdapter {
    connected: Arc<AtomicBool>,
    config: MattermostConfig,
    shutdown: Arc<Notify>,
    /// Shared HTTP client (connection pooling)
    client: reqwest::Client,
    /// Current user ID (bot ID)
    user_id: RwLock<Option<String>>,
    /// Handle to the receive task
    task_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
}

impl MattermostAdapter {
    /// Create a new Mattermost adapter
    pub async fn new(config: MattermostConfig) -> Result<Self> {
        if config.server_url.is_empty() {
            return Err(Error::Config("Mattermost server_url is required".into()));
        }
        if config.token.is_empty() {
            return Err(Error::Config("Mattermost token is required".into()));
        }

        tracing::info!(server = %config.server_url, "Mattermost adapter created");

        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            config,
            client: reqwest::Client::new(),
            shutdown: Arc::new(Notify::new()),
            user_id: RwLock::new(None),
            task_handle: RwLock::new(None),
        })
    }

    /// Get the current user (bot) info
    async fn get_me(&self) -> Result<String> {
        let client = &self.client;
        let url = format!(
            "{}/api/v4/users/me",
            self.config.server_url.trim_end_matches('/')
        );

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.config.token))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to get user info: {}", e)))?;

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Failed to parse response: {}", e)))?;

        let user_id = body
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| Error::Channel("No user ID in response".into()))?;

        *self.user_id.write().await = Some(user_id.clone());
        Ok(user_id)
    }

    /// Start the WebSocket receive loop
    async fn start_websocket(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        let ws_url = format!(
            "{}/api/v4/websocket",
            self.config
                .server_url
                .trim_end_matches('/')
                .replace("http://", "ws://")
                .replace("https://", "wss://")
        );

        tracing::info!(url = %ws_url, "Connecting to Mattermost WebSocket");

        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .map_err(|e| Error::Channel(format!("WebSocket connection failed: {}", e)))?;

        let (mut write, mut read) = ws_stream.split();

        // Send authentication
        let auth_msg = serde_json::json!({
            "seq": 1,
            "action": "authentication_challenge",
            "data": {
                "token": self.config.token
            }
        });
        write
            .send(WsMessage::Text(auth_msg.to_string()))
            .await
            .map_err(|e| Error::Channel(format!("Failed to authenticate: {}", e)))?;

        let connected = self.connected.clone();
        let shutdown = self.shutdown.clone();
        let my_user_id = self.user_id.read().await.clone();

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.notified() => {
                        tracing::info!("Mattermost WebSocket shutting down");
                        break;
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(WsMessage::Text(text))) => {
                                if let Err(e) = Self::handle_ws_message(
                                    &text,
                                    &tx,
                                    my_user_id.as_deref(),
                                ).await {
                                    tracing::error!(error = %e, "Error handling Mattermost message");
                                }
                            }
                            Some(Ok(WsMessage::Ping(data))) => {
                                // Pong is handled automatically by tokio-tungstenite
                                tracing::trace!("Received ping: {:?}", data);
                            }
                            Some(Ok(WsMessage::Close(_))) => {
                                tracing::info!("Mattermost WebSocket closed");
                                break;
                            }
                            Some(Err(e)) => {
                                tracing::error!(error = %e, "WebSocket error");
                                break;
                            }
                            None => {
                                tracing::info!("WebSocket stream ended");
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
            connected.store(false, Ordering::SeqCst);
        });

        *self.task_handle.write().await = Some(handle);
        self.connected.store(true, Ordering::SeqCst);
        tracing::info!("Mattermost WebSocket connected");

        Ok(())
    }

    /// Handle a WebSocket message
    async fn handle_ws_message(
        text: &str,
        tx: &mpsc::Sender<ChannelMessage>,
        my_user_id: Option<&str>,
    ) -> Result<()> {
        let event: MattermostWsEvent = serde_json::from_str(text)
            .map_err(|e| Error::Channel(format!("Failed to parse WS event: {}", e)))?;

        // Only process posted events
        if event.event != "posted" {
            return Ok(());
        }

        let data = event
            .data
            .ok_or_else(|| Error::Channel("No data in event".into()))?;
        let post_str = data
            .get("post")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Channel("No post in data".into()))?;

        let post: MattermostPost = serde_json::from_str(post_str)
            .map_err(|e| Error::Channel(format!("Failed to parse post: {}", e)))?;

        // Skip our own messages
        if let Some(my_id) = my_user_id
            && post.user_id == my_id
        {
            return Ok(());
        }

        let source = ChannelSource::with_chat("mattermost", &post.user_id, &post.channel_id);

        // Populate thread context if this post is a reply in a thread
        let thread = if !post.root_id.is_empty() {
            Some(crate::threading::ThreadContext::new(&post.root_id)
                .with_parent_message(&post.root_id))
        } else {
            None
        };

        let mut message = ChannelMessage::new(source, post.message)
            .with_platform_message_id(&post.id);

        if let Some(thread_ctx) = thread {
            message = message.with_thread(thread_ctx);
        }

        tx.send(message)
            .await
            .map_err(|e| Error::Channel(format!("Failed to forward message: {}", e)))?;

        Ok(())
    }

    /// Send a message to a channel, optionally replying in a thread
    pub async fn send_message(&self, channel_id: &str, text: &str) -> Result<()> {
        self.send_message_threaded(channel_id, text, None).await
    }

    /// Send a message, optionally as a threaded reply.
    /// `root_id` is the ID of the root post to reply to (Mattermost thread parent).
    pub async fn send_message_threaded(
        &self,
        channel_id: &str,
        text: &str,
        root_id: Option<&str>,
    ) -> Result<()> {
        let client = &self.client;
        let url = format!(
            "{}/api/v4/posts",
            self.config.server_url.trim_end_matches('/')
        );

        let mut body = serde_json::json!({
            "channel_id": channel_id,
            "message": text
        });

        // Attach root_id if provided — makes this a threaded reply
        if let Some(rid) = root_id {
            if !rid.is_empty() {
                body["root_id"] = serde_json::Value::String(rid.to_string());
            }
        }

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.token))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to send message: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "Mattermost API error {}: {}",
                status, body
            )));
        }

        tracing::info!(
            channel_id = %channel_id,
            threaded = root_id.is_some(),
            "Mattermost message sent"
        );
        Ok(())
    }

    /// Execute a Mattermost slash command in a channel
    pub async fn execute_command(&self, channel_id: &str, command: &str) -> Result<String> {
        let client = &self.client;
        let url = format!(
            "{}/api/v4/commands/execute",
            self.config.server_url.trim_end_matches('/')
        );

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.token))
            .json(&serde_json::json!({
                "channel_id": channel_id,
                "command": command
            }))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to execute command: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "Mattermost command error {}: {}",
                status, body
            )));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Failed to parse command response: {}", e)))?;

        tracing::info!(channel_id = %channel_id, command = %command, "Mattermost command executed");
        Ok(result.to_string())
    }

    /// Test the connection
    pub async fn test_connection(&self) -> Result<bool> {
        let user_id = self.get_me().await?;
        tracing::info!(user_id = %user_id, "Mattermost connection verified");
        Ok(true)
    }
}

#[async_trait]
impl ChannelAdapter for MattermostAdapter {
    fn channel_type(&self) -> &'static str {
        "mattermost"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::WebSocket
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        // Get our user ID first
        self.get_me().await?;

        // Start WebSocket connection
        self.start_websocket(tx).await?;

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.shutdown.notify_one();

        if let Some(handle) = self.task_handle.write().await.take() {
            let _ = handle.await;
        }

        tracing::info!("Mattermost adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "mattermost" {
            return Err(Error::channel("Invalid channel source for Mattermost"));
        }

        let channel_id = to
            .chat_id
            .as_deref()
            .ok_or_else(|| Error::channel("Mattermost send requires a channel_id"))?;

        // Honour thread_id if set on the source (acts as root_id for the reply)
        self.send_message_threaded(channel_id, content, to.thread_id.as_deref())
            .await
    }

    async fn send_threaded(
        &self,
        to: &ChannelSource,
        content: &str,
        opts: &crate::threading::ThreadedReplyOptions,
    ) -> Result<()> {
        if to.channel_type() != "mattermost" {
            return Err(Error::channel("Invalid channel source for Mattermost"));
        }

        let channel_id = to
            .chat_id
            .as_deref()
            .ok_or_else(|| Error::channel("Mattermost send_threaded requires a channel_id"))?;

        // Prefer explicit thread_id from opts, then fall back to source thread_id
        let root_id = opts
            .thread_id
            .as_deref()
            .or(to.thread_id.as_deref());

        self.send_message_threaded(channel_id, content, root_id).await
    }

    fn supports_threading(&self) -> bool {
        true
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

/// Mattermost WebSocket event
#[derive(Debug, Clone, Deserialize)]
pub struct MattermostWsEvent {
    /// Event type
    pub event: String,
    /// Event data
    pub data: Option<serde_json::Value>,
    /// Broadcast info
    pub broadcast: Option<serde_json::Value>,
    /// Sequence number
    pub seq: Option<i64>,
}

/// Mattermost post
#[derive(Debug, Clone, Deserialize)]
pub struct MattermostPost {
    /// Post ID
    pub id: String,
    /// Channel ID
    pub channel_id: String,
    /// User ID
    pub user_id: String,
    /// Message content
    pub message: String,
    /// Create timestamp
    pub create_at: Option<i64>,
    /// Root post ID — set when this post is a reply in a thread.
    /// Empty string means this is a root post.
    #[serde(default)]
    pub root_id: String,
}

/// Mattermost configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MattermostConfig {
    /// Server URL (e.g., https://mattermost.example.com)
    #[serde(default)]
    pub server_url: String,
    /// Personal access token or bot token
    #[serde(default, skip_serializing)]
    pub token: String,
    /// Team ID (optional, for team-specific operations)
    #[serde(default)]
    pub team_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mattermost_config_default() {
        let config = MattermostConfig::default();
        assert!(config.server_url.is_empty());
        assert!(config.token.is_empty());
    }

    #[tokio::test]
    async fn test_mattermost_adapter_validation() {
        // Empty config should fail
        let config = MattermostConfig::default();
        assert!(MattermostAdapter::new(config).await.is_err());

        // Missing token should fail
        let config = MattermostConfig {
            server_url: "https://mattermost.example.com".to_string(),
            ..Default::default()
        };
        assert!(MattermostAdapter::new(config).await.is_err());

        // Valid config should succeed
        let config = MattermostConfig {
            server_url: "https://mattermost.example.com".to_string(),
            token: "test-token".to_string(),
            ..Default::default()
        };
        assert!(MattermostAdapter::new(config).await.is_ok());
    }

    #[tokio::test]
    async fn test_mattermost_adapter_lifecycle() {
        let config = MattermostConfig {
            server_url: "https://mattermost.example.com".to_string(),
            token: "test-token".to_string(),
            ..Default::default()
        };

        let adapter = MattermostAdapter::new(config)
            .await
            .expect("MattermostAdapter::new should succeed");
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "mattermost");
    }

    #[test]
    fn test_mattermost_ws_event_parsing() {
        let json = r#"{
            "event": "posted",
            "data": {"post": "{\"id\":\"abc\",\"channel_id\":\"ch1\",\"user_id\":\"u1\",\"message\":\"Hello\"}"},
            "seq": 1
        }"#;

        let event: MattermostWsEvent =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(event.event, "posted");
        assert!(event.data.is_some());
    }

    #[test]
    fn test_mattermost_post_parsing() {
        let json = r#"{
            "id": "post123",
            "channel_id": "channel456",
            "user_id": "user789",
            "message": "Hello Mattermost!"
        }"#;

        let post: MattermostPost = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(post.id, "post123");
        assert_eq!(post.message, "Hello Mattermost!");
    }
}
