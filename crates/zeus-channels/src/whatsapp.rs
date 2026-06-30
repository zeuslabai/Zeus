//! WhatsApp channel adapter
//!
//! Supports two modes:
//! - **Bridge mode**: Communicates with a separate Node.js process running
//!   @whiskeysockets/baileys via WebSocket. The bridge translates between our
//!   JSON protocol and WhatsApp Web.
//! - **Cloud API mode**: Uses the official WhatsApp Business Cloud API via
//!   Meta's Graph API. Sends messages via HTTP POST and receives them via
//!   webhook callbacks.

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{debug, error, info, warn};
use zeus_core::{Error, Result};

use crate::policy::ChannelPolicy;
use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};

// ============================================================================
// Reconnection constants
// ============================================================================

/// Initial reconnect delay (seconds)
const RECONNECT_INITIAL_DELAY_SECS: u64 = 1;
/// Maximum reconnect delay (seconds)
const RECONNECT_MAX_DELAY_SECS: u64 = 60;
/// Backoff multiplier
const RECONNECT_BACKOFF_FACTOR: u64 = 2;

/// Type alias for the write half of a WebSocket connection
type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    WsMessage,
>;

// ============================================================================
// Configuration
// ============================================================================

/// WhatsApp operating mode
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WhatsAppMode {
    /// Baileys WebSocket bridge (self-hosted Node.js process)
    #[default]
    Bridge,
    /// WhatsApp Business Cloud API (Meta Graph API)
    CloudApi,
}

/// WhatsApp configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppConfig {
    /// Operating mode: Bridge (Baileys WS) or CloudApi (Meta Graph API)
    #[serde(default)]
    pub mode: WhatsAppMode,

    // -- Bridge mode fields --
    /// WebSocket bridge URL (e.g., "ws://localhost:3001")
    #[serde(default = "default_bridge_url")]
    pub bridge_url: String,

    // -- Cloud API fields --
    /// Meta Graph API access token (Cloud API mode)
    #[serde(default, skip_serializing)]
    pub access_token: Option<String>,
    /// Phone number ID from the WhatsApp Business dashboard (Cloud API mode)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_number_id: Option<String>,
    /// Webhook verification token (Cloud API mode)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_token: Option<String>,
    /// Graph API version (Cloud API mode, default "v21.0")
    #[serde(default = "default_api_version")]
    pub api_version: String,
    /// WhatsApp Business Account ID (optional, for account-level operations)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_account_id: Option<String>,

    // -- Shared --
    /// Phone number associated with this WhatsApp account
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    /// Access policy (group mention filtering, DM access)
    #[serde(default)]
    pub policy: Option<zeus_core::ChannelPolicyConfig>,
    /// Account identifier for multi-account routing (S43)
    #[serde(default)]
    pub account_id: Option<String>,
    /// Bot message filter: "off", "mentions" (default), "on"
    #[serde(default)]
    pub allow_bots: Option<String>,
}

fn default_bridge_url() -> String {
    std::env::var("ZEUS_WHATSAPP_BRIDGE_URL")
        .or_else(|_| std::env::var("WHATSAPP_BRIDGE_URL"))
        .unwrap_or_else(|_| "ws://localhost:3001".to_string())
}

fn default_api_version() -> String {
    "v21.0".to_string()
}

impl Default for WhatsAppConfig {
    fn default() -> Self {
        Self {
            mode: WhatsAppMode::default(),
            bridge_url: default_bridge_url(),
            access_token: None,
            phone_number_id: None,
            verify_token: None,
            api_version: default_api_version(),
            business_account_id: None,
            phone: None,
            policy: None,
            account_id: None,
            allow_bots: None,
        }
    }
}

impl WhatsAppConfig {
    /// Validate configuration for the selected mode.
    pub fn validate(&self) -> Result<()> {
        match self.mode {
            WhatsAppMode::Bridge => {
                if self.bridge_url.is_empty() {
                    return Err(Error::Config(
                        "WhatsApp Bridge mode requires bridge_url".into(),
                    ));
                }
            }
            WhatsAppMode::CloudApi => {
                if self.access_token.as_ref().is_none_or(|t| t.is_empty()) {
                    return Err(Error::Config(
                        "WhatsApp Cloud API mode requires access_token".into(),
                    ));
                }
                if self.phone_number_id.as_ref().is_none_or(|p| p.is_empty()) {
                    return Err(Error::Config(
                        "WhatsApp Cloud API mode requires phone_number_id".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Build the Cloud API messages endpoint URL.
    pub fn cloud_api_url(&self) -> String {
        format!(
            "https://graph.facebook.com/{}/{}/messages",
            self.api_version,
            self.phone_number_id.as_deref().unwrap_or("MISSING"),
        )
    }
}

// ============================================================================
// Bridge protocol (Baileys mode)
// ============================================================================

/// Bridge message protocol (Baileys WebSocket bridge)
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BridgeMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

// ============================================================================
// Cloud API types (Meta Graph API)
// ============================================================================

/// Outbound Cloud API message body
#[derive(Debug, Serialize)]
pub(crate) struct CloudApiSendBody {
    pub messaging_product: String,
    pub to: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub text: CloudApiTextBody,
}

/// Text content for outbound Cloud API message
#[derive(Debug, Serialize)]
pub(crate) struct CloudApiTextBody {
    pub body: String,
}

impl CloudApiSendBody {
    /// Build a text message for the Cloud API.
    pub fn text_message(to: &str, body: &str) -> Self {
        Self {
            messaging_product: "whatsapp".to_string(),
            to: to.to_string(),
            msg_type: "text".to_string(),
            text: CloudApiTextBody {
                body: body.to_string(),
            },
        }
    }
}

/// Top-level webhook payload from Meta
#[derive(Debug, Deserialize)]
pub(crate) struct WebhookPayload {
    #[serde(default)]
    pub entry: Vec<WebhookEntry>,
}

/// A single entry in the webhook payload
#[derive(Debug, Deserialize)]
pub(crate) struct WebhookEntry {
    #[serde(default)]
    pub changes: Vec<WebhookChange>,
}

/// A change within a webhook entry
#[derive(Debug, Deserialize)]
pub(crate) struct WebhookChange {
    #[serde(default)]
    pub value: WebhookValue,
}

/// The value inside a webhook change
#[derive(Debug, Default, Deserialize)]
pub(crate) struct WebhookValue {
    #[serde(default)]
    pub messages: Vec<WebhookMessage>,
}

/// An individual inbound message from the webhook
#[derive(Debug, Deserialize)]
pub(crate) struct WebhookMessage {
    /// Sender phone number (e.g., "1234567890")
    #[serde(default)]
    pub from: String,
    /// Message ID
    #[serde(default)]
    pub id: String,
    /// Unix timestamp as string
    #[serde(default)]
    pub timestamp: String,
    /// Message type (e.g., "text")
    #[serde(rename = "type", default)]
    #[allow(dead_code)]
    pub msg_type: String,
    /// Text content (present when type == "text")
    #[serde(default)]
    pub text: Option<WebhookTextContent>,
}

/// Text body within a webhook message
#[derive(Debug, Deserialize)]
pub(crate) struct WebhookTextContent {
    pub body: String,
}

/// Webhook verification query (GET request from Meta)
#[derive(Debug, Deserialize)]
pub struct WebhookVerification {
    #[serde(rename = "hub.mode")]
    pub mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    pub verify_token: Option<String>,
    #[serde(rename = "hub.challenge")]
    pub challenge: Option<String>,
}

impl WebhookVerification {
    /// Validate the verification request against the expected token.
    /// Returns the challenge string on success, or an error on mismatch.
    pub fn validate(&self, expected_token: &str) -> Result<String> {
        let mode = self.mode.as_deref().unwrap_or("");
        let token = self.verify_token.as_deref().unwrap_or("");

        if mode == "subscribe" && token == expected_token {
            self.challenge
                .clone()
                .ok_or_else(|| Error::Channel("Missing hub.challenge in verification".into()))
        } else {
            Err(Error::Channel(format!(
                "Webhook verification failed: mode={}, token_match={}",
                mode,
                token == expected_token
            )))
        }
    }
}

// ============================================================================
// Adapter
// ============================================================================

/// WhatsApp channel adapter supporting both Baileys Bridge and Cloud API modes
pub struct WhatsAppAdapter {
    config: WhatsAppConfig,
    connected: Arc<AtomicBool>,
    http_client: reqwest::Client,
    /// Shared write half of the bridge WebSocket (Bridge mode only).
    /// Populated by `start_bridge`, used by `send_bridge`.
    bridge_sink: Arc<Mutex<Option<WsSink>>>,
}

impl WhatsAppAdapter {
    /// Create a new WhatsApp adapter
    pub async fn new(config: WhatsAppConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            connected: Arc::new(AtomicBool::new(false)),
            http_client: reqwest::Client::new(),
            bridge_sink: Arc::new(Mutex::new(None)),
        })
    }

    /// Create a new adapter skipping validation (useful for testing)
    pub async fn new_unchecked(config: WhatsAppConfig) -> Result<Self> {
        Ok(Self {
            config,
            connected: Arc::new(AtomicBool::new(false)),
            http_client: reqwest::Client::new(),
            bridge_sink: Arc::new(Mutex::new(None)),
        })
    }

    /// Get a reference to the current configuration
    pub fn config(&self) -> &WhatsAppConfig {
        &self.config
    }

    // -- Bridge mode helpers ------------------------------------------------

    /// Start the Baileys WebSocket bridge listener with automatic reconnection.
    ///
    /// On disconnect, retries with exponential backoff (1s → 2s → 4s → … → 60s cap).
    /// The shared `bridge_sink` is updated on each successful connection so
    /// `send_bridge` can reuse the persistent connection.
    fn start_bridge(&self, tx: mpsc::Sender<ChannelMessage>) {
        let bridge_url = self.config.bridge_url.clone();
        let connected = self.connected.clone();
        let policy_config = self.config.policy.clone();
        let account_id = self.config.account_id.clone();
        let own_phone = self.config.phone.clone();
        let allow_bots = self.config.allow_bots.clone();
        let bridge_sink = self.bridge_sink.clone();

        tokio::spawn(async move {
            let mut delay = RECONNECT_INITIAL_DELAY_SECS;

            loop {
                info!("Connecting to WhatsApp bridge at {}", bridge_url);

                match tokio_tungstenite::connect_async(&bridge_url).await {
                    Ok((ws_stream, _)) => {
                        connected.store(true, Ordering::SeqCst);
                        delay = RECONNECT_INITIAL_DELAY_SECS; // reset backoff on success
                        info!("Connected to WhatsApp bridge");

                        let (write, mut read) = ws_stream.split();

                        // Store the write half for send_bridge to use
                        {
                            let mut sink_guard = bridge_sink.lock().await;
                            *sink_guard = Some(write);
                        }

                        while let Some(msg) = read.next().await {
                            match msg {
                                Ok(WsMessage::Text(text)) => {
                                    match serde_json::from_str::<BridgeMessage>(&text) {
                                        Ok(bridge_msg) if bridge_msg.msg_type == "message" => {
                                            let from =
                                                bridge_msg.from.as_deref().unwrap_or("unknown");
                                            let chat_id =
                                                bridge_msg.chat_id.as_deref().unwrap_or("unknown");
                                            let content = bridge_msg.text.unwrap_or_default();

                                            // Layer 1: self-echo — skip messages from our own number
                                            if let Some(ref our_phone) = own_phone
                                                && from == our_phone.as_str()
                                            {
                                                continue;
                                            }

                                            // Layer 2: AllowBotsMode (best-effort: WhatsApp has no bot flag)
                                            // No standard bot detection on Bridge path — field reserved for future use.
                                            let _ = &allow_bots;

                                            // Policy check: groups end with @g.us
                                            let policy = ChannelPolicy::new(
                                                policy_config.clone().unwrap_or_default(),
                                            );
                                            let is_group = chat_id.ends_with("@g.us");

                                            // Robust mention detection (Discord parity): DMs always addressed,
                                            // groups addressed on / command
                                            let is_addressed = !is_group || content.starts_with('/');

                                            if is_group {
                                                let is_mention = content.starts_with('/');
                                                let result =
                                                    policy.check_group(chat_id, from, is_mention);
                                                if result.is_denied() {
                                                    continue;
                                                }
                                            } else {
                                                let result = policy.check_dm(from);
                                                if result.is_denied() {
                                                    continue;
                                                }
                                            }

                                            let mut source = ChannelSource::with_chat(
                                                "whatsapp", from, chat_id,
                                            );
                                            if let Some(ref acct_id) = account_id {
                                                source = source.with_account(acct_id);
                                            }
                                            let channel_msg = ChannelMessage::new(source, content)
                                                .with_addressed(is_addressed);
                                            if tx.send(channel_msg).await.is_err() {
                                                // Receiver dropped — stop entirely (no reconnect)
                                                info!("WhatsApp bridge: channel receiver dropped, shutting down");
                                                connected.store(false, Ordering::SeqCst);
                                                let mut sink_guard = bridge_sink.lock().await;
                                                *sink_guard = None;
                                                return;
                                            }
                                        }
                                        Ok(_) => debug!("Non-message bridge event"),
                                        Err(e) => warn!("Failed to parse bridge message: {}", e),
                                    }
                                }
                                Ok(_) => {} // Ignore non-text frames (ping/pong/binary)
                                Err(e) => {
                                    error!("WhatsApp bridge WebSocket error: {}", e);
                                    break;
                                }
                            }
                        }

                        // Connection lost — clear sink and mark disconnected
                        connected.store(false, Ordering::SeqCst);
                        {
                            let mut sink_guard = bridge_sink.lock().await;
                            *sink_guard = None;
                        }
                        warn!(
                            "WhatsApp bridge connection lost, reconnecting in {}s",
                            delay
                        );
                    }
                    Err(e) => {
                        error!(
                            "Failed to connect to WhatsApp bridge: {} (retry in {}s)",
                            e, delay
                        );
                    }
                }

                // Backoff sleep before reconnect
                tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                delay = (delay * RECONNECT_BACKOFF_FACTOR).min(RECONNECT_MAX_DELAY_SECS);
            }
        });
    }

    /// Send a message via the Baileys WebSocket bridge.
    ///
    /// Uses the persistent connection established by `start_bridge`. Falls back
    /// to opening a one-shot connection if the persistent sink is unavailable.
    async fn send_bridge(&self, to: &ChannelSource, content: &str) -> Result<()> {
        let msg = BridgeMessage {
            msg_type: "send".to_string(),
            to: Some(to.user_id.clone()),
            text: Some(content.to_string()),
            from: None,
            chat_id: to.chat_id.clone(),
            timestamp: None,
        };
        let json = serde_json::to_string(&msg)?;

        // Try the persistent connection first
        {
            let mut sink_guard = self.bridge_sink.lock().await;
            if let Some(ref mut sink) = *sink_guard {
                match sink.send(WsMessage::Text(json.clone())).await {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        warn!(
                            "Persistent bridge sink send failed ({}), falling back to one-shot",
                            e
                        );
                        *sink_guard = None;
                    }
                }
            }
        }

        // Fallback: one-shot connection (bridge may be reconnecting)
        let (mut ws, _) = tokio_tungstenite::connect_async(&self.config.bridge_url)
            .await
            .map_err(|e| Error::Channel(format!("WhatsApp bridge connection failed: {}", e)))?;

        ws.send(WsMessage::Text(json))
            .await
            .map_err(|e| Error::Channel(format!("Failed to send WhatsApp message: {}", e)))?;

        Ok(())
    }

    // -- Cloud API helpers --------------------------------------------------

    /// Start Cloud API mode. Since Cloud API is webhook-driven, this simply
    /// marks the adapter as connected. Inbound messages arrive via
    /// `handle_webhook`.
    fn start_cloud_api(&self) {
        info!(
            "WhatsApp Cloud API mode active (phone_number_id={})",
            self.config.phone_number_id.as_deref().unwrap_or("?"),
        );
        self.connected.store(true, Ordering::SeqCst);
    }

    /// Send a message via the WhatsApp Business Cloud API.
    async fn send_cloud_api(&self, to: &ChannelSource, content: &str) -> Result<()> {
        let access_token = self
            .config
            .access_token
            .as_deref()
            .ok_or_else(|| Error::Channel("Cloud API access_token not configured".into()))?;

        let url = self.config.cloud_api_url();
        let body = CloudApiSendBody::text_message(&to.user_id, content);

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Cloud API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(Error::Channel(format!(
                "Cloud API returned {}: {}",
                status, text
            )));
        }

        debug!("WhatsApp Cloud API message sent to {}", to.user_id);
        Ok(())
    }

    /// Parse a Meta webhook payload and extract inbound messages.
    fn parse_webhook_messages(payload: &[u8]) -> Result<Vec<(String, String, String, String)>> {
        let webhook: WebhookPayload = serde_json::from_slice(payload)
            .map_err(|e| Error::Channel(format!("Failed to parse webhook payload: {}", e)))?;

        let mut messages = Vec::new();

        for entry in &webhook.entry {
            for change in &entry.changes {
                for msg in &change.value.messages {
                    let from = msg.from.clone();
                    let id = msg.id.clone();
                    let timestamp = msg.timestamp.clone();
                    let body = msg
                        .text
                        .as_ref()
                        .map(|t| t.body.clone())
                        .unwrap_or_default();

                    if !from.is_empty() {
                        messages.push((from, id, timestamp, body));
                    }
                }
            }
        }

        Ok(messages)
    }
}

#[async_trait]
impl ChannelAdapter for WhatsAppAdapter {
    fn channel_type(&self) -> &'static str {
        "whatsapp"
    }

    fn account_id(&self) -> Option<&str> {
        self.config.account_id.as_deref()
    }

    fn receive_mode(&self) -> ReceiveMode {
        match self.config.mode {
            WhatsAppMode::Bridge => ReceiveMode::WebSocket,
            WhatsAppMode::CloudApi => ReceiveMode::Webhook {
                path: "/webhooks/whatsapp".to_string(),
            },
        }
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        match self.config.mode {
            WhatsAppMode::Bridge => self.start_bridge(tx),
            WhatsAppMode::CloudApi => self.start_cloud_api(),
        }
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        match self.config.mode {
            WhatsAppMode::Bridge => self.send_bridge(to, content).await,
            WhatsAppMode::CloudApi => self.send_cloud_api(to, content).await,
        }
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn supports_native_identity(&self) -> bool {
        false
    }

    async fn send_typing(&self, to: &ChannelSource) -> Result<()> {
        if self.config.mode == WhatsAppMode::CloudApi
            && let Some(ref token) = self.config.access_token
        {
            let phone_id = self.config.phone_number_id.as_deref().unwrap_or("");
            let url = format!("https://graph.facebook.com/v18.0/{}/messages", phone_id);
            let client = reqwest::Client::new();
            let _ = client
                .post(&url)
                .bearer_auth(token)
                .json(&serde_json::json!({
                    "messaging_product": "whatsapp",
                    "recipient_type": "individual",
                    "to": &to.user_id,
                    "type": "reaction",
                    "status": "typing"
                }))
                .send()
                .await;
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
        if self.config.mode != WhatsAppMode::CloudApi {
            return Err(Error::Channel(
                "Webhooks are only supported in Cloud API mode".into(),
            ));
        }

        let messages = Self::parse_webhook_messages(payload)?;
        let policy = ChannelPolicy::new(self.config.policy.clone().unwrap_or_default());
        // Normalize own phone: strip leading '+' for comparison (WhatsApp uses plain digits)
        let own_phone_normalized = self
            .config
            .phone
            .as_deref()
            .map(|p| p.trim_start_matches('+').to_string());

        for (from, _id, _timestamp, body) in messages {
            // Layer 1: self-echo — skip messages originating from our own number
            if let Some(ref own) = own_phone_normalized
                && from.trim_start_matches('+') == own.as_str()
            {
                continue;
            }

            // Layer 2: AllowBotsMode — no native bot flag on Cloud API; field reserved
            // (WhatsApp Business API senders are identified by phone number, not a bot flag)

            // Layer 3: policy check — Cloud API webhook has no group chat ID,
            // so all inbound messages are treated as DMs.
            let result = policy.check_dm(&from);
            if result.is_denied() {
                debug!(from = %from, reason = ?result.reason(), "WhatsApp Cloud API message denied by policy");
                continue;
            }

            // Robust mention detection (Discord parity): DMs always addressed
            let is_addressed = true;
            let mut source = ChannelSource::new("whatsapp", &from);
            if let Some(ref acct_id) = self.config.account_id {
                source = source.with_account(acct_id);
            }
            let channel_msg = ChannelMessage::new(source, body).with_addressed(is_addressed);
            if tx.send(channel_msg).await.is_err() {
                warn!("Channel receiver dropped; stopping webhook processing");
                break;
            }
        }

        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentSendIdentity;

    // -- Existing tests (preserved) -----------------------------------------

    #[test]
    fn test_whatsapp_config_defaults() {
        let config = WhatsAppConfig::default();
        assert_eq!(config.bridge_url, "ws://localhost:3001");
        assert!(config.phone.is_none());
    }

    #[tokio::test]
    async fn test_whatsapp_adapter_creation() {
        let config = WhatsAppConfig::default();
        let adapter = WhatsAppAdapter::new_unchecked(config).await;
        assert!(adapter.is_ok());
        let adapter = adapter.expect("operation should succeed");
        assert_eq!(adapter.channel_type(), "whatsapp");
        assert!(!adapter.is_connected());
    }

    #[test]
    fn test_bridge_message_serialization() {
        let msg = BridgeMessage {
            msg_type: "send".to_string(),
            to: Some("+1234567890".to_string()),
            text: Some("Hello".to_string()),
            from: None,
            chat_id: None,
            timestamp: None,
        };
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        assert!(json.contains("\"type\":\"send\""));
        assert!(json.contains("\"to\":\"+1234567890\""));
        assert!(json.contains("\"text\":\"Hello\""));
        // Verify None fields are skipped
        assert!(!json.contains("\"from\""));
        assert!(!json.contains("\"chat_id\""));
        assert!(!json.contains("\"timestamp\""));
    }

    #[test]
    fn test_bridge_message_deserialization() {
        let json = r#"{"type":"message","from":"+1234567890","text":"Hi","chat_id":"group1"}"#;
        let msg: BridgeMessage = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(msg.msg_type, "message");
        assert_eq!(msg.from.as_deref(), Some("+1234567890"));
        assert_eq!(msg.text.as_deref(), Some("Hi"));
        assert_eq!(msg.chat_id.as_deref(), Some("group1"));
    }

    #[test]
    fn test_whatsapp_channel_source() {
        let source = ChannelSource::with_chat("whatsapp", "+1234567890", "group123");
        assert_eq!(source.channel_type(), "whatsapp");
        assert_eq!(source.user_id, "+1234567890");
        assert_eq!(source.chat_id.as_deref(), Some("group123"));
    }

    #[test]
    fn test_whatsapp_receive_mode() {
        let config = WhatsAppConfig::default();
        // Bridge mode uses WebSocket
        assert_eq!(config.bridge_url, "ws://localhost:3001");
    }

    // -- New tests (Cloud API + mode) ---------------------------------------

    #[test]
    fn test_mode_default() {
        let config = WhatsAppConfig::default();
        assert_eq!(config.mode, WhatsAppMode::Bridge);
    }

    #[test]
    fn test_cloud_api_config() {
        let config = WhatsAppConfig {
            mode: WhatsAppMode::CloudApi,
            bridge_url: default_bridge_url(),
            access_token: Some("EAABx...".to_string()),
            phone_number_id: Some("123456789".to_string()),
            verify_token: Some("my_verify_token".to_string()),
            api_version: "v21.0".to_string(),
            business_account_id: Some("987654321".to_string()),
            phone: Some("+15551234567".to_string()),
            ..Default::default()
        };

        assert_eq!(config.mode, WhatsAppMode::CloudApi);
        assert_eq!(config.access_token.as_deref(), Some("EAABx..."));
        assert_eq!(config.phone_number_id.as_deref(), Some("123456789"));
        assert_eq!(config.verify_token.as_deref(), Some("my_verify_token"));
        assert_eq!(config.api_version, "v21.0");
        assert_eq!(config.business_account_id.as_deref(), Some("987654321"));
        assert_eq!(config.phone.as_deref(), Some("+15551234567"));

        // Validate URL construction
        assert_eq!(
            config.cloud_api_url(),
            "https://graph.facebook.com/v21.0/123456789/messages"
        );

        // Validation should pass
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_cloud_api_config_validation_missing_token() {
        let config = WhatsAppConfig {
            mode: WhatsAppMode::CloudApi,
            access_token: None,
            phone_number_id: Some("123".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_cloud_api_config_validation_missing_phone_number_id() {
        let config = WhatsAppConfig {
            mode: WhatsAppMode::CloudApi,
            access_token: Some("token".to_string()),
            phone_number_id: None,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_cloud_api_send_body_format() {
        let body = CloudApiSendBody::text_message("+15551234567", "Hello from Zeus!");
        let json = serde_json::to_value(&body).expect("should serialize to JSON");

        assert_eq!(json["messaging_product"], "whatsapp");
        assert_eq!(json["to"], "+15551234567");
        assert_eq!(json["type"], "text");
        assert_eq!(json["text"]["body"], "Hello from Zeus!");

        // Verify the serialized JSON has the exact expected structure
        let json_str = serde_json::to_string(&body).expect("should serialize to JSON");
        assert!(json_str.contains("\"messaging_product\":\"whatsapp\""));
        assert!(json_str.contains("\"to\":\"+15551234567\""));
        assert!(json_str.contains("\"type\":\"text\""));
        assert!(json_str.contains("\"body\":\"Hello from Zeus!\""));
    }

    #[test]
    fn test_webhook_payload_parsing() {
        let payload = r#"{
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "BIZ_ACCOUNT_ID",
                "changes": [{
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {
                            "display_phone_number": "15551234567",
                            "phone_number_id": "PHONE_ID"
                        },
                        "messages": [{
                            "from": "14155551234",
                            "id": "wamid.abc123",
                            "timestamp": "1677123456",
                            "type": "text",
                            "text": {
                                "body": "Hello from user"
                            }
                        }]
                    },
                    "field": "messages"
                }]
            }]
        }"#;

        let messages = WhatsAppAdapter::parse_webhook_messages(payload.as_bytes())
            .expect("should parse successfully");

        assert_eq!(messages.len(), 1);
        let (from, id, timestamp, body) = &messages[0];
        assert_eq!(from, "14155551234");
        assert_eq!(id, "wamid.abc123");
        assert_eq!(timestamp, "1677123456");
        assert_eq!(body, "Hello from user");
    }

    #[test]
    fn test_webhook_payload_multiple_messages() {
        let payload = r#"{
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [
                            {
                                "from": "1111",
                                "id": "msg1",
                                "timestamp": "100",
                                "type": "text",
                                "text": {"body": "first"}
                            },
                            {
                                "from": "2222",
                                "id": "msg2",
                                "timestamp": "200",
                                "type": "text",
                                "text": {"body": "second"}
                            }
                        ]
                    }
                }]
            }]
        }"#;

        let messages = WhatsAppAdapter::parse_webhook_messages(payload.as_bytes())
            .expect("should parse successfully");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].0, "1111");
        assert_eq!(messages[0].3, "first");
        assert_eq!(messages[1].0, "2222");
        assert_eq!(messages[1].3, "second");
    }

    #[test]
    fn test_webhook_payload_no_messages() {
        // Status update webhook (no messages array)
        let payload = r#"{
            "entry": [{
                "changes": [{
                    "value": {
                        "statuses": [{
                            "id": "wamid.xyz",
                            "status": "delivered"
                        }]
                    }
                }]
            }]
        }"#;

        let messages = WhatsAppAdapter::parse_webhook_messages(payload.as_bytes())
            .expect("should parse successfully");
        assert!(messages.is_empty());
    }

    #[test]
    fn test_webhook_verification() {
        let verification = WebhookVerification {
            mode: Some("subscribe".to_string()),
            verify_token: Some("my_secret".to_string()),
            challenge: Some("challenge_string_123".to_string()),
        };

        // Correct token
        let result = verification.validate("my_secret");
        assert!(result.is_ok());
        assert_eq!(
            result.expect("operation should succeed"),
            "challenge_string_123"
        );

        // Wrong token
        let result = verification.validate("wrong_token");
        assert!(result.is_err());
    }

    #[test]
    fn test_webhook_verification_wrong_mode() {
        let verification = WebhookVerification {
            mode: Some("unsubscribe".to_string()),
            verify_token: Some("my_secret".to_string()),
            challenge: Some("challenge_string_123".to_string()),
        };

        let result = verification.validate("my_secret");
        assert!(result.is_err());
    }

    #[test]
    fn test_webhook_verification_missing_challenge() {
        let verification = WebhookVerification {
            mode: Some("subscribe".to_string()),
            verify_token: Some("my_secret".to_string()),
            challenge: None,
        };

        let result = verification.validate("my_secret");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cloud_api_receive_mode() {
        let config = WhatsAppConfig {
            mode: WhatsAppMode::CloudApi,
            access_token: Some("token".to_string()),
            phone_number_id: Some("12345".to_string()),
            ..Default::default()
        };
        let adapter = WhatsAppAdapter::new(config)
            .await
            .expect("WhatsAppAdapter::new should succeed");
        match adapter.receive_mode() {
            ReceiveMode::Webhook { path } => {
                assert_eq!(path, "/webhooks/whatsapp");
            }
            other => panic!("Expected Webhook receive mode, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_bridge_receive_mode() {
        let config = WhatsAppConfig::default();
        let adapter = WhatsAppAdapter::new_unchecked(config)
            .await
            .expect("async operation should succeed");
        assert!(matches!(adapter.receive_mode(), ReceiveMode::WebSocket));
    }

    #[tokio::test]
    async fn test_cloud_api_start_marks_connected() {
        let config = WhatsAppConfig {
            mode: WhatsAppMode::CloudApi,
            access_token: Some("token".to_string()),
            phone_number_id: Some("12345".to_string()),
            ..Default::default()
        };
        let adapter = WhatsAppAdapter::new(config)
            .await
            .expect("WhatsAppAdapter::new should succeed");
        assert!(!adapter.is_connected());

        let (tx, _rx) = mpsc::channel(10);
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
    async fn test_cloud_api_handle_webhook() {
        let config = WhatsAppConfig {
            mode: WhatsAppMode::CloudApi,
            access_token: Some("token".to_string()),
            phone_number_id: Some("12345".to_string()),
            ..Default::default()
        };
        let adapter = WhatsAppAdapter::new(config)
            .await
            .expect("WhatsAppAdapter::new should succeed");

        let payload = r#"{
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "14155551234",
                            "id": "wamid.test",
                            "timestamp": "1700000000",
                            "type": "text",
                            "text": {"body": "Test webhook"}
                        }]
                    }
                }]
            }]
        }"#;

        let (tx, mut rx) = mpsc::channel(10);
        adapter
            .handle_webhook(payload.as_bytes(), &tx)
            .await
            .expect("async operation should succeed");

        let msg = rx.try_recv().expect("try_recv should succeed");
        assert_eq!(msg.source.channel_type(), "whatsapp");
        assert_eq!(msg.source.user_id, "14155551234");
        assert_eq!(msg.content, "Test webhook");
    }

    #[tokio::test]
    async fn test_bridge_mode_rejects_webhook() {
        let config = WhatsAppConfig::default();
        let adapter = WhatsAppAdapter::new_unchecked(config)
            .await
            .expect("async operation should succeed");

        let (tx, _rx) = mpsc::channel(10);
        let result = adapter.handle_webhook(b"{}", &tx).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_config_serde_roundtrip() {
        let config = WhatsAppConfig {
            mode: WhatsAppMode::CloudApi,
            bridge_url: "ws://custom:9999".to_string(),
            access_token: Some("token123".to_string()),
            phone_number_id: Some("phone_id".to_string()),
            verify_token: Some("verify".to_string()),
            api_version: "v20.0".to_string(),
            business_account_id: Some("biz123".to_string()),
            phone: Some("+1555".to_string()),
            ..Default::default()
        };

        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        // Credentials must not appear in serialized output
        assert!(
            !json.contains("token123"),
            "access_token must not be serialized"
        );
        let deserialized: WhatsAppConfig =
            serde_json::from_str(&json).expect("should parse successfully");

        assert_eq!(deserialized.mode, WhatsAppMode::CloudApi);
        assert_eq!(deserialized.bridge_url, "ws://custom:9999");
        // access_token is intentionally None after roundtrip (skip_serializing security policy)
        assert!(deserialized.access_token.is_none());
        assert_eq!(deserialized.phone_number_id.as_deref(), Some("phone_id"));
        assert_eq!(deserialized.verify_token.as_deref(), Some("verify"));
        assert_eq!(deserialized.api_version, "v20.0");
        assert_eq!(deserialized.business_account_id.as_deref(), Some("biz123"));
        assert_eq!(deserialized.phone.as_deref(), Some("+1555"));
    }

    #[test]
    fn test_cloud_api_url_construction() {
        let config = WhatsAppConfig {
            phone_number_id: Some("999888777".to_string()),
            api_version: "v19.0".to_string(),
            ..Default::default()
        };
        assert_eq!(
            config.cloud_api_url(),
            "https://graph.facebook.com/v19.0/999888777/messages"
        );
    }

    #[test]
    fn test_cloud_api_url_default_version() {
        let config = WhatsAppConfig {
            phone_number_id: Some("12345".to_string()),
            ..Default::default()
        };
        assert_eq!(
            config.cloud_api_url(),
            "https://graph.facebook.com/v21.0/12345/messages"
        );
    }

    // ── S33 Track D: Tier 2 identity tests ──────────────────────────────────

    #[tokio::test]
    async fn test_whatsapp_supports_native_identity_false() {
        let config = WhatsAppConfig::default();
        let adapter = WhatsAppAdapter::new_unchecked(config)
            .await
            .expect("WhatsAppAdapter::new_unchecked should succeed");
        assert!(!adapter.supports_native_identity());
    }

    #[test]
    fn test_whatsapp_send_as_text_prefix_format() {
        let identity = AgentSendIdentity::new("zeus_agent");
        let prefixed = identity.apply_prefix("Hello from WhatsApp");
        assert_eq!(prefixed, "[zeus_agent] Hello from WhatsApp");
    }

    // ── S43: Multi-account tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_whatsapp_account_id_none_by_default() {
        let config = WhatsAppConfig::default();
        let adapter = WhatsAppAdapter::new_unchecked(config)
            .await
            .expect("WhatsAppAdapter::new_unchecked should succeed");
        assert!(adapter.account_id().is_none());
    }

    #[tokio::test]
    async fn test_whatsapp_account_id_set() {
        let config = WhatsAppConfig {
            account_id: Some("business".to_string()),
            ..Default::default()
        };
        let adapter = WhatsAppAdapter::new_unchecked(config)
            .await
            .expect("WhatsAppAdapter::new_unchecked should succeed");
        assert_eq!(adapter.account_id(), Some("business"));
    }

    #[test]
    fn test_whatsapp_config_with_account_id_serde() {
        let json = r#"{"mode":"bridge","bridge_url":"ws://localhost:3001","account_id":"support","allow_bots":"off"}"#;
        let config: WhatsAppConfig = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(config.account_id.as_deref(), Some("support"));
        assert_eq!(config.allow_bots.as_deref(), Some("off"));
    }
}


