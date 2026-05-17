//! WebChat channel adapter
//!
//! Provides a WebSocket-based browser chat widget for embedding in websites.
//! Accepts incoming WebSocket connections and routes messages bidirectionally.

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, RwLock, mpsc};
use zeus_core::{Error, Result};

/// WebChat channel adapter
pub struct WebChatAdapter {
    connected: Arc<AtomicBool>,
    config: WebChatConfig,
    shutdown: Arc<Notify>,
    /// Active client connections (session_id -> sender)
    clients: Arc<RwLock<HashMap<String, mpsc::Sender<String>>>>,
}

impl WebChatAdapter {
    /// Create a new WebChat adapter
    pub fn new(config: WebChatConfig) -> Self {
        tracing::info!("WebChat adapter created");

        Self {
            connected: Arc::new(AtomicBool::new(false)),
            config,
            shutdown: Arc::new(Notify::new()),
            clients: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new client connection
    pub async fn register_client(&self, session_id: String, sender: mpsc::Sender<String>) {
        self.clients
            .write()
            .await
            .insert(session_id.clone(), sender);
        tracing::info!(session_id = %session_id, "WebChat client registered");
    }

    /// Unregister a client connection
    pub async fn unregister_client(&self, session_id: &str) {
        self.clients.write().await.remove(session_id);
        tracing::info!(session_id = %session_id, "WebChat client unregistered");
    }

    /// Send a message to a specific client
    pub async fn send_to_client(&self, session_id: &str, content: &str) -> Result<()> {
        let clients = self.clients.read().await;
        let sender = clients
            .get(session_id)
            .ok_or_else(|| Error::Channel(format!("WebChat client not found: {}", session_id)))?;

        let message = serde_json::json!({
            "type": "message",
            "content": content,
            "timestamp": chrono::Utc::now().to_rfc3339()
        })
        .to_string();

        sender
            .send(message)
            .await
            .map_err(|e| Error::Channel(format!("Failed to send to WebChat client: {}", e)))?;

        tracing::debug!(session_id = %session_id, "WebChat message sent");
        Ok(())
    }

    /// Broadcast a message to all connected clients
    pub async fn broadcast(&self, content: &str) -> Result<()> {
        let clients = self.clients.read().await;
        let message = serde_json::json!({
            "type": "message",
            "content": content,
            "timestamp": chrono::Utc::now().to_rfc3339()
        })
        .to_string();

        for (session_id, sender) in clients.iter() {
            if let Err(e) = sender.send(message.clone()).await {
                tracing::warn!(session_id = %session_id, error = %e, "Failed to broadcast to client");
            }
        }

        Ok(())
    }

    /// Process an incoming WebChat message
    pub fn process_message(&self, session_id: &str, payload: &WebChatPayload) -> ChannelMessage {
        let source = ChannelSource::with_chat("webchat", session_id, session_id);
        ChannelMessage::new(source, payload.content.clone())
    }

    /// Get the number of connected clients
    pub async fn client_count(&self) -> usize {
        self.clients.read().await.len()
    }

    /// Get the WebSocket path
    pub fn websocket_path(&self) -> &str {
        self.config.websocket_path.as_deref().unwrap_or("/ws/chat")
    }

    /// Check if authentication is required
    pub fn requires_auth(&self) -> bool {
        self.config.auth_token.is_some()
    }

    /// Validate an authentication token
    pub fn validate_token(&self, token: &str) -> bool {
        match &self.config.auth_token {
            Some(expected) => token == expected,
            None => true, // No auth required
        }
    }
}

#[async_trait]
impl ChannelAdapter for WebChatAdapter {
    fn channel_type(&self) -> &'static str {
        "webchat"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::WebSocket
    }

    async fn start(&self, _tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        self.connected.store(true, Ordering::SeqCst);
        tracing::info!(
            path = %self.websocket_path(),
            "WebChat adapter started"
        );
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.shutdown.notify_one();

        // Clear all clients
        self.clients.write().await.clear();

        tracing::info!("WebChat adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "webchat" {
            return Err(Error::channel("Invalid channel source for WebChat"));
        }

        let session_id = to.chat_id.as_deref().unwrap_or(&to.user_id);

        self.send_to_client(session_id, content).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn send_typing(&self, to: &ChannelSource) -> Result<()> {
        let session_id = to.chat_id.as_deref().unwrap_or(&to.user_id);
        let typing_msg = serde_json::json!({
            "type": "typing",
            "session_id": session_id,
        });
        // Send typing indicator to all connected WebSocket clients for this session
        let clients = self.clients.read().await;
        if let Some(client_tx) = clients.get(session_id) {
            let _ = client_tx.send(typing_msg.to_string()).await;
        }
        Ok(())
    }

    fn supports_typing(&self) -> bool {
        true
    }

    async fn handle_webhook(
        &self,
        payload: &[u8],
        tx: &mpsc::Sender<ChannelMessage>,
    ) -> Result<()> {
        // WebChat uses WebSocket, but this can handle HTTP fallback
        let chat_payload: WebChatPayload = serde_json::from_slice(payload)
            .map_err(|e| Error::Channel(format!("Failed to parse WebChat payload: {}", e)))?;

        let session_id = chat_payload.session_id.as_deref().unwrap_or("anonymous");
        let message = self.process_message(session_id, &chat_payload);

        tx.send(message)
            .await
            .map_err(|e| Error::Channel(format!("Failed to forward WebChat message: {}", e)))?;

        Ok(())
    }
}

/// WebChat message payload
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebChatPayload {
    /// Message type
    #[serde(rename = "type", default = "default_message_type")]
    pub message_type: String,
    /// Message content
    pub content: String,
    /// Session ID
    pub session_id: Option<String>,
    /// Optional metadata
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

fn default_message_type() -> String {
    "message".to_string()
}

/// WebChat configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebChatConfig {
    /// WebSocket path (default: /ws/chat)
    #[serde(default)]
    pub websocket_path: Option<String>,
    /// Optional authentication token
    #[serde(default)]
    pub auth_token: Option<String>,
    /// CORS allowed origins
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// Maximum message size in bytes
    #[serde(default = "default_max_message_size")]
    pub max_message_size: usize,
    /// Connection timeout in seconds
    #[serde(default = "default_connection_timeout")]
    pub connection_timeout_secs: u64,
}

impl Default for WebChatConfig {
    fn default() -> Self {
        Self {
            websocket_path: None,
            auth_token: None,
            allowed_origins: Vec::new(),
            max_message_size: 65536,
            connection_timeout_secs: 300,
        }
    }
}

fn default_max_message_size() -> usize {
    65536 // 64KB
}

fn default_connection_timeout() -> u64 {
    300 // 5 minutes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webchat_config_default() {
        let config = WebChatConfig::default();
        assert!(config.websocket_path.is_none());
        assert!(config.auth_token.is_none());
        assert_eq!(config.max_message_size, 65536);
    }

    #[test]
    fn test_webchat_adapter_creation() {
        let config = WebChatConfig::default();
        let adapter = WebChatAdapter::new(config);
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "webchat");
        assert_eq!(adapter.websocket_path(), "/ws/chat");
    }

    #[tokio::test]
    async fn test_webchat_adapter_lifecycle() {
        let config = WebChatConfig::default();
        let adapter = WebChatAdapter::new(config);
        let (tx, _rx) = mpsc::channel(100);

        adapter
            .start(tx)
            .await
            .expect("async operation should succeed");
        assert!(adapter.is_connected());

        adapter
            .stop()
            .await
            .expect("async operation should succeed");
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_webchat_client_registration() {
        let config = WebChatConfig::default();
        let adapter = WebChatAdapter::new(config);

        let (client_tx, _client_rx) = mpsc::channel(10);
        adapter
            .register_client("session1".to_string(), client_tx)
            .await;

        assert_eq!(adapter.client_count().await, 1);

        adapter.unregister_client("session1").await;
        assert_eq!(adapter.client_count().await, 0);
    }

    #[test]
    fn test_webchat_auth_validation() {
        let config = WebChatConfig {
            auth_token: Some("secret123".to_string()),
            ..Default::default()
        };
        let adapter = WebChatAdapter::new(config);

        assert!(adapter.requires_auth());
        assert!(adapter.validate_token("secret123"));
        assert!(!adapter.validate_token("wrong"));
    }

    #[test]
    fn test_webchat_payload_parsing() {
        let json = r#"{"type": "message", "content": "Hello!", "session_id": "abc123"}"#;
        let payload: WebChatPayload =
            serde_json::from_str(json).expect("should parse successfully");

        assert_eq!(payload.message_type, "message");
        assert_eq!(payload.content, "Hello!");
        assert_eq!(payload.session_id, Some("abc123".to_string()));
    }

    #[test]
    fn test_process_message() {
        let config = WebChatConfig::default();
        let adapter = WebChatAdapter::new(config);

        let payload = WebChatPayload {
            message_type: "message".to_string(),
            content: "Test message".to_string(),
            session_id: Some("user123".to_string()),
            metadata: HashMap::new(),
        };

        let message = adapter.process_message("user123", &payload);
        assert_eq!(message.content, "Test message");
        assert_eq!(message.source.channel_type(), "webchat");
    }
}
