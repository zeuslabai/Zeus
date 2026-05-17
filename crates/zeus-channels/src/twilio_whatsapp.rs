//! Twilio WhatsApp channel adapter
//!
//! Sends and receives WhatsApp messages via the Twilio WhatsApp API.
//! Uses the same Twilio Messages REST API as SMS, but with the
//! `whatsapp:+number` prefix format for From/To fields.
//!
//! Twilio WhatsApp supports:
//! - Sandbox mode (for development/testing)
//! - Production mode (with approved WhatsApp Business number)
//! - Text messages, media, and template messages
//!
//! Inbound messages arrive via Twilio webhook (form-urlencoded, same as SMS).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use zeus_core::{Error, Result};

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};

// ============================================================================
// Constants
// ============================================================================

/// Twilio REST API base URL.
const TWILIO_API_BASE: &str = "https://api.twilio.com/2010-04-01";

/// Maximum WhatsApp message length (Twilio limit).
const MAX_MESSAGE_LENGTH: usize = 4096;

// ============================================================================
// Configuration
// ============================================================================

/// Twilio WhatsApp channel configuration.
///
/// ```toml
/// [channels.twilio_whatsapp]
/// account_sid = "ACxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
/// auth_token = "your_auth_token"
/// whatsapp_number = "+14155238886"     # Twilio sandbox or production number
/// webhook_path = "/v1/webhooks/whatsapp"
/// sandbox = true                        # true for Twilio sandbox
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwilioWhatsAppConfig {
    /// Twilio Account SID
    #[serde(default)]
    pub account_sid: String,

    /// Twilio Auth Token
    #[serde(default)]
    pub auth_token: String,

    /// WhatsApp-enabled Twilio number (E.164 format, e.g. "+14155238886")
    ///
    /// For sandbox: use the Twilio sandbox number
    /// For production: use your approved WhatsApp Business number
    #[serde(default)]
    pub whatsapp_number: String,

    /// Webhook path for receiving inbound WhatsApp messages
    #[serde(default = "default_webhook_path")]
    pub webhook_path: String,

    /// Whether using Twilio WhatsApp sandbox (default: true)
    #[serde(default = "default_sandbox")]
    pub sandbox: bool,

    /// Status callback URL for message delivery reports (optional)
    #[serde(default)]
    pub status_callback_url: Option<String>,
}

fn default_webhook_path() -> String {
    "/v1/webhooks/whatsapp".to_string()
}

fn default_sandbox() -> bool {
    true
}

impl Default for TwilioWhatsAppConfig {
    fn default() -> Self {
        Self {
            account_sid: String::new(),
            auth_token: String::new(),
            whatsapp_number: String::new(),
            webhook_path: default_webhook_path(),
            sandbox: default_sandbox(),
            status_callback_url: None,
        }
    }
}

impl TwilioWhatsAppConfig {
    /// Create config from environment variables.
    ///
    /// Reads:
    /// - `TWILIO_ACCOUNT_SID`
    /// - `TWILIO_AUTH_TOKEN`
    /// - `TWILIO_WHATSAPP_NUMBER`
    /// - `TWILIO_WHATSAPP_SANDBOX` (optional, "true"/"false")
    pub fn from_env() -> Self {
        let sandbox = std::env::var("TWILIO_WHATSAPP_SANDBOX")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(true);

        Self {
            account_sid: std::env::var("TWILIO_ACCOUNT_SID").unwrap_or_default(),
            auth_token: std::env::var("TWILIO_AUTH_TOKEN").unwrap_or_default(),
            whatsapp_number: std::env::var("TWILIO_WHATSAPP_NUMBER").unwrap_or_default(),
            webhook_path: default_webhook_path(),
            sandbox,
            status_callback_url: None,
        }
    }

    /// Validate that required fields are present.
    pub fn validate(&self) -> Result<()> {
        if self.account_sid.is_empty() {
            return Err(Error::Channel(
                "Twilio WhatsApp: account_sid is required".into(),
            ));
        }
        if self.auth_token.is_empty() {
            return Err(Error::Channel(
                "Twilio WhatsApp: auth_token is required".into(),
            ));
        }
        if self.whatsapp_number.is_empty() {
            return Err(Error::Channel(
                "Twilio WhatsApp: whatsapp_number is required".into(),
            ));
        }
        if !self.whatsapp_number.starts_with('+') {
            return Err(Error::Channel(
                "Twilio WhatsApp: whatsapp_number must be in E.164 format (+1234567890)".into(),
            ));
        }
        Ok(())
    }

    /// Format the WhatsApp number for Twilio API (whatsapp:+number).
    pub fn whatsapp_from(&self) -> String {
        format!("whatsapp:{}", self.whatsapp_number)
    }
}

// ============================================================================
// Twilio webhook payload
// ============================================================================

/// Inbound WhatsApp message from Twilio webhook.
///
/// Twilio sends the same form-urlencoded format as SMS webhooks,
/// but From/To fields include the `whatsapp:` prefix.
#[derive(Debug, Deserialize)]
pub struct TwilioWhatsAppWebhook {
    /// Twilio message SID
    #[serde(rename = "MessageSid")]
    pub message_sid: String,

    /// Sender (e.g., "whatsapp:+14155551234")
    #[serde(rename = "From")]
    pub from: String,

    /// Recipient — our WhatsApp number (e.g., "whatsapp:+14155238886")
    #[serde(rename = "To")]
    pub to: String,

    /// Message body
    #[serde(rename = "Body")]
    pub body: String,

    /// Number of media attachments
    #[serde(rename = "NumMedia", default)]
    pub num_media: String,

    /// Sender's profile name (WhatsApp-specific)
    #[serde(rename = "ProfileName", default)]
    pub profile_name: Option<String>,

    /// Account SID
    #[serde(rename = "AccountSid", default)]
    pub account_sid: Option<String>,

    /// Message status (for status callbacks)
    #[serde(rename = "MessageStatus", default)]
    pub message_status: Option<String>,

    /// First media URL (if present)
    #[serde(rename = "MediaUrl0", default)]
    pub media_url_0: Option<String>,

    /// First media content type (if present)
    #[serde(rename = "MediaContentType0", default)]
    pub media_content_type_0: Option<String>,
}

impl TwilioWhatsAppWebhook {
    /// Extract the phone number from the `whatsapp:+number` format.
    pub fn sender_phone(&self) -> &str {
        self.from.strip_prefix("whatsapp:").unwrap_or(&self.from)
    }

    /// Extract the recipient phone number.
    pub fn recipient_phone(&self) -> &str {
        self.to.strip_prefix("whatsapp:").unwrap_or(&self.to)
    }
}

// ============================================================================
// Twilio API response
// ============================================================================

/// Twilio Messages API response (subset of fields).
#[derive(Debug, Deserialize)]
struct TwilioMessageResponse {
    /// Message SID
    sid: Option<String>,
    /// Message status
    status: Option<String>,
    /// Error code (if failed)
    error_code: Option<i64>,
    /// Error message (if failed)
    error_message: Option<String>,
}

// ============================================================================
// Adapter
// ============================================================================

/// Twilio WhatsApp channel adapter.
///
/// Sends messages via Twilio Messages REST API with `whatsapp:` prefix.
/// Receives messages via Twilio webhook callbacks (form-urlencoded).
pub struct TwilioWhatsAppAdapter {
    config: TwilioWhatsAppConfig,
    client: reqwest::Client,
    connected: Arc<AtomicBool>,
}

impl TwilioWhatsAppAdapter {
    /// Create a new Twilio WhatsApp adapter.
    pub fn new(config: TwilioWhatsAppConfig) -> Self {
        info!(
            from = %config.whatsapp_number,
            sandbox = config.sandbox,
            "Twilio WhatsApp adapter created"
        );
        Self {
            config,
            client: reqwest::Client::new(),
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Send a WhatsApp message via Twilio REST API.
    ///
    /// The `to` number must be in E.164 format ("+14155551234").
    /// The adapter automatically adds the `whatsapp:` prefix.
    async fn send_whatsapp(&self, to: &str, body: &str) -> Result<String> {
        if body.is_empty() {
            return Err(Error::Channel("Cannot send empty WhatsApp message".into()));
        }

        if body.len() > MAX_MESSAGE_LENGTH {
            warn!(
                len = body.len(),
                max = MAX_MESSAGE_LENGTH,
                "WhatsApp message exceeds max length, will be truncated by Twilio"
            );
        }

        let url = format!(
            "{}/Accounts/{}/Messages.json",
            TWILIO_API_BASE, self.config.account_sid
        );

        // Twilio WhatsApp requires the whatsapp: prefix on From and To
        let from = self.config.whatsapp_from();
        let to_whatsapp = if to.starts_with("whatsapp:") {
            to.to_string()
        } else {
            format!("whatsapp:{}", to)
        };

        let mut params = vec![
            ("From", from.as_str()),
            ("To", to_whatsapp.as_str()),
            ("Body", body),
        ];

        // Add status callback if configured
        let callback;
        if let Some(ref url) = self.config.status_callback_url {
            callback = url.clone();
            params.push(("StatusCallback", callback.as_str()));
        }

        debug!(
            from = %from,
            to = %to_whatsapp,
            body_len = body.len(),
            "Sending WhatsApp message via Twilio"
        );

        let resp = self
            .client
            .post(&url)
            .basic_auth(&self.config.account_sid, Some(&self.config.auth_token))
            .form(&params)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Twilio WhatsApp send failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
            return Err(Error::Channel(format!(
                "Twilio WhatsApp API returned {status}: {text}"
            )));
        }

        let response: TwilioMessageResponse = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Twilio response parse failed: {e}")))?;

        if let Some(ref code) = response.error_code {
            let msg = response.error_message.as_deref().unwrap_or("unknown error");
            return Err(Error::Channel(format!(
                "Twilio WhatsApp error {code}: {msg}"
            )));
        }

        let sid = response.sid.unwrap_or_else(|| "unknown".to_string());

        debug!(
            sid = %sid,
            status = ?response.status,
            to = %to_whatsapp,
            "WhatsApp message sent"
        );

        Ok(sid)
    }

    /// Send a media message (image, document, etc.) via Twilio.
    pub async fn send_media(&self, to: &str, body: &str, media_url: &str) -> Result<String> {
        let url = format!(
            "{}/Accounts/{}/Messages.json",
            TWILIO_API_BASE, self.config.account_sid
        );

        let from = self.config.whatsapp_from();
        let to_whatsapp = if to.starts_with("whatsapp:") {
            to.to_string()
        } else {
            format!("whatsapp:{}", to)
        };

        let params = [
            ("From", from.as_str()),
            ("To", to_whatsapp.as_str()),
            ("Body", body),
            ("MediaUrl", media_url),
        ];

        let resp = self
            .client
            .post(&url)
            .basic_auth(&self.config.account_sid, Some(&self.config.auth_token))
            .form(&params)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Twilio WhatsApp media send failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
            return Err(Error::Channel(format!(
                "Twilio WhatsApp API returned {status}: {text}"
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Twilio response parse failed: {e}")))?;

        let sid = json["sid"].as_str().unwrap_or("unknown").to_string();

        debug!(sid = %sid, "WhatsApp media message sent");
        Ok(sid)
    }

    /// Parse inbound Twilio webhook into a ChannelMessage.
    fn parse_webhook(payload: &[u8]) -> Result<(TwilioWhatsAppWebhook, ChannelMessage)> {
        let webhook: TwilioWhatsAppWebhook = serde_urlencoded::from_bytes(payload)
            .map_err(|e| Error::Channel(format!("Failed to parse WhatsApp webhook: {e}")))?;

        // Check if this is a status callback (no body, has MessageStatus)
        if webhook.message_status.is_some() && webhook.body.is_empty() {
            return Err(Error::Channel("Status callback, not a user message".into()));
        }

        let sender_phone = webhook.sender_phone().to_string();
        let recipient_phone = webhook.recipient_phone().to_string();

        let source = ChannelSource::with_chat("twilio_whatsapp", &sender_phone, &recipient_phone);

        // Include profile name in content prefix if available
        let content = if let Some(ref name) = webhook.profile_name {
            if webhook.body.is_empty() {
                format!("[{}]", name)
            } else {
                webhook.body.clone()
            }
        } else {
            webhook.body.clone()
        };

        let mut msg = ChannelMessage::new(source, content);
        msg.id = webhook.message_sid.clone();

        // Handle media attachments by appending URL to content
        if let Some(ref media_url) = webhook.media_url_0 {
            let media_type = webhook.media_content_type_0.as_deref().unwrap_or("file");
            msg.content = format!(
                "{}\n[Attachment: {} ({})]",
                msg.content, media_url, media_type
            );
        }

        Ok((webhook, msg))
    }

    /// Check the Twilio account status (optional health check).
    pub async fn check_account(&self) -> Result<bool> {
        let url = format!(
            "{}/Accounts/{}.json",
            TWILIO_API_BASE, self.config.account_sid
        );

        let resp = self
            .client
            .get(&url)
            .basic_auth(&self.config.account_sid, Some(&self.config.auth_token))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Twilio account check failed: {e}")))?;

        Ok(resp.status().is_success())
    }
}

#[async_trait]
impl ChannelAdapter for TwilioWhatsAppAdapter {
    fn channel_type(&self) -> &'static str {
        "twilio_whatsapp"
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
            from = %self.config.whatsapp_number,
            sandbox = self.config.sandbox,
            webhook = %self.config.webhook_path,
            "Twilio WhatsApp adapter started"
        );
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        info!("Twilio WhatsApp adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "twilio_whatsapp" && to.channel_type() != "whatsapp" {
            return Err(Error::channel("Invalid channel source for Twilio WhatsApp"));
        }
        self.send_whatsapp(&to.user_id, content).await?;
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
            from = %webhook.sender_phone(),
            profile = ?webhook.profile_name,
            sid = %webhook.message_sid,
            media = %webhook.num_media,
            "Inbound WhatsApp message received via Twilio"
        );

        tx.send(message)
            .await
            .map_err(|e| Error::Channel(format!("Failed to forward WhatsApp message: {e}")))?;

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
    fn test_config_default() {
        let config = TwilioWhatsAppConfig::default();
        assert!(config.account_sid.is_empty());
        assert!(config.auth_token.is_empty());
        assert!(config.whatsapp_number.is_empty());
        assert_eq!(config.webhook_path, "/v1/webhooks/whatsapp");
        assert!(config.sandbox);
        assert!(config.status_callback_url.is_none());
    }

    #[test]
    fn test_config_validate_missing_sid() {
        let config = TwilioWhatsAppConfig::default();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validate_missing_token() {
        let config = TwilioWhatsAppConfig {
            account_sid: "AC123".into(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validate_missing_number() {
        let config = TwilioWhatsAppConfig {
            account_sid: "AC123".into(),
            auth_token: "token".into(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validate_bad_number_format() {
        let config = TwilioWhatsAppConfig {
            account_sid: "AC123".into(),
            auth_token: "token".into(),
            whatsapp_number: "14155238886".into(), // Missing +
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validate_ok() {
        let config = TwilioWhatsAppConfig {
            account_sid: "AC123".into(),
            auth_token: "token".into(),
            whatsapp_number: "+14155238886".into(),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_whatsapp_from_format() {
        let config = TwilioWhatsAppConfig {
            whatsapp_number: "+14155238886".into(),
            ..Default::default()
        };
        assert_eq!(config.whatsapp_from(), "whatsapp:+14155238886");
    }

    #[test]
    fn test_config_serialization() {
        let config = TwilioWhatsAppConfig {
            account_sid: "AC123".into(),
            auth_token: "tok".into(),
            whatsapp_number: "+14155238886".into(),
            webhook_path: "/v1/webhooks/whatsapp".into(),
            sandbox: false,
            status_callback_url: Some("https://example.com/status".into()),
        };
        let json = serde_json::to_string(&config).expect("should serialize");
        let parsed: TwilioWhatsAppConfig = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(parsed.account_sid, "AC123");
        assert_eq!(parsed.whatsapp_number, "+14155238886");
        assert!(!parsed.sandbox);
        assert_eq!(
            parsed.status_callback_url.as_deref(),
            Some("https://example.com/status")
        );
    }

    #[test]
    fn test_adapter_creation() {
        let config = TwilioWhatsAppConfig {
            account_sid: "AC123".into(),
            auth_token: "token".into(),
            whatsapp_number: "+14155238886".into(),
            ..Default::default()
        };
        let adapter = TwilioWhatsAppAdapter::new(config);
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "twilio_whatsapp");
    }

    #[tokio::test]
    async fn test_adapter_lifecycle() {
        let config = TwilioWhatsAppConfig {
            account_sid: "AC123".into(),
            auth_token: "token".into(),
            whatsapp_number: "+14155238886".into(),
            ..Default::default()
        };
        let adapter = TwilioWhatsAppAdapter::new(config);
        let (tx, _rx) = mpsc::channel(100);

        adapter.start(tx).await.expect("start should succeed");
        assert!(adapter.is_connected());

        adapter.stop().await.expect("stop should succeed");
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_adapter_start_fails_without_config() {
        let config = TwilioWhatsAppConfig::default();
        let adapter = TwilioWhatsAppAdapter::new(config);
        let (tx, _rx) = mpsc::channel(100);
        assert!(adapter.start(tx).await.is_err());
    }

    #[test]
    fn test_receive_mode() {
        let config = TwilioWhatsAppConfig::default();
        let adapter = TwilioWhatsAppAdapter::new(config);
        match adapter.receive_mode() {
            ReceiveMode::Webhook { path } => {
                assert_eq!(path, "/v1/webhooks/whatsapp");
            }
            _ => panic!("Expected Webhook receive mode"),
        }
    }

    #[test]
    fn test_parse_webhook_valid() {
        // Twilio WhatsApp webhook payload (form-urlencoded)
        let payload = b"MessageSid=SM123abc&From=whatsapp%3A%2B14155551234&To=whatsapp%3A%2B14155238886&Body=Hello+from+WhatsApp&NumMedia=0&ProfileName=John+Doe";
        let (webhook, message) =
            TwilioWhatsAppAdapter::parse_webhook(payload).expect("should parse");

        assert_eq!(webhook.message_sid, "SM123abc");
        assert_eq!(webhook.from, "whatsapp:+14155551234");
        assert_eq!(webhook.to, "whatsapp:+14155238886");
        assert_eq!(webhook.body, "Hello from WhatsApp");
        assert_eq!(webhook.sender_phone(), "+14155551234");
        assert_eq!(webhook.recipient_phone(), "+14155238886");
        assert_eq!(webhook.profile_name.as_deref(), Some("John Doe"));

        assert_eq!(message.content, "Hello from WhatsApp");
        assert_eq!(message.source.channel_type(), "twilio_whatsapp");
        assert_eq!(message.source.user_id, "+14155551234");
        assert_eq!(message.id, "SM123abc");
    }

    #[test]
    fn test_parse_webhook_with_media() {
        let payload = b"MessageSid=SM456&From=whatsapp%3A%2B14155551234&To=whatsapp%3A%2B14155238886&Body=Check+this+out&NumMedia=1&MediaUrl0=https%3A%2F%2Fapi.twilio.com%2Fmedia.jpg&MediaContentType0=image%2Fjpeg";
        let (_webhook, message) =
            TwilioWhatsAppAdapter::parse_webhook(payload).expect("should parse");

        assert!(message.content.contains("Check this out"));
        assert!(message.content.contains("media.jpg"));
        assert!(message.content.contains("image/jpeg"));
    }

    #[test]
    fn test_parse_webhook_status_callback() {
        // Status callbacks have MessageStatus but empty Body
        let payload = b"MessageSid=SM789&From=whatsapp%3A%2B14155551234&To=whatsapp%3A%2B14155238886&Body=&MessageStatus=delivered&NumMedia=0";
        let result = TwilioWhatsAppAdapter::parse_webhook(payload);
        assert!(result.is_err(), "Status callbacks should be rejected");
    }

    #[test]
    fn test_parse_webhook_invalid() {
        let payload = b"invalid garbage data";
        assert!(TwilioWhatsAppAdapter::parse_webhook(payload).is_err());
    }

    #[test]
    fn test_webhook_sender_phone_strip() {
        let webhook = TwilioWhatsAppWebhook {
            message_sid: "SM1".into(),
            from: "whatsapp:+14155551234".into(),
            to: "whatsapp:+14155238886".into(),
            body: "test".into(),
            num_media: "0".into(),
            profile_name: None,
            account_sid: None,
            message_status: None,
            media_url_0: None,
            media_content_type_0: None,
        };
        assert_eq!(webhook.sender_phone(), "+14155551234");
        assert_eq!(webhook.recipient_phone(), "+14155238886");
    }

    #[test]
    fn test_webhook_sender_phone_no_prefix() {
        let webhook = TwilioWhatsAppWebhook {
            message_sid: "SM1".into(),
            from: "+14155551234".into(), // No whatsapp: prefix
            to: "+14155238886".into(),
            body: "test".into(),
            num_media: "0".into(),
            profile_name: None,
            account_sid: None,
            message_status: None,
            media_url_0: None,
            media_content_type_0: None,
        };
        assert_eq!(webhook.sender_phone(), "+14155551234");
    }

    #[tokio::test]
    async fn test_webhook_forwarding() {
        let config = TwilioWhatsAppConfig {
            account_sid: "AC123".into(),
            auth_token: "token".into(),
            whatsapp_number: "+14155238886".into(),
            ..Default::default()
        };
        let adapter = TwilioWhatsAppAdapter::new(config);
        let (tx, mut rx) = mpsc::channel(100);

        let payload = b"MessageSid=SM999&From=whatsapp%3A%2B14155550001&To=whatsapp%3A%2B14155238886&Body=Test+message&NumMedia=0";
        adapter
            .handle_webhook(payload, &tx)
            .await
            .expect("webhook should succeed");

        let msg = rx.recv().await.expect("should receive message");
        assert_eq!(msg.content, "Test message");
        assert_eq!(msg.source.user_id, "+14155550001");
        assert_eq!(msg.source.channel_type(), "twilio_whatsapp");
    }

    #[tokio::test]
    async fn test_send_empty_message_fails() {
        let config = TwilioWhatsAppConfig {
            account_sid: "AC123".into(),
            auth_token: "token".into(),
            whatsapp_number: "+14155238886".into(),
            ..Default::default()
        };
        let adapter = TwilioWhatsAppAdapter::new(config);
        let result = adapter.send_whatsapp("+14155551234", "").await;
        assert!(result.is_err());
    }
}
