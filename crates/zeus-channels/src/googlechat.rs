//! Google Chat channel adapter
//!
//! Provides Google Chat messaging support via Google Workspace Chat API.
//! Uses service account authentication and webhook callbacks for receiving messages.

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, RwLock, mpsc};
use zeus_core::{Error, Result};

/// Token lifetime margin: refresh 60s before actual expiry to avoid races.
const TOKEN_EXPIRY_MARGIN_SECS: u64 = 60;
/// Google OAuth2 access tokens expire after 3600s; we use 3540s effective lifetime.
const TOKEN_LIFETIME_SECS: u64 = 3600 - TOKEN_EXPIRY_MARGIN_SECS;

/// Google Chat channel adapter
pub struct GoogleChatAdapter {
    connected: Arc<AtomicBool>,
    config: GoogleChatConfig,
    shutdown: Arc<Notify>,
    /// Shared HTTP client (connection pooling)
    client: reqwest::Client,
    /// Cached access token paired with its expiry Instant
    access_token: RwLock<Option<(String, Instant)>>,
}

impl GoogleChatAdapter {
    /// Create a new Google Chat adapter
    pub async fn new(config: GoogleChatConfig) -> Result<Self> {
        if config.service_account_key.is_empty() && config.credentials_path.is_none() {
            return Err(Error::Config(
                "Google Chat requires service_account_key or credentials_path".into(),
            ));
        }

        tracing::info!("Google Chat adapter created");

        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            config,
            shutdown: Arc::new(Notify::new()),
            client: reqwest::Client::new(),
            access_token: RwLock::new(None),
        })
    }

    /// Get an access token using service account credentials (JWT-based OAuth2).
    ///
    /// Priority:
    /// 1. Cached token (if not yet expired — tokens live ~3540s with 60s margin)
    /// 2. Pre-provided `access_token` in config (treated as long-lived; no expiry)
    /// 3. Service account JSON key (inline or file) → JWT → token exchange
    async fn get_access_token(&self) -> Result<String> {
        // Check cached token — only reuse if still within lifetime
        if let Some((token, issued_at)) = self.access_token.read().await.as_ref() {
            if issued_at.elapsed() < Duration::from_secs(TOKEN_LIFETIME_SECS) {
                return Ok(token.clone());
            }
            tracing::debug!("Google Chat: cached token expired, refreshing");
        }

        // Pre-configured access token (for dev/testing; assumed valid)
        if let Some(token) = &self.config.access_token {
            *self.access_token.write().await = Some((token.clone(), Instant::now()));
            return Ok(token.clone());
        }

        // Load service account key JSON
        let sa_json = if !self.config.service_account_key.is_empty() {
            self.config.service_account_key.clone()
        } else if let Some(path) = &self.config.credentials_path {
            tokio::fs::read_to_string(path)
                .await
                .map_err(|e| Error::Channel(format!("Failed to read credentials file: {}", e)))?
        } else {
            return Err(Error::Channel(
                "No access_token, service_account_key, or credentials_path configured".into(),
            ));
        };

        // Parse service account fields
        let sa: serde_json::Value = serde_json::from_str(&sa_json)
            .map_err(|e| Error::Channel(format!("Invalid service account JSON: {}", e)))?;

        let client_email = sa["client_email"]
            .as_str()
            .ok_or_else(|| Error::Channel("Missing client_email in service account key".into()))?;
        let private_key_pem = sa["private_key"]
            .as_str()
            .ok_or_else(|| Error::Channel("Missing private_key in service account key".into()))?;
        let token_uri = sa["token_uri"]
            .as_str()
            .unwrap_or("https://oauth2.googleapis.com/token");

        // Build JWT: header.payload.signature
        let now = chrono::Utc::now().timestamp();
        let header = serde_json::json!({"alg": "RS256", "typ": "JWT"});
        let claims = serde_json::json!({
            "iss": client_email,
            "scope": "https://www.googleapis.com/auth/chat.bot",
            "aud": token_uri,
            "iat": now,
            "exp": now + 3600,
        });

        let header_b64 = URL_SAFE_NO_PAD.encode(header.to_string().as_bytes());
        let claims_b64 = URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());
        let signing_input = format!("{}.{}", header_b64, claims_b64);

        // Parse PEM private key and sign with RS256
        let pem_bytes = pem_to_der(private_key_pem)
            .map_err(|e| Error::Channel(format!("Failed to parse private key PEM: {}", e)))?;
        let key_pair = ring::signature::RsaKeyPair::from_pkcs8(&pem_bytes)
            .map_err(|e| Error::Channel(format!("Invalid RSA key: {}", e)))?;

        let mut signature = vec![0u8; key_pair.public().modulus_len()];
        key_pair
            .sign(
                &ring::signature::RSA_PKCS1_SHA256,
                &ring::rand::SystemRandom::new(),
                signing_input.as_bytes(),
                &mut signature,
            )
            .map_err(|e| Error::Channel(format!("RSA signing failed: {}", e)))?;

        let signature_b64 = URL_SAFE_NO_PAD.encode(&signature);
        let jwt = format!("{}.{}", signing_input, signature_b64);

        // Exchange JWT for access token
        let client = &self.client;
        let resp = client
            .post(token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &jwt),
            ])
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Token exchange request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "Token exchange failed ({}): {}",
                status, body
            )));
        }

        let token_resp: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Failed to parse token response: {}", e)))?;

        let access_token = token_resp["access_token"]
            .as_str()
            .ok_or_else(|| Error::Channel("No access_token in token response".into()))?
            .to_string();

        *self.access_token.write().await = Some((access_token.clone(), Instant::now()));
        tracing::info!(
            "Google Chat: obtained access token via service account JWT (expires in ~{}s)",
            TOKEN_LIFETIME_SECS
        );

        Ok(access_token)
    }

    /// Send a message to a Google Chat space
    pub async fn send_message(&self, space_name: &str, text: &str) -> Result<()> {
        self.send_chat_message(space_name, ChatMessage::text(text))
            .await
    }

    /// Send a card message to a Google Chat space
    pub async fn send_card(&self, space_name: &str, card: ChatCard) -> Result<()> {
        self.send_chat_message(space_name, ChatMessage::card(card))
            .await
    }

    /// Send a chat message (text or card)
    async fn send_chat_message(&self, space_name: &str, message: ChatMessage) -> Result<()> {
        let token = self.get_access_token().await?;
        let client = &self.client;

        let url = format!("https://chat.googleapis.com/v1/{}/messages", space_name);

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&message.to_json())
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to send Google Chat message: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "Google Chat API error {}: {}",
                status, body
            )));
        }

        tracing::info!(space = %space_name, "Google Chat message sent");
        Ok(())
    }

    /// Process an incoming Google Chat event
    pub fn process_event(&self, event: &GoogleChatEvent) -> Option<ChannelMessage> {
        // Only process MESSAGE events
        if event.event_type != "MESSAGE" {
            return None;
        }

        let message = event.message.as_ref()?;
        let text = message.text.clone()?;
        let space_name = event.space.as_ref()?.name.clone();
        let sender = message.sender.as_ref()?;

        // Skip bot messages
        if sender.user_type == Some("BOT".to_string()) {
            return None;
        }

        let user_id = sender.name.clone();
        let source = ChannelSource::with_chat("googlechat", &user_id, &space_name);

        Some(ChannelMessage::new(source, text))
    }

    /// Test the connection
    pub async fn test_connection(&self) -> Result<bool> {
        let _token = self.get_access_token().await?;
        tracing::info!("Google Chat connection verified (token obtained)");
        Ok(true)
    }
}

#[async_trait]
impl ChannelAdapter for GoogleChatAdapter {
    fn channel_type(&self) -> &'static str {
        "googlechat"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Webhook {
            path: self
                .config
                .webhook_path
                .clone()
                .unwrap_or_else(|| "/api/googlechat".to_string()),
        }
    }

    async fn start(&self, _tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        self.connected.store(true, Ordering::SeqCst);
        tracing::info!("Google Chat adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.shutdown.notify_one();
        *self.access_token.write().await = None;
        tracing::info!("Google Chat adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "googlechat" {
            return Err(Error::channel("Invalid channel source for Google Chat"));
        }

        let space_name = to
            .chat_id
            .as_deref()
            .ok_or_else(|| Error::channel("Google Chat send requires a space name"))?;

        self.send_message(space_name, content).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn handle_webhook(
        &self,
        payload: &[u8],
        tx: &mpsc::Sender<ChannelMessage>,
    ) -> Result<()> {
        let event: GoogleChatEvent = serde_json::from_slice(payload)
            .map_err(|e| Error::Channel(format!("Failed to parse Google Chat event: {}", e)))?;

        if let Some(message) = self.process_event(&event) {
            tx.send(message).await.map_err(|e| {
                Error::Channel(format!("Failed to forward Google Chat message: {}", e))
            })?;
        }

        Ok(())
    }
}

/// Google Chat event
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleChatEvent {
    /// Event type (MESSAGE, ADDED_TO_SPACE, REMOVED_FROM_SPACE, etc.)
    #[serde(rename = "type")]
    pub event_type: String,
    /// Event time
    pub event_time: Option<String>,
    /// Space info
    pub space: Option<GoogleChatSpace>,
    /// Message (for MESSAGE events)
    pub message: Option<GoogleChatMessage>,
    /// User who triggered the event
    pub user: Option<GoogleChatUser>,
}

/// Google Chat space
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleChatSpace {
    /// Space resource name (spaces/xxx)
    pub name: String,
    /// Space display name
    pub display_name: Option<String>,
    /// Space type (ROOM, DM, etc.)
    #[serde(rename = "type")]
    pub space_type: Option<String>,
}

/// Google Chat message
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleChatMessage {
    /// Message resource name
    pub name: Option<String>,
    /// Message text
    pub text: Option<String>,
    /// Sender
    pub sender: Option<GoogleChatUser>,
    /// Create time
    pub create_time: Option<String>,
}

/// Google Chat user
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleChatUser {
    /// User resource name (users/xxx)
    pub name: String,
    /// Display name
    pub display_name: Option<String>,
    /// User type (HUMAN, BOT)
    #[serde(rename = "type")]
    pub user_type: Option<String>,
}

/// Chat message for sending
pub enum ChatMessage {
    Text { text: String },
    Card { card: ChatCard },
}

impl ChatMessage {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn card(card: ChatCard) -> Self {
        Self::Card { card }
    }

    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Text { text } => serde_json::json!({
                "text": text
            }),
            Self::Card { card } => serde_json::json!({
                "cardsV2": [{
                    "card": card
                }]
            }),
        }
    }
}

/// Google Chat Card (V2 format)
#[derive(Debug, Clone, Serialize)]
pub struct ChatCard {
    header: Option<ChatCardHeader>,
    sections: Vec<ChatCardSection>,
}

impl ChatCard {
    pub fn new() -> Self {
        Self {
            header: None,
            sections: Vec::new(),
        }
    }

    pub fn with_header(mut self, title: impl Into<String>) -> Self {
        self.header = Some(ChatCardHeader {
            title: title.into(),
            subtitle: None,
            image_url: None,
        });
        self
    }

    pub fn add_section(mut self, section: ChatCardSection) -> Self {
        self.sections.push(section);
        self
    }

    pub fn text_section(self, text: impl Into<String>) -> Self {
        self.add_section(ChatCardSection::new().add_widget(ChatCardWidget::text_paragraph(text)))
    }

    pub fn button_section(self, text: impl Into<String>, url: impl Into<String>) -> Self {
        self.add_section(ChatCardSection::new().add_widget(ChatCardWidget::button(text, url)))
    }
}

impl Default for ChatCard {
    fn default() -> Self {
        Self::new()
    }
}

/// Chat card header
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatCardHeader {
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_url: Option<String>,
}

/// Chat card section
#[derive(Debug, Clone, Serialize)]
pub struct ChatCardSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    header: Option<String>,
    widgets: Vec<ChatCardWidget>,
}

impl ChatCardSection {
    pub fn new() -> Self {
        Self {
            header: None,
            widgets: Vec::new(),
        }
    }

    pub fn with_header(mut self, header: impl Into<String>) -> Self {
        self.header = Some(header.into());
        self
    }

    pub fn add_widget(mut self, widget: ChatCardWidget) -> Self {
        self.widgets.push(widget);
        self
    }
}

impl Default for ChatCardSection {
    fn default() -> Self {
        Self::new()
    }
}

/// Chat card widget
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatCardWidget {
    TextParagraph { text: String },
    Image { image_url: String },
    ButtonList { buttons: Vec<ChatCardButton> },
    KeyValue { top_label: String, content: String },
}

impl ChatCardWidget {
    pub fn text_paragraph(text: impl Into<String>) -> Self {
        Self::TextParagraph { text: text.into() }
    }

    pub fn image(url: impl Into<String>) -> Self {
        Self::Image {
            image_url: url.into(),
        }
    }

    pub fn button(text: impl Into<String>, url: impl Into<String>) -> Self {
        Self::ButtonList {
            buttons: vec![ChatCardButton::text_button(text, url)],
        }
    }

    pub fn key_value(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self::KeyValue {
            top_label: label.into(),
            content: value.into(),
        }
    }
}

/// Chat card button
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatCardButton {
    text: String,
    on_click: ChatCardOnClick,
}

impl ChatCardButton {
    pub fn text_button(text: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            on_click: ChatCardOnClick::OpenLink { url: url.into() },
        }
    }
}

/// Chat card onClick action
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatCardOnClick {
    OpenLink { url: String },
}

/// Extract DER bytes from a PEM-encoded PKCS#8 private key.
fn pem_to_der(pem: &str) -> std::result::Result<Vec<u8>, String> {
    let mut lines = Vec::new();
    for line in pem.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("-----") || trimmed.is_empty() {
            continue;
        }
        lines.push(trimmed);
    }
    let b64 = lines.join("");
    base64::engine::general_purpose::STANDARD
        .decode(&b64)
        .map_err(|e| format!("Base64 decode error: {}", e))
}

/// Google Chat configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoogleChatConfig {
    /// Service account key JSON (inline)
    #[serde(default)]
    pub service_account_key: String,
    /// Path to credentials file
    #[serde(default)]
    pub credentials_path: Option<String>,
    /// Pre-configured access token (for development)
    #[serde(default, skip_serializing)]
    pub access_token: Option<String>,
    /// Webhook path for receiving messages
    #[serde(default)]
    pub webhook_path: Option<String>,
    /// Project ID
    #[serde(default)]
    pub project_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_googlechat_config_default() {
        let config = GoogleChatConfig::default();
        assert!(config.service_account_key.is_empty());
        assert!(config.credentials_path.is_none());
    }

    #[tokio::test]
    async fn test_googlechat_adapter_validation() {
        // Empty config should fail
        let config = GoogleChatConfig::default();
        assert!(GoogleChatAdapter::new(config).await.is_err());

        // Valid config should succeed
        let config = GoogleChatConfig {
            service_account_key: "test-key".to_string(),
            ..Default::default()
        };
        assert!(GoogleChatAdapter::new(config).await.is_ok());
    }

    #[tokio::test]
    async fn test_googlechat_adapter_lifecycle() {
        let config = GoogleChatConfig {
            service_account_key: "test-key".to_string(),
            ..Default::default()
        };

        let adapter = GoogleChatAdapter::new(config)
            .await
            .expect("GoogleChatAdapter::new should succeed");
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "googlechat");
    }

    #[test]
    fn test_googlechat_event_parsing() {
        let json = r#"{
            "type": "MESSAGE",
            "space": {"name": "spaces/abc123", "type": "ROOM"},
            "message": {
                "text": "Hello Google Chat!",
                "sender": {"name": "users/user123", "type": "HUMAN"}
            }
        }"#;

        let event: GoogleChatEvent = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(event.event_type, "MESSAGE");
        assert_eq!(
            event.message.as_ref().expect("as_ref should succeed").text,
            Some("Hello Google Chat!".to_string())
        );
    }

    #[tokio::test]
    async fn test_process_event() {
        let config = GoogleChatConfig {
            service_account_key: "test-key".to_string(),
            ..Default::default()
        };
        let adapter = GoogleChatAdapter::new(config)
            .await
            .expect("GoogleChatAdapter::new should succeed");

        let event = GoogleChatEvent {
            event_type: "MESSAGE".to_string(),
            event_time: None,
            space: Some(GoogleChatSpace {
                name: "spaces/abc".to_string(),
                display_name: Some("Test Space".to_string()),
                space_type: Some("ROOM".to_string()),
            }),
            message: Some(GoogleChatMessage {
                name: None,
                text: Some("Test message".to_string()),
                sender: Some(GoogleChatUser {
                    name: "users/123".to_string(),
                    display_name: Some("Test User".to_string()),
                    user_type: Some("HUMAN".to_string()),
                }),
                create_time: None,
            }),
            user: None,
        };

        let message = adapter
            .process_event(&event)
            .expect("process_event should succeed");
        assert_eq!(message.content, "Test message");
        assert_eq!(message.source.channel_type(), "googlechat");
    }

    #[test]
    fn test_chat_card_creation() {
        let card = ChatCard::new()
            .with_header("Test Card")
            .text_section("This is a test card");

        let json = serde_json::to_string(&card).expect("should serialize to JSON");
        assert!(json.contains("\"title\":\"Test Card\""));
        assert!(json.contains("This is a test card"));
    }

    #[test]
    fn test_chat_card_with_button() {
        let card = ChatCard::new().button_section("Click Here", "https://example.com");

        let json = serde_json::to_string(&card).expect("should serialize to JSON");
        assert!(json.contains("Click Here"));
        assert!(json.contains("https://example.com"));
    }

    #[test]
    fn test_chat_card_widget_text() {
        let widget = ChatCardWidget::text_paragraph("Sample text");
        let json = serde_json::to_string(&widget).expect("should serialize to JSON");
        assert!(json.contains("textParagraph"));
        assert!(json.contains("Sample text"));
    }

    #[test]
    fn test_chat_card_widget_image() {
        let widget = ChatCardWidget::image("https://example.com/image.jpg");
        let json = serde_json::to_string(&widget).expect("should serialize to JSON");
        assert!(json.contains("Image") || json.contains("imageUrl") || json.contains("image_url"));
        assert!(json.contains("https://example.com/image.jpg"));
    }

    #[test]
    fn test_chat_card_widget_key_value() {
        let widget = ChatCardWidget::key_value("Status", "Active");
        let json = serde_json::to_string(&widget).expect("should serialize to JSON");
        assert!(json.contains("keyValue"));
        assert!(json.contains("Status"));
        assert!(json.contains("Active"));
    }

    #[test]
    fn test_chat_message_text() {
        let message = ChatMessage::text("Hello Google Chat");
        let json = message.to_json();
        assert_eq!(json["text"], "Hello Google Chat");
    }

    #[test]
    fn test_chat_message_card() {
        let card = ChatCard::new().text_section("Card content");
        let message = ChatMessage::card(card);
        let json = message.to_json();
        assert!(json["cardsV2"].is_array());
        assert!(json["cardsV2"][0]["card"].is_object());
    }

    #[test]
    fn test_chat_card_section_with_header() {
        let section = ChatCardSection::new()
            .with_header("Section Title")
            .add_widget(ChatCardWidget::text_paragraph("Content"));

        let json = serde_json::to_string(&section).expect("should serialize to JSON");
        assert!(json.contains("Section Title"));
        assert!(json.contains("Content"));
    }

    #[test]
    fn test_chat_card_button() {
        let button = ChatCardButton::text_button("Visit", "https://example.com");
        let json = serde_json::to_string(&button).expect("should serialize to JSON");
        assert!(json.contains("Visit"));
        assert!(json.contains("openLink"));
    }

    #[test]
    fn test_chat_card_complex() {
        let card = ChatCard::new()
            .with_header("Welcome")
            .add_section(
                ChatCardSection::new()
                    .with_header("Details")
                    .add_widget(ChatCardWidget::text_paragraph("Description text"))
                    .add_widget(ChatCardWidget::key_value("Version", "1.0")),
            )
            .add_section(ChatCardSection::new().add_widget(ChatCardWidget::button(
                "Learn More",
                "https://docs.example.com",
            )));

        let json = serde_json::to_string(&card).expect("should serialize to JSON");
        assert!(json.contains("Welcome"));
        assert!(json.contains("Details"));
        assert!(json.contains("Learn More"));
    }
}
