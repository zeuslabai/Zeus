//! SMS channel adapter
//!
//! Provides SMS messaging via Twilio REST API. Reuses the same Twilio
//! credentials used by the voice channel (account_sid, auth_token, from_number).
//! Inbound messages arrive via Twilio webhook callbacks.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tracing::{debug, info};
use zeus_core::{Error, Result};

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};

// ============================================================================
// Configuration
// ============================================================================

/// SMS channel configuration (Twilio)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsConfig {
    /// Twilio Account SID
    #[serde(default)]
    pub account_sid: String,

    /// Twilio Auth Token
    #[serde(default)]
    pub auth_token: String,

    /// Phone number to send from (E.164 format, e.g. "+14155551234")
    #[serde(default)]
    pub from_number: String,

    /// Webhook path for receiving inbound SMS (default: "/v1/webhooks/sms")
    #[serde(default = "default_webhook_path")]
    pub webhook_path: String,
}

fn default_webhook_path() -> String {
    "/v1/webhooks/sms".to_string()
}

impl Default for SmsConfig {
    fn default() -> Self {
        Self {
            account_sid: String::new(),
            auth_token: String::new(),
            from_number: String::new(),
            webhook_path: default_webhook_path(),
        }
    }
}

impl SmsConfig {
    /// Create config from environment variables
    pub fn from_env() -> Self {
        Self {
            account_sid: std::env::var("TWILIO_ACCOUNT_SID").unwrap_or_default(),
            auth_token: std::env::var("TWILIO_AUTH_TOKEN").unwrap_or_default(),
            from_number: std::env::var("TWILIO_FROM_NUMBER").unwrap_or_default(),
            webhook_path: default_webhook_path(),
        }
    }

    /// Validate that required fields are present
    pub fn validate(&self) -> Result<()> {
        if self.account_sid.is_empty() {
            return Err(Error::Channel("SMS: account_sid is required".into()));
        }
        if self.auth_token.is_empty() {
            return Err(Error::Channel("SMS: auth_token is required".into()));
        }
        if self.from_number.is_empty() {
            return Err(Error::Channel("SMS: from_number is required".into()));
        }
        Ok(())
    }
}

// ============================================================================
// Twilio webhook payload
// ============================================================================

/// Inbound SMS payload from Twilio webhook
#[derive(Debug, Deserialize)]
pub struct TwilioSmsWebhook {
    /// Twilio message SID
    #[serde(rename = "MessageSid")]
    pub message_sid: String,

    /// Sender phone number
    #[serde(rename = "From")]
    pub from: String,

    /// Recipient phone number (our number)
    #[serde(rename = "To")]
    pub to: String,

    /// Message body
    #[serde(rename = "Body")]
    pub body: String,

    /// Number of media attachments
    #[serde(rename = "NumMedia", default)]
    pub num_media: String,
}

// ============================================================================
// Adapter
// ============================================================================

/// SMS channel adapter using Twilio
pub struct SmsAdapter {
    config: SmsConfig,
    client: reqwest::Client,
    connected: Arc<AtomicBool>,
}

impl SmsAdapter {
    /// Create a new SMS adapter
    pub fn new(config: SmsConfig) -> Self {
        info!(from = %config.from_number, "SMS adapter created");
        Self {
            config,
            client: reqwest::Client::new(),
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Send an SMS via Twilio REST API
    async fn send_sms(&self, to: &str, body: &str) -> Result<String> {
        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
            self.config.account_sid
        );

        let params = [
            ("From", self.config.from_number.as_str()),
            ("To", to),
            ("Body", body),
        ];

        let resp = self
            .client
            .post(&url)
            .basic_auth(&self.config.account_sid, Some(&self.config.auth_token))
            .form(&params)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("SMS send failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
            return Err(Error::Channel(format!(
                "Twilio SMS API returned {status}: {text}"
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("SMS response parse failed: {e}")))?;

        let sid = json["sid"].as_str().unwrap_or("unknown").to_string();

        debug!(sid = %sid, to = %to, "SMS sent");
        Ok(sid)
    }

    /// Parse inbound Twilio webhook into a ChannelMessage
    fn parse_webhook(payload: &[u8]) -> Result<(TwilioSmsWebhook, ChannelMessage)> {
        let webhook: TwilioSmsWebhook = serde_urlencoded::from_bytes(payload)
            .map_err(|e| Error::Channel(format!("Failed to parse SMS webhook: {e}")))?;

        let source = ChannelSource::with_chat("sms", &webhook.from, &webhook.to);
        let mut msg = ChannelMessage::new(source, webhook.body.clone());
        msg.id = webhook.message_sid.clone();

        Ok((webhook, msg))
    }
}

#[async_trait]
impl ChannelAdapter for SmsAdapter {
    fn channel_type(&self) -> &'static str {
        "sms"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Webhook {
            path: self.config.webhook_path.clone(),
        }
    }

    async fn start(&self, _tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        self.config.validate()?;
        self.connected.store(true, Ordering::SeqCst);
        info!(
            from = %self.config.from_number,
            webhook = %self.config.webhook_path,
            "SMS adapter started"
        );
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        info!("SMS adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "sms" {
            return Err(Error::channel("Invalid channel source for SMS"));
        }
        self.send_sms(&to.user_id, content).await?;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn handle_webhook(
        &self,
        payload: &[u8],
        tx: &mpsc::Sender<ChannelMessage>,
    ) -> Result<()> {
        let (webhook, message) = Self::parse_webhook(payload)?;
        debug!(
            from = %webhook.from,
            sid = %webhook.message_sid,
            "Inbound SMS received"
        );

        tx.send(message)
            .await
            .map_err(|e| Error::Channel(format!("Failed to forward SMS: {e}")))?;

        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sms_config_default() {
        let config = SmsConfig::default();
        assert!(config.account_sid.is_empty());
        assert_eq!(config.webhook_path, "/v1/webhooks/sms");
    }

    #[test]
    fn test_sms_config_validate_missing_sid() {
        let config = SmsConfig::default();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_sms_config_validate_missing_token() {
        let config = SmsConfig {
            account_sid: "AC123".into(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_sms_config_validate_missing_number() {
        let config = SmsConfig {
            account_sid: "AC123".into(),
            auth_token: "token".into(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_sms_config_validate_ok() {
        let config = SmsConfig {
            account_sid: "AC123".into(),
            auth_token: "token".into(),
            from_number: "+14155551234".into(),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_sms_adapter_creation() {
        let config = SmsConfig {
            account_sid: "AC123".into(),
            auth_token: "token".into(),
            from_number: "+14155551234".into(),
            ..Default::default()
        };
        let adapter = SmsAdapter::new(config);
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "sms");
    }

    #[tokio::test]
    async fn test_sms_adapter_lifecycle() {
        let config = SmsConfig {
            account_sid: "AC123".into(),
            auth_token: "token".into(),
            from_number: "+14155551234".into(),
            ..Default::default()
        };
        let adapter = SmsAdapter::new(config);
        let (tx, _rx) = mpsc::channel(100);

        adapter.start(tx).await.expect("start should succeed");
        assert!(adapter.is_connected());

        adapter.stop().await.expect("stop should succeed");
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_sms_adapter_start_fails_without_config() {
        let config = SmsConfig::default();
        let adapter = SmsAdapter::new(config);
        let (tx, _rx) = mpsc::channel(100);

        assert!(adapter.start(tx).await.is_err());
    }

    #[test]
    fn test_sms_receive_mode() {
        let config = SmsConfig::default();
        let adapter = SmsAdapter::new(config);
        match adapter.receive_mode() {
            ReceiveMode::Webhook { path } => {
                assert_eq!(path, "/v1/webhooks/sms");
            }
            _ => panic!("Expected Webhook receive mode"),
        }
    }

    #[test]
    fn test_parse_webhook_valid() {
        // Twilio sends form-urlencoded data
        let payload =
            b"MessageSid=SM123&From=%2B14155551234&To=%2B14155559999&Body=Hello+Zeus&NumMedia=0";
        let (webhook, message) = SmsAdapter::parse_webhook(payload).expect("should parse");

        assert_eq!(webhook.message_sid, "SM123");
        assert_eq!(webhook.from, "+14155551234");
        assert_eq!(webhook.to, "+14155559999");
        assert_eq!(webhook.body, "Hello Zeus");
        assert_eq!(message.content, "Hello Zeus");
        assert_eq!(message.source.channel_type(), "sms");
        assert_eq!(message.source.user_id, "+14155551234");
        assert_eq!(message.id, "SM123");
    }

    #[test]
    fn test_parse_webhook_invalid() {
        let payload = b"invalid garbage data";
        assert!(SmsAdapter::parse_webhook(payload).is_err());
    }

    #[tokio::test]
    async fn test_sms_webhook_forwarding() {
        let config = SmsConfig {
            account_sid: "AC123".into(),
            auth_token: "token".into(),
            from_number: "+14155551234".into(),
            ..Default::default()
        };
        let adapter = SmsAdapter::new(config);
        let (tx, mut rx) = mpsc::channel(100);

        let payload =
            b"MessageSid=SM456&From=%2B14155550001&To=%2B14155551234&Body=Test+message&NumMedia=0";
        adapter
            .handle_webhook(payload, &tx)
            .await
            .expect("webhook should succeed");

        let msg = rx.recv().await.expect("should receive message");
        assert_eq!(msg.content, "Test message");
        assert_eq!(msg.source.user_id, "+14155550001");
    }
}
