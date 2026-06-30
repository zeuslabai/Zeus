//! Microsoft Teams channel adapter
//!
//! Provides Teams messaging support via Microsoft Graph API.
//! Supports sending messages and receiving via polling (Graph API pull model).

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, RwLock, mpsc};
use zeus_core::{Error, Result};

/// Cached OAuth2 access token with expiry
struct CachedToken {
    token: String,
    expires_at: Instant,
}

/// Microsoft Teams channel adapter (Microsoft Graph API)
pub struct TeamsAdapter {
    connected: Arc<AtomicBool>,
    config: TeamsConfig,
    shutdown: Arc<Notify>,
    token_cache: Arc<RwLock<Option<CachedToken>>>,
    client: reqwest::Client,
}

impl TeamsAdapter {
    /// Create a new Teams adapter
    pub async fn new(config: TeamsConfig) -> Result<Self> {
        if config.tenant_id.is_empty() {
            return Err(Error::Config("Teams tenant_id is required".into()));
        }
        if config.client_id.is_empty() {
            return Err(Error::Config("Teams client_id is required".into()));
        }
        if config.client_secret.is_empty() {
            return Err(Error::Config("Teams client_secret is required".into()));
        }
        if config.team_id.is_empty() {
            return Err(Error::Config("Teams team_id is required".into()));
        }
        if config.channel_id.is_empty() {
            return Err(Error::Config("Teams channel_id is required".into()));
        }

        tracing::info!("Teams adapter created (Graph API)");

        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            config,
            shutdown: Arc::new(Notify::new()),
            token_cache: Arc::new(RwLock::new(None)),
            client: reqwest::Client::new(),
        })
    }

    /// Acquire an OAuth2 access token via client credentials flow (cached)
    async fn get_access_token(&self) -> Result<String> {
        // Return cached token if still valid
        {
            let cache = self.token_cache.read().await;
            if let Some(ref cached) = *cache
                && cached.expires_at > Instant::now()
            {
                return Ok(cached.token.clone());
            }
        }

        let token_url = format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            self.config.tenant_id
        );

        let response = self
            .client
            .post(&token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.config.client_id.as_str()),
                ("client_secret", self.config.client_secret.as_str()),
                ("scope", "https://graph.microsoft.com/.default"),
            ])
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to get Teams token: {}", e)))?;

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: u64,
        }

        let token_resp: TokenResponse = response
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Failed to parse token response: {}", e)))?;

        let token = token_resp.access_token.clone();
        // Cache with 5-minute buffer before expiry
        let expires_at =
            Instant::now() + Duration::from_secs(token_resp.expires_in.saturating_sub(300));

        *self.token_cache.write().await = Some(CachedToken {
            token: token.clone(),
            expires_at,
        });

        Ok(token)
    }

    /// Send a text message to the configured Teams channel via Graph API
    ///
    /// Endpoint: POST /v1.0/teams/{team_id}/channels/{channel_id}/messages
    pub async fn send_message(&self, text: &str) -> Result<()> {
        let token = self.get_access_token().await?;
        let url = format!(
            "https://graph.microsoft.com/v1.0/teams/{}/channels/{}/messages",
            self.config.team_id, self.config.channel_id
        );

        let body = serde_json::json!({
            "body": {
                "contentType": "text",
                "content": text
            }
        });

        let response = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to send Teams message: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let err_body = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "Teams Graph API error {}: {}",
                status, err_body
            )));
        }

        tracing::info!("Teams message sent via Graph API");
        Ok(())
    }

    /// Poll for recent messages from the configured Teams channel via Graph API
    ///
    /// Endpoint: GET /v1.0/teams/{team_id}/channels/{channel_id}/messages
    pub async fn receive_messages(&self) -> Result<Vec<ChannelMessage>> {
        let token = self.get_access_token().await?;
        let url = format!(
            "https://graph.microsoft.com/v1.0/teams/{}/channels/{}/messages",
            self.config.team_id, self.config.channel_id
        );

        let response = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to poll Teams messages: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let err_body = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "Teams Graph API error {}: {}",
                status, err_body
            )));
        }

        #[derive(Deserialize)]
        struct GraphMessageBody {
            content: Option<String>,
        }

        #[derive(Deserialize)]
        struct GraphMessageUser {
            id: Option<String>,
        }

        #[derive(Deserialize)]
        struct GraphMessageFrom {
            user: Option<GraphMessageUser>,
        }

        #[derive(Deserialize)]
        struct GraphMessage {
            id: String,
            body: GraphMessageBody,
            from: Option<GraphMessageFrom>,
            #[serde(rename = "createdDateTime")]
            created_date_time: Option<String>,
        }

        #[derive(Deserialize)]
        struct GraphResponse {
            value: Vec<GraphMessage>,
        }

        let data: GraphResponse = response
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Failed to parse Teams messages: {}", e)))?;

        let channel_id = self.config.channel_id.clone();
        let messages = data
            .value
            .into_iter()
            .map(|msg| {
                let user_id = msg
                    .from
                    .and_then(|f| f.user)
                    .and_then(|u| u.id)
                    .unwrap_or_else(|| "unknown".to_string());

                let source = ChannelSource::with_chat("teams", &user_id, &channel_id);
                let content = msg.body.content.unwrap_or_default();
                let mut cm = ChannelMessage::new(source, content);
                cm.id = msg.id;
                if let Some(ts) = msg.created_date_time
                    && let Ok(parsed) = ts.parse::<chrono::DateTime<chrono::Utc>>()
                {
                    cm.timestamp = parsed;
                }
                cm
            })
            .collect();

        Ok(messages)
    }

    /// Get metadata for the configured Teams channel via Graph API
    ///
    /// Endpoint: GET /v1.0/teams/{team_id}/channels/{channel_id}
    pub async fn get_channel_info(&self) -> Result<serde_json::Value> {
        let token = self.get_access_token().await?;
        let url = format!(
            "https://graph.microsoft.com/v1.0/teams/{}/channels/{}",
            self.config.team_id, self.config.channel_id
        );

        let response = self
            .client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to get Teams channel info: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let err_body = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "Teams Graph API error {}: {}",
                status, err_body
            )));
        }

        let info: serde_json::Value = response
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Failed to parse Teams channel info: {}", e)))?;

        Ok(info)
    }

    /// Send an Adaptive Card to the configured Teams channel via Graph API
    pub async fn send_adaptive_card(&self, card: AdaptiveCard) -> Result<()> {
        let token = self.get_access_token().await?;
        let url = format!(
            "https://graph.microsoft.com/v1.0/teams/{}/channels/{}/messages",
            self.config.team_id, self.config.channel_id
        );

        let card_json = serde_json::to_string(&card)
            .map_err(|e| Error::Channel(format!("Failed to serialize adaptive card: {}", e)))?;

        let body = serde_json::json!({
            "body": {
                "contentType": "html",
                "content": "<attachment id=\"card\"></attachment>"
            },
            "attachments": [{
                "id": "card",
                "contentType": "application/vnd.microsoft.card.adaptive",
                "content": card_json
            }]
        });

        let response = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Failed to send Teams adaptive card: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let err_body = response.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "Teams Graph API error {}: {}",
                status, err_body
            )));
        }

        tracing::info!("Teams adaptive card sent via Graph API");
        Ok(())
    }

    /// Test the connection by acquiring a Graph API token
    pub async fn test_connection(&self) -> Result<bool> {
        let _token = self.get_access_token().await?;
        tracing::info!("Teams connection verified (Graph API token obtained)");
        Ok(true)
    }
}

#[async_trait]
impl ChannelAdapter for TeamsAdapter {
    fn channel_type(&self) -> &'static str {
        "teams"
    }

    /// Teams uses polling via Graph API (30-second interval)
    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Polling { interval_secs: 30 }
    }

    async fn start(&self, _tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        // Verify Graph API credentials by acquiring a token
        let _token = self.get_access_token().await?;
        self.connected.store(true, Ordering::SeqCst);
        tracing::info!("Teams adapter started (Graph API polling mode)");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.shutdown.notify_one();
        // Clear cached token on stop
        *self.token_cache.write().await = None;
        tracing::info!("Teams adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "teams" {
            return Err(Error::channel("Invalid channel source for Teams"));
        }
        self.send_message(content).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn handle_webhook(
        &self,
        _payload: &[u8],
        _tx: &mpsc::Sender<ChannelMessage>,
    ) -> Result<()> {
        // Teams adapter uses Graph API polling — webhooks are not supported
        Err(Error::Channel(
            "Teams adapter uses Graph API polling; webhooks are not supported".into(),
        ))
    }
}

/// Teams configuration for Microsoft Graph API
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamsConfig {
    /// Azure AD Tenant ID
    #[serde(default)]
    pub tenant_id: String,
    /// Azure AD Application (client) ID
    #[serde(default)]
    pub client_id: String,
    /// Azure AD Client secret
    #[serde(default, skip_serializing)]
    pub client_secret: String,
    /// Microsoft Teams team ID
    #[serde(default)]
    pub team_id: String,
    /// Teams channel ID within the team
    #[serde(default)]
    pub channel_id: String,
}

/// Adaptive Card for Teams (compatible with Graph API message attachments)
#[derive(Debug, Clone, Serialize)]
pub struct AdaptiveCard {
    #[serde(rename = "type")]
    card_type: String,
    #[serde(rename = "$schema")]
    schema: String,
    version: String,
    body: Vec<AdaptiveCardElement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actions: Option<Vec<AdaptiveCardAction>>,
}

impl AdaptiveCard {
    pub fn new() -> Self {
        Self {
            card_type: "AdaptiveCard".to_string(),
            schema: "http://adaptivecards.io/schemas/adaptive-card.json".to_string(),
            version: "1.4".to_string(),
            body: Vec::new(),
            actions: None,
        }
    }

    pub fn add_element(mut self, element: AdaptiveCardElement) -> Self {
        self.body.push(element);
        self
    }

    pub fn text_block(self, text: impl Into<String>) -> Self {
        self.add_element(AdaptiveCardElement::text_block(text))
    }

    pub fn text_block_large(self, text: impl Into<String>) -> Self {
        self.add_element(
            AdaptiveCardElement::text_block(text)
                .size("large")
                .weight("bolder"),
        )
    }

    pub fn image(self, url: impl Into<String>) -> Self {
        self.add_element(AdaptiveCardElement::image(url))
    }

    pub fn add_action(mut self, action: AdaptiveCardAction) -> Self {
        self.actions.get_or_insert_with(Vec::new).push(action);
        self
    }

    pub fn open_url_action(self, title: impl Into<String>, url: impl Into<String>) -> Self {
        self.add_action(AdaptiveCardAction::open_url(title, url))
    }
}

impl Default for AdaptiveCard {
    fn default() -> Self {
        Self::new()
    }
}

/// Adaptive Card element
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum AdaptiveCardElement {
    TextBlock {
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
    Image {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        size: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "altText")]
        alt_text: Option<String>,
    },
    FactSet {
        facts: Vec<Fact>,
    },
    Container {
        items: Vec<AdaptiveCardElement>,
        #[serde(skip_serializing_if = "Option::is_none")]
        style: Option<String>,
    },
}

impl AdaptiveCardElement {
    pub fn text_block(text: impl Into<String>) -> Self {
        Self::TextBlock {
            text: text.into(),
            size: None,
            weight: None,
            color: None,
            wrap: Some(true),
        }
    }

    pub fn size(mut self, size: impl Into<String>) -> Self {
        if let Self::TextBlock {
            size: ref mut s, ..
        } = self
        {
            *s = Some(size.into());
        }
        self
    }

    pub fn weight(mut self, weight: impl Into<String>) -> Self {
        if let Self::TextBlock {
            weight: ref mut w, ..
        } = self
        {
            *w = Some(weight.into());
        }
        self
    }

    pub fn color(mut self, color: impl Into<String>) -> Self {
        if let Self::TextBlock {
            color: ref mut c, ..
        } = self
        {
            *c = Some(color.into());
        }
        self
    }

    pub fn image(url: impl Into<String>) -> Self {
        Self::Image {
            url: url.into(),
            size: None,
            alt_text: None,
        }
    }

    pub fn fact_set(facts: Vec<Fact>) -> Self {
        Self::FactSet { facts }
    }

    pub fn container(items: Vec<AdaptiveCardElement>) -> Self {
        Self::Container { items, style: None }
    }
}

/// Fact for FactSet
#[derive(Debug, Clone, Serialize)]
pub struct Fact {
    pub title: String,
    pub value: String,
}

impl Fact {
    pub fn new(title: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            value: value.into(),
        }
    }
}

/// Adaptive Card action
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum AdaptiveCardAction {
    #[serde(rename = "Action.OpenUrl")]
    OpenUrl { title: String, url: String },
    #[serde(rename = "Action.Submit")]
    Submit {
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
}

impl AdaptiveCardAction {
    pub fn open_url(title: impl Into<String>, url: impl Into<String>) -> Self {
        Self::OpenUrl {
            title: title.into(),
            url: url.into(),
        }
    }

    pub fn submit(title: impl Into<String>, data: Option<serde_json::Value>) -> Self {
        Self::Submit {
            title: title.into(),
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> TeamsConfig {
        TeamsConfig {
            tenant_id: "test-tenant-id".to_string(),
            client_id: "test-client-id".to_string(),
            client_secret: "test-client-secret".to_string(),
            team_id: "test-team-id".to_string(),
            channel_id: "test-channel-id".to_string(),
        }
    }

    #[test]
    fn test_teams_config_fields() {
        let config = make_config();
        assert_eq!(config.tenant_id, "test-tenant-id");
        assert_eq!(config.client_id, "test-client-id");
        assert_eq!(config.client_secret, "test-client-secret");
        assert_eq!(config.team_id, "test-team-id");
        assert_eq!(config.channel_id, "test-channel-id");
    }

    #[test]
    fn test_teams_config_default() {
        let config = TeamsConfig::default();
        assert!(config.tenant_id.is_empty());
        assert!(config.client_id.is_empty());
        assert!(config.client_secret.is_empty());
        assert!(config.team_id.is_empty());
        assert!(config.channel_id.is_empty());
    }

    #[tokio::test]
    async fn test_teams_adapter_validation_empty() {
        let config = TeamsConfig::default();
        assert!(TeamsAdapter::new(config).await.is_err());
    }

    #[tokio::test]
    async fn test_teams_adapter_validation_missing_tenant() {
        let mut config = make_config();
        config.tenant_id = String::new();
        assert!(TeamsAdapter::new(config).await.is_err());
    }

    #[tokio::test]
    async fn test_teams_adapter_validation_missing_client_id() {
        let mut config = make_config();
        config.client_id = String::new();
        assert!(TeamsAdapter::new(config).await.is_err());
    }

    #[tokio::test]
    async fn test_teams_adapter_validation_missing_team_id() {
        let mut config = make_config();
        config.team_id = String::new();
        assert!(TeamsAdapter::new(config).await.is_err());
    }

    #[tokio::test]
    async fn test_teams_adapter_created_ok() {
        let adapter = TeamsAdapter::new(make_config())
            .await
            .expect("should create adapter with valid config");
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "teams");
    }

    #[tokio::test]
    async fn test_channel_type() {
        let adapter = TeamsAdapter::new(make_config()).await.unwrap();
        assert_eq!(adapter.channel_type(), "teams");
    }

    #[tokio::test]
    async fn test_receive_mode_is_polling() {
        let adapter = TeamsAdapter::new(make_config()).await.unwrap();
        match adapter.receive_mode() {
            ReceiveMode::Polling { interval_secs } => assert_eq!(interval_secs, 30),
            _ => panic!("expected ReceiveMode::Polling"),
        }
    }

    #[tokio::test]
    async fn test_is_connected_initially_false() {
        let adapter = TeamsAdapter::new(make_config()).await.unwrap();
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_handle_webhook_returns_error() {
        let adapter = TeamsAdapter::new(make_config()).await.unwrap();
        let (tx, _rx) = mpsc::channel(10);
        let result = adapter.handle_webhook(b"{}", &tx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_send_wrong_channel_type() {
        let adapter = TeamsAdapter::new(make_config()).await.unwrap();
        let source = ChannelSource::new("telegram", "user123");
        let result = adapter.send(&source, "hello").await;
        assert!(result.is_err());
    }

    // === AdaptiveCard type tests ===

    #[test]
    fn test_adaptive_card_creation() {
        let card = AdaptiveCard::new()
            .text_block_large("Welcome to Zeus")
            .text_block("This is an adaptive card message");

        let json = serde_json::to_string(&card).expect("should serialize");
        assert!(json.contains("\"type\":\"AdaptiveCard\""));
        assert!(json.contains("\"version\":\"1.4\""));
        assert!(json.contains("Welcome to Zeus"));
    }

    #[test]
    fn test_adaptive_card_with_image() {
        let card = AdaptiveCard::new()
            .image("https://example.com/image.jpg")
            .text_block("Caption");

        let json = serde_json::to_string(&card).expect("should serialize");
        assert!(json.contains("\"type\":\"Image\""));
        assert!(json.contains("https://example.com/image.jpg"));
    }

    #[test]
    fn test_adaptive_card_with_action() {
        let card = AdaptiveCard::new()
            .text_block("Click the button")
            .open_url_action("Visit Site", "https://example.com");

        let json = serde_json::to_string(&card).expect("should serialize");
        assert!(json.contains("Action.OpenUrl"));
        assert!(json.contains("Visit Site"));
        assert!(json.contains("https://example.com"));
    }

    #[test]
    fn test_adaptive_card_element_text_block() {
        let element = AdaptiveCardElement::text_block("Sample text")
            .size("large")
            .weight("bolder")
            .color("accent");

        let json = serde_json::to_string(&element).expect("should serialize");
        assert!(json.contains("\"type\":\"TextBlock\""));
        assert!(json.contains("\"size\":\"large\""));
        assert!(json.contains("\"weight\":\"bolder\""));
        assert!(json.contains("\"color\":\"accent\""));
    }

    #[test]
    fn test_adaptive_card_fact_set() {
        let facts = vec![Fact::new("Name", "Zeus"), Fact::new("Version", "1.0")];
        let element = AdaptiveCardElement::fact_set(facts);

        let json = serde_json::to_string(&element).expect("should serialize");
        assert!(json.contains("\"type\":\"FactSet\""));
        assert!(json.contains("Zeus"));
        assert!(json.contains("Version"));
    }

    #[test]
    fn test_adaptive_card_container() {
        let container = AdaptiveCardElement::container(vec![
            AdaptiveCardElement::text_block("Item 1"),
            AdaptiveCardElement::text_block("Item 2"),
        ]);

        let json = serde_json::to_string(&container).expect("should serialize");
        assert!(json.contains("\"type\":\"Container\""));
        assert!(json.contains("Item 1"));
        assert!(json.contains("Item 2"));
    }

    #[test]
    fn test_adaptive_card_action_submit() {
        let action =
            AdaptiveCardAction::submit("Submit", Some(serde_json::json!({"key": "value"})));
        let json = serde_json::to_string(&action).expect("should serialize");
        assert!(json.contains("Action.Submit"));
        assert!(json.contains("\"key\":\"value\""));
    }

    #[test]
    fn test_teams_config_serde_roundtrip() {
        let config = make_config();
        let json = serde_json::to_string(&config).expect("should serialize");
        let back: TeamsConfig = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(back.tenant_id, config.tenant_id);
        assert_eq!(back.client_id, config.client_id);
        assert_eq!(back.team_id, config.team_id);
        assert_eq!(back.channel_id, config.channel_id);
    }
}
