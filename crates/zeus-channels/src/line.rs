//! LINE channel adapter
//!
//! Provides LINE messaging support via LINE Messaging API.
//! Uses webhook callbacks for receiving and REST API for sending.

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, mpsc};
use zeus_core::{Error, Result};

/// LINE channel adapter
pub struct LineAdapter {
    connected: Arc<AtomicBool>,
    config: LineConfig,
    shutdown: Arc<Notify>,
    /// Shared HTTP client (connection pooling)
    client: reqwest::Client,
}

impl LineAdapter {
    /// Create a new LINE adapter
    pub async fn new(config: LineConfig) -> Result<Self> {
        if config.channel_access_token.is_empty() {
            return Err(Error::Config(
                "LINE channel_access_token is required".into(),
            ));
        }

        tracing::info!("LINE adapter created");

        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            config,
            shutdown: Arc::new(Notify::new()),
            client: reqwest::Client::new(),
        })
    }

    /// Send a reply message
    pub async fn reply_message(&self, reply_token: &str, text: &str) -> Result<()> {
        let client = &self.client;

        let response = client
            .post("https://api.line.me/v2/bot/message/reply")
            .header(
                "Authorization",
                format!("Bearer {}", self.config.channel_access_token),
            )
            .json(&serde_json::json!({
                "replyToken": reply_token,
                "messages": [{"type": "text", "text": text}]
            }))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to send LINE reply: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "LINE API error {}: {}",
                status, body
            )));
        }

        tracing::info!("LINE reply sent");
        Ok(())
    }

    /// Send a push message
    pub async fn push_message(&self, user_id: &str, text: &str) -> Result<()> {
        self.push_messages(user_id, vec![LineMessageType::text(text)])
            .await
    }

    /// Send multiple LINE messages (supports text, image, flex)
    pub async fn push_messages(&self, user_id: &str, messages: Vec<LineMessageType>) -> Result<()> {
        let client = &self.client;

        let response = client
            .post("https://api.line.me/v2/bot/message/push")
            .header(
                "Authorization",
                format!("Bearer {}", self.config.channel_access_token),
            )
            .json(&serde_json::json!({
                "to": user_id,
                "messages": messages
            }))
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to send LINE push: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "LINE API error {}: {}",
                status, body
            )));
        }

        tracing::info!(user_id = %user_id, "LINE push messages sent");
        Ok(())
    }

    /// Send an image message
    pub async fn send_image(
        &self,
        user_id: &str,
        original_url: &str,
        preview_url: &str,
    ) -> Result<()> {
        self.push_messages(
            user_id,
            vec![LineMessageType::image(original_url, preview_url)],
        )
        .await
    }

    /// Send a flex message
    pub async fn send_flex(
        &self,
        user_id: &str,
        alt_text: &str,
        flex: FlexContainer,
    ) -> Result<()> {
        self.push_messages(user_id, vec![LineMessageType::flex(alt_text, flex)])
            .await
    }

    /// Process an incoming LINE webhook event
    pub fn process_webhook(&self, event: &LineWebhookEvent) -> Option<ChannelMessage> {
        // Only process message events
        if event.event_type != "message" {
            return None;
        }

        let message = event.message.as_ref()?;

        // Only process text messages
        if message.message_type != "text" {
            return None;
        }

        let text = message.text.clone()?;
        let user_id = event.source.as_ref()?.user_id.clone()?;
        let group_id = event.source.as_ref().and_then(|s| s.group_id.clone());

        let source = if let Some(group) = group_id {
            ChannelSource::with_chat("line", &user_id, &group)
        } else {
            ChannelSource::new("line", &user_id)
        };

        Some(ChannelMessage::new(source, text))
    }

    /// Validate webhook signature
    pub fn validate_signature(&self, body: &[u8], signature: &str) -> bool {
        if let Some(secret) = &self.config.channel_secret {
            use base64::{Engine as _, engine::general_purpose::STANDARD};
            use hmac::{Hmac, Mac};
            use sha2::Sha256;

            type HmacSha256 = Hmac<Sha256>;

            if let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) {
                mac.update(body);
                let result = mac.finalize();
                let expected = STANDARD.encode(result.into_bytes());
                return expected == signature;
            }
        }
        // If no secret configured, skip validation (not recommended for production)
        true
    }
}

#[async_trait]
impl ChannelAdapter for LineAdapter {
    fn channel_type(&self) -> &'static str {
        "line"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Webhook {
            path: self
                .config
                .webhook_path
                .clone()
                .unwrap_or_else(|| "/api/line".to_string()),
        }
    }

    async fn start(&self, _tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        self.connected.store(true, Ordering::SeqCst);
        tracing::info!("LINE adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.shutdown.notify_one();
        tracing::info!("LINE adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "line" {
            return Err(Error::channel("Invalid channel source for LINE"));
        }

        self.push_message(&to.user_id, content).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn handle_webhook(
        &self,
        payload: &[u8],
        tx: &mpsc::Sender<ChannelMessage>,
    ) -> Result<()> {
        let webhook: LineWebhook = serde_json::from_slice(payload)
            .map_err(|e| Error::Channel(format!("Failed to parse LINE webhook: {}", e)))?;

        for event in webhook.events {
            if let Some(message) = self.process_webhook(&event) {
                tx.send(message).await.map_err(|e| {
                    Error::Channel(format!("Failed to forward LINE message: {}", e))
                })?;
            }
        }

        Ok(())
    }
}

/// LINE webhook payload
#[derive(Debug, Clone, Deserialize)]
pub struct LineWebhook {
    /// Destination user ID
    pub destination: Option<String>,
    /// Events
    pub events: Vec<LineWebhookEvent>,
}

/// LINE webhook event
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineWebhookEvent {
    /// Event type
    #[serde(rename = "type")]
    pub event_type: String,
    /// Reply token
    pub reply_token: Option<String>,
    /// Timestamp
    pub timestamp: Option<i64>,
    /// Source
    pub source: Option<LineSource>,
    /// Message (for message events)
    pub message: Option<LineMessage>,
}

/// LINE source
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineSource {
    /// Source type (user, group, room)
    #[serde(rename = "type")]
    pub source_type: String,
    /// User ID
    pub user_id: Option<String>,
    /// Group ID
    pub group_id: Option<String>,
    /// Room ID
    pub room_id: Option<String>,
}

/// LINE message
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineMessage {
    /// Message ID
    pub id: String,
    /// Message type
    #[serde(rename = "type")]
    pub message_type: String,
    /// Text content (for text messages)
    pub text: Option<String>,
}

/// LINE message type for sending
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[allow(clippy::large_enum_variant)]
pub enum LineMessageType {
    Text {
        text: String,
    },
    Image {
        #[serde(rename = "originalContentUrl")]
        original_url: String,
        #[serde(rename = "previewImageUrl")]
        preview_url: String,
    },
    Flex {
        #[serde(rename = "altText")]
        alt_text: String,
        contents: FlexContainer,
    },
}

impl LineMessageType {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn image(original_url: impl Into<String>, preview_url: impl Into<String>) -> Self {
        Self::Image {
            original_url: original_url.into(),
            preview_url: preview_url.into(),
        }
    }

    pub fn flex(alt_text: impl Into<String>, contents: FlexContainer) -> Self {
        Self::Flex {
            alt_text: alt_text.into(),
            contents,
        }
    }
}

/// LINE Flex Message container
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[allow(clippy::large_enum_variant)]
pub enum FlexContainer {
    Bubble {
        #[serde(skip_serializing_if = "Option::is_none")]
        size: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        header: Option<FlexBox>,
        #[serde(skip_serializing_if = "Option::is_none")]
        hero: Option<FlexComponent>,
        #[serde(skip_serializing_if = "Option::is_none")]
        body: Option<FlexBox>,
        #[serde(skip_serializing_if = "Option::is_none")]
        footer: Option<FlexBox>,
    },
    Carousel {
        contents: Vec<FlexContainer>,
    },
}

impl FlexContainer {
    pub fn bubble() -> Self {
        Self::Bubble {
            size: None,
            header: None,
            hero: None,
            body: None,
            footer: None,
        }
    }

    pub fn with_body(mut self, body: FlexBox) -> Self {
        if let Self::Bubble {
            body: ref mut b, ..
        } = self
        {
            *b = Some(body);
        }
        self
    }

    pub fn with_header(mut self, header: FlexBox) -> Self {
        if let Self::Bubble {
            header: ref mut h, ..
        } = self
        {
            *h = Some(header);
        }
        self
    }

    pub fn with_footer(mut self, footer: FlexBox) -> Self {
        if let Self::Bubble {
            footer: ref mut f, ..
        } = self
        {
            *f = Some(footer);
        }
        self
    }
}

/// Flex box component
#[derive(Debug, Clone, Serialize)]
pub struct FlexBox {
    #[serde(rename = "type")]
    box_type: String,
    layout: String,
    contents: Vec<FlexComponent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spacing: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    margin: Option<String>,
}

impl FlexBox {
    pub fn vertical() -> Self {
        Self {
            box_type: "box".to_string(),
            layout: "vertical".to_string(),
            contents: Vec::new(),
            spacing: None,
            margin: None,
        }
    }

    pub fn horizontal() -> Self {
        Self {
            box_type: "box".to_string(),
            layout: "horizontal".to_string(),
            contents: Vec::new(),
            spacing: None,
            margin: None,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn add(mut self, component: FlexComponent) -> Self {
        self.contents.push(component);
        self
    }

    pub fn spacing(mut self, spacing: impl Into<String>) -> Self {
        self.spacing = Some(spacing.into());
        self
    }
}

/// Flex message component
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FlexComponent {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        size: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        weight: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        wrap: Option<bool>,
    },
    Button {
        action: FlexAction,
        #[serde(skip_serializing_if = "Option::is_none")]
        style: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    Image {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        size: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "aspectRatio")]
        aspect_ratio: Option<String>,
    },
    Separator {
        #[serde(skip_serializing_if = "Option::is_none")]
        margin: Option<String>,
    },
}

impl FlexComponent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            size: None,
            weight: None,
            color: None,
            wrap: None,
        }
    }

    pub fn button(label: impl Into<String>, uri: impl Into<String>) -> Self {
        Self::Button {
            action: FlexAction::uri(label, uri),
            style: None,
            color: None,
        }
    }

    pub fn image(url: impl Into<String>) -> Self {
        Self::Image {
            url: url.into(),
            size: None,
            aspect_ratio: None,
        }
    }

    pub fn separator() -> Self {
        Self::Separator { margin: None }
    }
}

/// Flex action
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FlexAction {
    Uri {
        label: String,
        uri: String,
    },
    Message {
        label: String,
        text: String,
    },
    Postback {
        label: String,
        data: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "displayText")]
        display_text: Option<String>,
    },
}

impl FlexAction {
    pub fn uri(label: impl Into<String>, uri: impl Into<String>) -> Self {
        Self::Uri {
            label: label.into(),
            uri: uri.into(),
        }
    }

    pub fn message(label: impl Into<String>, text: impl Into<String>) -> Self {
        Self::Message {
            label: label.into(),
            text: text.into(),
        }
    }

    pub fn postback(
        label: impl Into<String>,
        data: impl Into<String>,
        display_text: Option<String>,
    ) -> Self {
        Self::Postback {
            label: label.into(),
            data: data.into(),
            display_text,
        }
    }
}

/// LINE configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LineConfig {
    /// Channel access token
    #[serde(default)]
    pub channel_access_token: String,
    /// Channel secret (for webhook validation)
    #[serde(default)]
    pub channel_secret: Option<String>,
    /// Webhook path
    #[serde(default)]
    pub webhook_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_config_default() {
        let config = LineConfig::default();
        assert!(config.channel_access_token.is_empty());
    }

    #[tokio::test]
    async fn test_line_adapter_validation() {
        // Empty config should fail
        let config = LineConfig::default();
        assert!(LineAdapter::new(config).await.is_err());

        // Valid config should succeed
        let config = LineConfig {
            channel_access_token: "test-token".to_string(),
            ..Default::default()
        };
        assert!(LineAdapter::new(config).await.is_ok());
    }

    #[tokio::test]
    async fn test_line_adapter_lifecycle() {
        let config = LineConfig {
            channel_access_token: "test-token".to_string(),
            ..Default::default()
        };

        let adapter = LineAdapter::new(config)
            .await
            .expect("LineAdapter::new should succeed");
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "line");
    }

    #[test]
    fn test_line_webhook_parsing() {
        let json = r#"{
            "destination": "Utest",
            "events": [{
                "type": "message",
                "replyToken": "reply123",
                "source": {"type": "user", "userId": "U123"},
                "message": {"id": "msg1", "type": "text", "text": "Hello LINE!"}
            }]
        }"#;

        let webhook: LineWebhook = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(webhook.events.len(), 1);
        assert_eq!(webhook.events[0].event_type, "message");
    }

    #[tokio::test]
    async fn test_process_webhook() {
        let config = LineConfig {
            channel_access_token: "test-token".to_string(),
            ..Default::default()
        };
        let adapter = LineAdapter::new(config)
            .await
            .expect("LineAdapter::new should succeed");

        let event = LineWebhookEvent {
            event_type: "message".to_string(),
            reply_token: Some("token".to_string()),
            timestamp: Some(0),
            source: Some(LineSource {
                source_type: "user".to_string(),
                user_id: Some("U123".to_string()),
                group_id: None,
                room_id: None,
            }),
            message: Some(LineMessage {
                id: "msg1".to_string(),
                message_type: "text".to_string(),
                text: Some("Test message".to_string()),
            }),
        };

        let message = adapter
            .process_webhook(&event)
            .expect("process_webhook should succeed");
        assert_eq!(message.content, "Test message");
        assert_eq!(message.source.channel_type(), "line");
    }

    #[test]
    fn test_line_message_type_text() {
        let msg = LineMessageType::text("Hello");
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        assert!(json.contains("\"type\":\"text\""));
        assert!(json.contains("\"text\":\"Hello\""));
    }

    #[test]
    fn test_line_message_type_image() {
        let msg = LineMessageType::image(
            "https://example.com/original.jpg",
            "https://example.com/preview.jpg",
        );
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        assert!(json.contains("\"type\":\"image\""));
        assert!(json.contains("originalContentUrl"));
        assert!(json.contains("previewImageUrl"));
    }

    #[test]
    fn test_flex_container_bubble() {
        let bubble = FlexContainer::bubble().with_body(
            FlexBox::vertical()
                .add(FlexComponent::text("Hello"))
                .add(FlexComponent::separator()),
        );
        let json = serde_json::to_string(&bubble).expect("should serialize to JSON");
        assert!(json.contains("\"type\":\"bubble\""));
        assert!(json.contains("\"body\""));
    }

    #[test]
    fn test_flex_box_vertical() {
        let flex_box = FlexBox::vertical()
            .add(FlexComponent::text("Title"))
            .add(FlexComponent::text("Description"))
            .spacing("md");
        let json = serde_json::to_string(&flex_box).expect("should serialize to JSON");
        assert!(json.contains("\"layout\":\"vertical\""));
        assert!(json.contains("\"spacing\":\"md\""));
    }

    #[test]
    fn test_flex_component_text() {
        let comp = FlexComponent::text("Sample text");
        let json = serde_json::to_string(&comp).expect("should serialize to JSON");
        assert!(json.contains("\"type\":\"text\""));
        assert!(json.contains("\"text\":\"Sample text\""));
    }

    #[test]
    fn test_flex_component_button() {
        let comp = FlexComponent::button("Click me", "https://example.com");
        let json = serde_json::to_string(&comp).expect("should serialize to JSON");
        assert!(json.contains("\"type\":\"button\""));
        assert!(json.contains("\"label\":\"Click me\""));
        assert!(json.contains("\"uri\":\"https://example.com\""));
    }

    #[test]
    fn test_flex_action_uri() {
        let action = FlexAction::uri("Visit", "https://example.com");
        let json = serde_json::to_string(&action).expect("should serialize to JSON");
        assert!(json.contains("\"type\":\"uri\""));
        assert!(json.contains("\"label\":\"Visit\""));
    }

    #[test]
    fn test_flex_action_message() {
        let action = FlexAction::message("Say Hi", "Hello!");
        let json = serde_json::to_string(&action).expect("should serialize to JSON");
        assert!(json.contains("\"type\":\"message\""));
        assert!(json.contains("\"text\":\"Hello!\""));
    }

    #[test]
    fn test_flex_message_type() {
        let flex = FlexContainer::bubble()
            .with_body(FlexBox::vertical().add(FlexComponent::text("Test message")));
        let msg = LineMessageType::flex("Flex message", flex);
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        assert!(json.contains("\"type\":\"flex\""));
        assert!(json.contains("\"altText\":\"Flex message\""));
        assert!(json.contains("\"contents\""));
    }

    #[test]
    fn test_line_signature_validation() {
        let config = LineConfig {
            channel_access_token: "test-token".to_string(),
            channel_secret: Some("test-secret".to_string()),
            webhook_path: None,
        };
        let adapter = futures::executor::block_on(LineAdapter::new(config))
            .expect("LineAdapter::new should succeed");

        // Test with a known signature (this will fail because the signature is invalid)
        let body = b"test payload";
        let signature = "invalid";
        assert!(!adapter.validate_signature(body, signature));
    }

    #[test]
    fn test_line_signature_validation_no_secret() {
        let config = LineConfig {
            channel_access_token: "test-token".to_string(),
            channel_secret: None,
            webhook_path: None,
        };
        let adapter = futures::executor::block_on(LineAdapter::new(config))
            .expect("LineAdapter::new should succeed");

        // Should pass validation when no secret is configured
        let body = b"test payload";
        let signature = "any-signature";
        assert!(adapter.validate_signature(body, signature));
    }
}
