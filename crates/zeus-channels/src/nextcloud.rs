//! Nextcloud Talk channel adapter
//!
//! Provides Nextcloud Talk messaging support via Talk API.
//! Supports long-polling for receiving and REST API for sending.

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, RwLock, mpsc};
use zeus_core::{Error, Result};

/// Nextcloud Talk channel adapter
pub struct NextcloudAdapter {
    connected: Arc<AtomicBool>,
    config: NextcloudConfig,
    shutdown: Arc<Notify>,
    /// Shared HTTP client (connection pooling)
    client: reqwest::Client,
    /// Handle to the receive task
    task_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
    /// Last known message ID for each conversation
    last_known_ids: Arc<RwLock<std::collections::HashMap<String, i64>>>,
}

impl NextcloudAdapter {
    /// Create a new Nextcloud Talk adapter
    pub async fn new(config: NextcloudConfig) -> Result<Self> {
        if config.server_url.is_empty() {
            return Err(Error::Config("Nextcloud server_url is required".into()));
        }
        if config.username.is_empty() || config.password.is_empty() {
            return Err(Error::Config(
                "Nextcloud username and password are required".into(),
            ));
        }

        tracing::info!(server = %config.server_url, "Nextcloud Talk adapter created");

        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            config,
            shutdown: Arc::new(Notify::new()),
            client: reqwest::Client::new(),
            task_handle: RwLock::new(None),
            last_known_ids: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    /// Build authorization header (Basic auth or App Password)
    fn auth_header(&self) -> String {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let credentials = format!("{}:{}", self.config.username, self.config.password);
        format!("Basic {}", STANDARD.encode(credentials))
    }

    /// Get the list of conversations
    pub async fn get_conversations(&self) -> Result<Vec<NextcloudConversation>> {
        let client = &self.client;
        let url = format!(
            "{}/ocs/v2.php/apps/spreed/api/v4/room",
            self.config.server_url.trim_end_matches('/')
        );

        let response = client
            .get(&url)
            .header("Authorization", self.auth_header())
            .header("OCS-APIRequest", "true")
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to get conversations: {}", e)))?;

        let body: NextcloudOcsResponse<Vec<NextcloudConversation>> = response
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Failed to parse conversations: {}", e)))?;

        Ok(body.ocs.data)
    }

    /// Get messages from a conversation
    pub async fn get_messages(
        &self,
        token: &str,
        last_known_id: Option<i64>,
    ) -> Result<Vec<NextcloudMessage>> {
        let client = &self.client;
        let mut url = format!(
            "{}/ocs/v2.php/apps/spreed/api/v4/chat/{}",
            self.config.server_url.trim_end_matches('/'),
            token
        );

        if let Some(id) = last_known_id {
            url.push_str(&format!("?lastKnownMessageId={}&lookIntoFuture=1", id));
        }

        let response = client
            .get(&url)
            .header("Authorization", self.auth_header())
            .header("OCS-APIRequest", "true")
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to get messages: {}", e)))?;

        let body: NextcloudOcsResponse<Vec<NextcloudMessage>> = response
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Failed to parse messages: {}", e)))?;

        Ok(body.ocs.data)
    }

    /// Send a message to a conversation
    pub async fn send_message(&self, token: &str, text: &str) -> Result<()> {
        let client = &self.client;
        let url = format!(
            "{}/ocs/v2.php/apps/spreed/api/v4/chat/{}",
            self.config.server_url.trim_end_matches('/'),
            token
        );

        let response = client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("OCS-APIRequest", "true")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "message": text
            }))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to send message: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "Nextcloud API error {}: {}",
                status, body
            )));
        }

        tracing::info!("Nextcloud message sent to conversation");
        Ok(())
    }

    /// Start the polling loop
    async fn start_polling(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        let connected = self.connected.clone();
        let shutdown = self.shutdown.clone();
        let config = self.config.clone();
        let last_known_ids = self.last_known_ids.clone();
        let poll_interval = self.config.poll_interval_secs.unwrap_or(5);
        let client = self.client.clone();

        let handle = tokio::spawn(async move {
            let adapter = NextcloudAdapter {
                connected: connected.clone(),
                config,
                shutdown: Arc::new(Notify::new()),
                client,
                task_handle: RwLock::new(None),
                last_known_ids: last_known_ids.clone(),
            };

            loop {
                tokio::select! {
                    _ = shutdown.notified() => {
                        tracing::info!("Nextcloud polling shutting down");
                        break;
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(poll_interval)) => {
                        if let Err(e) = Self::poll_messages(&adapter, &tx, &last_known_ids).await {
                            tracing::error!(error = %e, "Error polling Nextcloud messages");
                        }
                    }
                }
            }
            connected.store(false, Ordering::SeqCst);
        });

        *self.task_handle.write().await = Some(handle);
        self.connected.store(true, Ordering::SeqCst);
        tracing::info!("Nextcloud Talk polling started");

        Ok(())
    }

    /// Poll for new messages
    async fn poll_messages(
        adapter: &NextcloudAdapter,
        tx: &mpsc::Sender<ChannelMessage>,
        last_known_ids: &Arc<RwLock<std::collections::HashMap<String, i64>>>,
    ) -> Result<()> {
        let conversations = adapter.get_conversations().await?;

        for conv in conversations {
            let last_id = last_known_ids.read().await.get(&conv.token).copied();
            let messages = adapter.get_messages(&conv.token, last_id).await?;

            for msg in messages {
                // Skip system messages and our own messages
                if msg.message_type == "system" || msg.actor_id == adapter.config.username {
                    continue;
                }

                let source = ChannelSource::with_chat("nextcloud", &msg.actor_id, &conv.token);
                let message = ChannelMessage::new(source, msg.message);

                if let Err(e) = tx.send(message).await {
                    tracing::error!(error = %e, "Failed to forward Nextcloud message");
                }

                // Update last known ID
                last_known_ids
                    .write()
                    .await
                    .insert(conv.token.clone(), msg.id);
            }
        }

        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for NextcloudAdapter {
    fn channel_type(&self) -> &'static str {
        "nextcloud"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Polling {
            interval_secs: self.config.poll_interval_secs.unwrap_or(5),
        }
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        self.start_polling(tx).await
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.shutdown.notify_one();

        if let Some(handle) = self.task_handle.write().await.take() {
            let _ = handle.await;
        }

        tracing::info!("Nextcloud Talk adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "nextcloud" {
            return Err(Error::channel("Invalid channel source for Nextcloud"));
        }

        let token = to
            .chat_id
            .as_deref()
            .ok_or_else(|| Error::channel("Nextcloud send requires a conversation token"))?;

        self.send_message(token, content).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

/// OCS API response wrapper
#[derive(Debug, Clone, Deserialize)]
pub struct NextcloudOcsResponse<T> {
    /// OCS container
    pub ocs: NextcloudOcs<T>,
}

/// OCS container
#[derive(Debug, Clone, Deserialize)]
pub struct NextcloudOcs<T> {
    /// Meta information
    pub meta: NextcloudMeta,
    /// Data
    pub data: T,
}

/// OCS meta
#[derive(Debug, Clone, Deserialize)]
pub struct NextcloudMeta {
    /// Status
    pub status: String,
    /// Status code
    pub statuscode: i32,
}

/// Nextcloud conversation
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NextcloudConversation {
    /// Conversation token
    pub token: String,
    /// Display name
    pub display_name: String,
    /// Conversation type (1=one-to-one, 2=group, 3=public)
    #[serde(rename = "type")]
    pub conversation_type: i32,
}

/// Nextcloud message
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NextcloudMessage {
    /// Message ID
    pub id: i64,
    /// Message content
    pub message: String,
    /// Actor ID (sender)
    pub actor_id: String,
    /// Actor display name
    pub actor_display_name: String,
    /// Message type (comment, system, etc.)
    pub message_type: String,
    /// Timestamp
    pub timestamp: i64,
}

/// Nextcloud configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NextcloudConfig {
    /// Server URL (e.g., https://cloud.example.com)
    #[serde(default)]
    pub server_url: String,
    /// Username
    #[serde(default)]
    pub username: String,
    /// Password or app password
    #[serde(default)]
    pub password: String,
    /// Poll interval in seconds
    #[serde(default)]
    pub poll_interval_secs: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nextcloud_config_default() {
        let config = NextcloudConfig::default();
        assert!(config.server_url.is_empty());
        assert!(config.username.is_empty());
    }

    #[tokio::test]
    async fn test_nextcloud_adapter_validation() {
        // Empty config should fail
        let config = NextcloudConfig::default();
        assert!(NextcloudAdapter::new(config).await.is_err());

        // Missing password should fail
        let config = NextcloudConfig {
            server_url: "https://cloud.example.com".to_string(),
            username: "user".to_string(),
            ..Default::default()
        };
        assert!(NextcloudAdapter::new(config).await.is_err());

        // Valid config should succeed
        let config = NextcloudConfig {
            server_url: "https://cloud.example.com".to_string(),
            username: "user".to_string(),
            password: "password".to_string(),
            ..Default::default()
        };
        assert!(NextcloudAdapter::new(config).await.is_ok());
    }

    #[tokio::test]
    async fn test_nextcloud_adapter_lifecycle() {
        let config = NextcloudConfig {
            server_url: "https://cloud.example.com".to_string(),
            username: "user".to_string(),
            password: "password".to_string(),
            ..Default::default()
        };

        let adapter = NextcloudAdapter::new(config)
            .await
            .expect("NextcloudAdapter::new should succeed");
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "nextcloud");
    }

    #[test]
    fn test_ocs_response_parsing() {
        let json = r#"{
            "ocs": {
                "meta": {"status": "ok", "statuscode": 200},
                "data": [{"token": "abc123", "displayName": "Test", "type": 1}]
            }
        }"#;

        let response: NextcloudOcsResponse<Vec<NextcloudConversation>> =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(response.ocs.data.len(), 1);
        assert_eq!(response.ocs.data[0].token, "abc123");
    }

    #[test]
    fn test_nextcloud_message_parsing() {
        let json = r#"{
            "id": 123,
            "message": "Hello Nextcloud!",
            "actorId": "user1",
            "actorDisplayName": "User One",
            "messageType": "comment",
            "timestamp": 1640000000
        }"#;

        let msg: NextcloudMessage = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(msg.id, 123);
        assert_eq!(msg.message, "Hello Nextcloud!");
        assert_eq!(msg.actor_id, "user1");
        assert_eq!(msg.message_type, "comment");
    }

    #[test]
    fn test_nextcloud_conversation_types() {
        let json = r#"[
            {"token": "one2one", "displayName": "Private", "type": 1},
            {"token": "group", "displayName": "Team Chat", "type": 2},
            {"token": "public", "displayName": "Public Room", "type": 3}
        ]"#;

        let convs: Vec<NextcloudConversation> =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(convs.len(), 3);
        assert_eq!(convs[0].conversation_type, 1); // one-to-one
        assert_eq!(convs[1].conversation_type, 2); // group
        assert_eq!(convs[2].conversation_type, 3); // public
    }

    #[tokio::test]
    async fn test_auth_header_generation() {
        let config = NextcloudConfig {
            server_url: "https://cloud.example.com".to_string(),
            username: "testuser".to_string(),
            password: "testpass".to_string(),
            ..Default::default()
        };

        let adapter = NextcloudAdapter::new(config)
            .await
            .expect("NextcloudAdapter::new should succeed");
        let auth = adapter.auth_header();

        // Should start with "Basic "
        assert!(auth.starts_with("Basic "));

        // Should be base64-encoded credentials
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let decoded = STANDARD.decode(&auth[6..]).expect("decode should succeed");
        let decoded_str = String::from_utf8(decoded).expect("operation should succeed");
        assert_eq!(decoded_str, "testuser:testpass");
    }

    #[test]
    fn test_receive_mode_polling() {
        let config = NextcloudConfig {
            server_url: "https://cloud.example.com".to_string(),
            username: "user".to_string(),
            password: "pass".to_string(),
            poll_interval_secs: Some(10),
        };

        // Create adapter via futures executor
        let adapter = futures::executor::block_on(NextcloudAdapter::new(config))
            .expect("NextcloudAdapter::new should succeed");

        match adapter.receive_mode() {
            ReceiveMode::Polling { interval_secs } => {
                assert_eq!(interval_secs, 10);
            }
            _ => panic!("Expected Polling receive mode"),
        }
    }

    #[test]
    fn test_receive_mode_polling_default() {
        let config = NextcloudConfig {
            server_url: "https://cloud.example.com".to_string(),
            username: "user".to_string(),
            password: "pass".to_string(),
            poll_interval_secs: None,
        };

        let adapter = futures::executor::block_on(NextcloudAdapter::new(config))
            .expect("NextcloudAdapter::new should succeed");

        match adapter.receive_mode() {
            ReceiveMode::Polling { interval_secs } => {
                assert_eq!(interval_secs, 5); // default
            }
            _ => panic!("Expected Polling receive mode"),
        }
    }

    #[test]
    fn test_ocs_meta_parsing() {
        let json = r#"{"status": "ok", "statuscode": 200}"#;
        let meta: NextcloudMeta = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(meta.status, "ok");
        assert_eq!(meta.statuscode, 200);
    }
}
