//! Zalo channel adapter
//!
//! Provides Zalo messaging support via Zalo Official Account API.
//! Uses webhook callbacks for receiving and REST API for sending.

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, RwLock, mpsc};
use zeus_core::{Error, Result};

const ZALO_API_BASE: &str = "https://openapi.zalo.me/v2.0/oa";

/// Zalo channel adapter
pub struct ZaloAdapter {
    connected: Arc<AtomicBool>,
    config: ZaloConfig,
    shutdown: Arc<Notify>,
    /// Shared HTTP client (connection pooling)
    client: reqwest::Client,
    /// Access token
    access_token: RwLock<Option<String>>,
}

impl ZaloAdapter {
    /// Create a new Zalo adapter
    pub async fn new(config: ZaloConfig) -> Result<Self> {
        if config.app_id.is_empty() {
            return Err(Error::Config("Zalo app_id is required".into()));
        }
        if config.secret_key.is_empty() {
            return Err(Error::Config("Zalo secret_key is required".into()));
        }

        tracing::info!("Zalo adapter created");

        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            config,
            shutdown: Arc::new(Notify::new()),
            client: reqwest::Client::new(),
            access_token: RwLock::new(None),
        })
    }

    /// Get an access token
    async fn get_access_token(&self) -> Result<String> {
        // Check cache
        if let Some(token) = self.access_token.read().await.as_ref() {
            return Ok(token.clone());
        }

        // If token is pre-configured
        if let Some(token) = &self.config.access_token {
            *self.access_token.write().await = Some(token.clone());
            return Ok(token.clone());
        }

        // Refresh token flow
        if let Some(refresh_token) = &self.config.refresh_token {
            let client = &self.client;
            let response = client
                .post("https://oauth.zaloapp.com/v4/oa/access_token")
                .form(&[
                    ("refresh_token", refresh_token.as_str()),
                    ("app_id", &self.config.app_id),
                    ("grant_type", "refresh_token"),
                ])
                .header("secret_key", &self.config.secret_key)
                .send()
                .await
                .map_err(|e| Error::Channel(format!("Failed to refresh token: {}", e)))?;

            let body: serde_json::Value = response
                .json()
                .await
                .map_err(|e| Error::Channel(format!("Failed to parse token response: {}", e)))?;

            let token = body
                .get("access_token")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| Error::Channel("No access_token in response".into()))?;

            *self.access_token.write().await = Some(token.clone());
            return Ok(token);
        }

        Err(Error::Channel(
            "Zalo access_token or refresh_token required".into(),
        ))
    }

    /// Send a message
    pub async fn send_message(&self, user_id: &str, text: &str) -> Result<()> {
        let token = self.get_access_token().await?;
        let client = &self.client;

        let url = format!("{}/message/text", ZALO_API_BASE);

        let response = client
            .post(&url)
            .header("access_token", &token)
            .json(&serde_json::json!({
                "recipient": {
                    "user_id": user_id
                },
                "message": {
                    "text": text
                }
            }))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to send Zalo message: {}", e)))?;

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Failed to parse response: {}", e)))?;

        if body.get("error").and_then(|e| e.as_i64()) != Some(0) {
            let message = body
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            return Err(Error::Channel(format!("Zalo API error: {}", message)));
        }

        tracing::info!(user_id = %user_id, "Zalo message sent");
        Ok(())
    }

    /// Process an incoming webhook event
    pub fn process_webhook(&self, event: &ZaloWebhookEvent) -> Option<ChannelMessage> {
        // Only process user_send_text events
        if event.event_name != "user_send_text" {
            return None;
        }

        let sender = event.sender.as_ref()?;
        let message = event.message.as_ref()?;

        let user_id = sender.id.clone();
        let text = message.text.clone()?;

        let source = ChannelSource::new("zalo", &user_id);
        Some(ChannelMessage::new(source, text))
    }

    /// Verify webhook signature
    pub fn verify_signature(&self, timestamp: &str, mac: &str, data: &str) -> bool {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        type HmacSha256 = Hmac<Sha256>;

        let key = format!("{}{}", self.config.app_id, timestamp);

        if let Ok(mut hmac) = HmacSha256::new_from_slice(key.as_bytes()) {
            hmac.update(data.as_bytes());
            let result = hmac.finalize();
            let expected = hex::encode(result.into_bytes());
            return expected == mac;
        }

        false
    }
}

#[async_trait]
impl ChannelAdapter for ZaloAdapter {
    fn channel_type(&self) -> &'static str {
        "zalo"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Webhook {
            path: self
                .config
                .webhook_path
                .clone()
                .unwrap_or_else(|| "/api/zalo".to_string()),
        }
    }

    async fn start(&self, _tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        self.connected.store(true, Ordering::SeqCst);
        tracing::info!("Zalo adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.shutdown.notify_one();
        *self.access_token.write().await = None;
        tracing::info!("Zalo adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "zalo" {
            return Err(Error::channel("Invalid channel source for Zalo"));
        }

        self.send_message(&to.user_id, content).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn handle_webhook(
        &self,
        payload: &[u8],
        tx: &mpsc::Sender<ChannelMessage>,
    ) -> Result<()> {
        let event: ZaloWebhookEvent = serde_json::from_slice(payload)
            .map_err(|e| Error::Channel(format!("Failed to parse Zalo webhook: {}", e)))?;

        if let Some(message) = self.process_webhook(&event) {
            tx.send(message)
                .await
                .map_err(|e| Error::Channel(format!("Failed to forward Zalo message: {}", e)))?;
        }

        Ok(())
    }
}

/// Zalo webhook event
#[derive(Debug, Clone, Deserialize)]
pub struct ZaloWebhookEvent {
    /// App ID
    pub app_id: String,
    /// User ID alias for OA
    pub user_id_by_app: Option<String>,
    /// Event name
    pub event_name: String,
    /// Timestamp
    pub timestamp: String,
    /// Sender info
    pub sender: Option<ZaloSender>,
    /// Recipient info
    pub recipient: Option<ZaloRecipient>,
    /// Message
    pub message: Option<ZaloMessage>,
}

/// Zalo sender
#[derive(Debug, Clone, Deserialize)]
pub struct ZaloSender {
    /// User ID
    pub id: String,
}

/// Zalo recipient
#[derive(Debug, Clone, Deserialize)]
pub struct ZaloRecipient {
    /// OA ID
    pub id: String,
}

/// Zalo message
#[derive(Debug, Clone, Deserialize)]
pub struct ZaloMessage {
    /// Message ID
    pub msg_id: Option<String>,
    /// Text content
    pub text: Option<String>,
}

/// Zalo configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ZaloConfig {
    /// App ID
    #[serde(default)]
    pub app_id: String,
    /// Secret Key
    #[serde(default, skip_serializing)]
    pub secret_key: String,
    /// Access Token (if already obtained)
    #[serde(default, skip_serializing)]
    pub access_token: Option<String>,
    /// Refresh Token (for token refresh)
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Webhook path
    #[serde(default)]
    pub webhook_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zalo_config_default() {
        let config = ZaloConfig::default();
        assert!(config.app_id.is_empty());
        assert!(config.secret_key.is_empty());
    }

    #[tokio::test]
    async fn test_zalo_adapter_validation() {
        // Empty config should fail
        let config = ZaloConfig::default();
        assert!(ZaloAdapter::new(config).await.is_err());

        // Missing secret should fail
        let config = ZaloConfig {
            app_id: "test-app-id".to_string(),
            ..Default::default()
        };
        assert!(ZaloAdapter::new(config).await.is_err());

        // Valid config should succeed
        let config = ZaloConfig {
            app_id: "test-app-id".to_string(),
            secret_key: "test-secret".to_string(),
            ..Default::default()
        };
        assert!(ZaloAdapter::new(config).await.is_ok());
    }

    #[tokio::test]
    async fn test_zalo_adapter_lifecycle() {
        let config = ZaloConfig {
            app_id: "test-app-id".to_string(),
            secret_key: "test-secret".to_string(),
            ..Default::default()
        };

        let adapter = ZaloAdapter::new(config)
            .await
            .expect("ZaloAdapter::new should succeed");
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "zalo");
    }

    #[test]
    fn test_zalo_webhook_parsing() {
        let json = r#"{
            "app_id": "123456",
            "event_name": "user_send_text",
            "timestamp": "1234567890",
            "sender": {"id": "user123"},
            "recipient": {"id": "oa123"},
            "message": {"msg_id": "msg123", "text": "Hello Zalo!"}
        }"#;

        let event: ZaloWebhookEvent =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(event.event_name, "user_send_text");
        assert_eq!(
            event.message.as_ref().expect("as_ref should succeed").text,
            Some("Hello Zalo!".to_string())
        );
    }

    #[tokio::test]
    async fn test_process_webhook() {
        let config = ZaloConfig {
            app_id: "test-app-id".to_string(),
            secret_key: "test-secret".to_string(),
            ..Default::default()
        };
        let adapter = ZaloAdapter::new(config)
            .await
            .expect("ZaloAdapter::new should succeed");

        let event = ZaloWebhookEvent {
            app_id: "123".to_string(),
            user_id_by_app: None,
            event_name: "user_send_text".to_string(),
            timestamp: "123".to_string(),
            sender: Some(ZaloSender {
                id: "user123".to_string(),
            }),
            recipient: Some(ZaloRecipient {
                id: "oa123".to_string(),
            }),
            message: Some(ZaloMessage {
                msg_id: Some("msg1".to_string()),
                text: Some("Test message".to_string()),
            }),
        };

        let message = adapter
            .process_webhook(&event)
            .expect("process_webhook should succeed");
        assert_eq!(message.content, "Test message");
        assert_eq!(message.source.channel_type(), "zalo");
    }
}
