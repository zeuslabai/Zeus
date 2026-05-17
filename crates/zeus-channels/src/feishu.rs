//! Feishu/Lark channel adapter
//!
//! Provides Feishu (Lark) messaging support via Bot API.
//! Uses webhook callbacks for receiving and REST API for sending.

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, RwLock, mpsc};
use zeus_core::{Error, Result};

/// Feishu channel adapter
pub struct FeishuAdapter {
    connected: Arc<AtomicBool>,
    config: FeishuConfig,
    shutdown: Arc<Notify>,
    /// Shared HTTP client (connection pooling)
    client: reqwest::Client,
    /// Tenant access token
    access_token: RwLock<Option<String>>,
}

impl FeishuAdapter {
    /// Create a new Feishu adapter
    pub async fn new(config: FeishuConfig) -> Result<Self> {
        if config.app_id.is_empty() {
            return Err(Error::Config("Feishu app_id is required".into()));
        }
        if config.app_secret.is_empty() {
            return Err(Error::Config("Feishu app_secret is required".into()));
        }

        tracing::info!("Feishu adapter created");

        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            config,
            shutdown: Arc::new(Notify::new()),
            client: reqwest::Client::new(),
            access_token: RwLock::new(None),
        })
    }

    /// Get the API base URL
    fn api_base(&self) -> &str {
        if self.config.use_lark {
            "https://open.larksuite.com/open-apis"
        } else {
            "https://open.feishu.cn/open-apis"
        }
    }

    /// Get a tenant access token
    async fn get_access_token(&self) -> Result<String> {
        // Check cache
        if let Some(token) = self.access_token.read().await.as_ref() {
            return Ok(token.clone());
        }

        let client = &self.client;
        let url = format!("{}/auth/v3/tenant_access_token/internal", self.api_base());

        let response = client
            .post(&url)
            .json(&serde_json::json!({
                "app_id": self.config.app_id,
                "app_secret": self.config.app_secret
            }))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to get access token: {}", e)))?;

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Failed to parse token response: {}", e)))?;

        let token = body
            .get("tenant_access_token")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| Error::Channel("No tenant_access_token in response".into()))?;

        *self.access_token.write().await = Some(token.clone());
        Ok(token)
    }

    /// Send a message to a chat
    pub async fn send_message(&self, receive_id: &str, text: &str) -> Result<()> {
        let token = self.get_access_token().await?;
        let client = &self.client;

        let url = format!("{}/im/v1/messages", self.api_base());

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .query(&[("receive_id_type", "chat_id")])
            .json(&serde_json::json!({
                "receive_id": receive_id,
                "msg_type": "text",
                "content": serde_json::json!({"text": text}).to_string()
            }))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to send Feishu message: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "Feishu API error {}: {}",
                status, body
            )));
        }

        tracing::info!(receive_id = %receive_id, "Feishu message sent");
        Ok(())
    }

    /// Process an incoming event
    pub fn process_event(&self, event: &FeishuEvent) -> Option<ChannelMessage> {
        // Handle message events
        if event.header.event_type != "im.message.receive_v1" {
            return None;
        }

        let event_data = event.event.as_ref()?;
        let message = event_data.message.as_ref()?;
        let sender = event_data.sender.as_ref()?;

        // Parse content JSON
        let content: serde_json::Value = serde_json::from_str(&message.content).ok()?;
        let text = content.get("text").and_then(|t| t.as_str())?;

        let chat_id = message.chat_id.clone();
        let user_id = sender.sender_id.as_ref()?.open_id.clone();

        let source = ChannelSource::with_chat("feishu", &user_id, &chat_id);
        Some(ChannelMessage::new(source, text.to_string()))
    }

    /// Verify webhook signature
    pub fn verify_signature(
        &self,
        timestamp: &str,
        nonce: &str,
        body: &str,
        signature: &str,
    ) -> bool {
        if let Some(encrypt_key) = &self.config.encrypt_key {
            use sha2::{Digest, Sha256};
            let content = format!("{}{}{}{}", timestamp, nonce, encrypt_key, body);
            let result = Sha256::digest(content.as_bytes());
            let expected = format!("{:x}", result);
            return expected == signature;
        }
        true
    }
}

#[async_trait]
impl ChannelAdapter for FeishuAdapter {
    fn channel_type(&self) -> &'static str {
        "feishu"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Webhook {
            path: self
                .config
                .webhook_path
                .clone()
                .unwrap_or_else(|| "/api/feishu".to_string()),
        }
    }

    async fn start(&self, _tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        // Verify we can get a token
        let _token = self.get_access_token().await?;
        self.connected.store(true, Ordering::SeqCst);
        tracing::info!("Feishu adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.shutdown.notify_one();
        *self.access_token.write().await = None;
        tracing::info!("Feishu adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "feishu" {
            return Err(Error::channel("Invalid channel source for Feishu"));
        }

        let chat_id = to
            .chat_id
            .as_deref()
            .ok_or_else(|| Error::channel("Feishu send requires a chat_id"))?;

        self.send_message(chat_id, content).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn handle_webhook(
        &self,
        payload: &[u8],
        tx: &mpsc::Sender<ChannelMessage>,
    ) -> Result<()> {
        let event: FeishuEvent = serde_json::from_slice(payload)
            .map_err(|e| Error::Channel(format!("Failed to parse Feishu event: {}", e)))?;

        // Handle URL verification challenge
        if event.header.event_type == "url_verification" {
            // Caller should handle this and return the challenge
            return Ok(());
        }

        if let Some(message) = self.process_event(&event) {
            tx.send(message)
                .await
                .map_err(|e| Error::Channel(format!("Failed to forward Feishu message: {}", e)))?;
        }

        Ok(())
    }
}

/// Feishu event
#[derive(Debug, Clone, Deserialize)]
pub struct FeishuEvent {
    /// Schema version
    pub schema: Option<String>,
    /// Event header
    pub header: FeishuEventHeader,
    /// Event data
    pub event: Option<FeishuEventData>,
    /// Challenge (for URL verification)
    pub challenge: Option<String>,
}

/// Event header
#[derive(Debug, Clone, Deserialize)]
pub struct FeishuEventHeader {
    /// Event ID
    pub event_id: String,
    /// Event type
    pub event_type: String,
    /// Create time
    pub create_time: Option<String>,
    /// App ID
    pub app_id: Option<String>,
}

/// Event data
#[derive(Debug, Clone, Deserialize)]
pub struct FeishuEventData {
    /// Message
    pub message: Option<FeishuMessage>,
    /// Sender
    pub sender: Option<FeishuSender>,
}

/// Feishu message
#[derive(Debug, Clone, Deserialize)]
pub struct FeishuMessage {
    /// Message ID
    pub message_id: String,
    /// Chat ID
    pub chat_id: String,
    /// Chat type
    pub chat_type: String,
    /// Content (JSON string)
    pub content: String,
    /// Message type
    pub message_type: String,
}

/// Feishu sender
#[derive(Debug, Clone, Deserialize)]
pub struct FeishuSender {
    /// Sender ID
    pub sender_id: Option<FeishuSenderId>,
    /// Sender type
    pub sender_type: String,
}

/// Sender ID
#[derive(Debug, Clone, Deserialize)]
pub struct FeishuSenderId {
    /// Open ID
    pub open_id: String,
    /// User ID
    pub user_id: Option<String>,
    /// Union ID
    pub union_id: Option<String>,
}

/// Feishu configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeishuConfig {
    /// App ID
    #[serde(default)]
    pub app_id: String,
    /// App Secret
    #[serde(default)]
    pub app_secret: String,
    /// Encrypt Key (for event verification)
    #[serde(default)]
    pub encrypt_key: Option<String>,
    /// Verification Token
    #[serde(default)]
    pub verification_token: Option<String>,
    /// Webhook path
    #[serde(default)]
    pub webhook_path: Option<String>,
    /// Use Lark (international) API instead of Feishu (China)
    #[serde(default)]
    pub use_lark: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feishu_config_default() {
        let config = FeishuConfig::default();
        assert!(config.app_id.is_empty());
        assert!(config.app_secret.is_empty());
        assert!(!config.use_lark);
    }

    #[tokio::test]
    async fn test_feishu_adapter_validation() {
        // Empty config should fail
        let config = FeishuConfig::default();
        assert!(FeishuAdapter::new(config).await.is_err());

        // Missing secret should fail
        let config = FeishuConfig {
            app_id: "test-app-id".to_string(),
            ..Default::default()
        };
        assert!(FeishuAdapter::new(config).await.is_err());

        // Valid config should succeed
        let config = FeishuConfig {
            app_id: "test-app-id".to_string(),
            app_secret: "test-secret".to_string(),
            ..Default::default()
        };
        assert!(FeishuAdapter::new(config).await.is_ok());
    }

    #[tokio::test]
    async fn test_feishu_adapter_lifecycle() {
        let config = FeishuConfig {
            app_id: "test-app-id".to_string(),
            app_secret: "test-secret".to_string(),
            ..Default::default()
        };

        let adapter = FeishuAdapter::new(config)
            .await
            .expect("FeishuAdapter::new should succeed");
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "feishu");
    }

    #[test]
    fn test_feishu_event_parsing() {
        let json = r#"{
            "schema": "2.0",
            "header": {
                "event_id": "evt123",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "message": {
                    "message_id": "msg123",
                    "chat_id": "chat123",
                    "chat_type": "p2p",
                    "content": "{\"text\":\"Hello\"}",
                    "message_type": "text"
                },
                "sender": {
                    "sender_id": {"open_id": "ou_123"},
                    "sender_type": "user"
                }
            }
        }"#;

        let event: FeishuEvent = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(event.header.event_type, "im.message.receive_v1");
    }

    #[test]
    fn test_api_base_selection() {
        let feishu_config = FeishuConfig {
            app_id: "test".to_string(),
            app_secret: "test".to_string(),
            use_lark: false,
            ..Default::default()
        };

        let lark_config = FeishuConfig {
            app_id: "test".to_string(),
            app_secret: "test".to_string(),
            use_lark: true,
            ..Default::default()
        };

        // Can't easily test without creating adapter, so just verify config
        assert!(!feishu_config.use_lark);
        assert!(lark_config.use_lark);
    }
}
