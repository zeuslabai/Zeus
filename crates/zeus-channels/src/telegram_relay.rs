//! Smart Telegram Relay - Bidirectional messaging between Telegram and Zeus sessions
//!
//! Zeus Smart Relay features:
//!
//! - **Bidirectional relay**: Background polling + message forwarding to Zeus sessions
//! - **Auto-detect message intent**: Command, Chat, Media, Callback classification
//! - **Inline keyboard builder**: Fluent API for approval/action button layouts
//! - **Group/DM routing**: `SessionRouter` with per-chat session state and thread awareness
//! - **Thread awareness**: reply_to support for message threading
//! - **Message chunking**: Auto-split long messages (Telegram 4096 char limit)
//! - **Typing indicators**: Send "typing..." status to chats
//! - **Photo/document sending**: Rich media support via Bot API
//! - **Message editing**: Edit previously sent messages
//! - **Security**: Username allowlist, rate limiting per sender

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use zeus_core::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use crate::filters::AllowBotsMode;

use crate::policy::{ChannelPolicy, PolicyResult};

// ============================================================================
// Markdown sanitization (Telegram outbound)
// ============================================================================


// ============================================================================
// Configuration
// ============================================================================

/// Configuration for Telegram relay
///
/// All fields are sourced from `config.toml` under `[channels.telegram_relay]`.
/// No environment variable fallbacks — `~/.zeus/config.toml` is the single source of truth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramRelayConfig {
    /// Bot token from @BotFather. Must be set in config.toml.
    #[serde(default, skip_serializing)]
    pub bot_token: String,
    /// Default chat ID to send/receive messages. Must be set in config.toml.
    #[serde(default)]
    pub chat_id: String,
    /// Allowed usernames (comma-separated, case-insensitive)
    #[serde(default)]
    pub allowed_users: Option<String>,
    /// Target Zeus session ID (must be set in config.toml if needed)
    #[serde(default)]
    pub target_session: Option<String>,
    /// Maximum message length before chunking (default: 4000)
    #[serde(default = "default_max_message_length")]
    pub max_message_length: usize,
    /// Rate limit: max messages per sender per minute (default: 30)
    #[serde(default = "default_rate_limit")]
    pub rate_limit_per_minute: usize,
    /// Enable group chat support (default: true)
    #[serde(default = "default_true")]
    pub enable_groups: bool,
    /// Require @mention in group chats (default: false — bots see all group messages)
    #[serde(default)]
    pub require_mention_in_groups: bool,
    /// Bot username (without @, auto-detected if not set)
    #[serde(default)]
    pub bot_username: Option<String>,
    /// Access policy (group mention filtering, DM access).
    /// When set, overrides `require_mention_in_groups` with the policy's group setting.
    #[serde(default)]
    pub policy: Option<zeus_core::ChannelPolicyConfig>,
    /// When set, bind an HTTP webhook listener on this port instead of long-polling.
    #[serde(default)]
    pub webhook_port: Option<u16>,
    /// URL path for the webhook endpoint (default: `/telegram/webhook`).
    #[serde(default = "default_webhook_path")]
    pub webhook_path: String,
    /// Public HTTPS URL to register with Telegram's `setWebhook` API.
    /// If set together with `webhook_port`, the relay calls `setWebhook` on startup.
    #[serde(default)]
    pub webhook_url: Option<String>,
    /// Allow bot-authored messages through the relay (`off` | `mentions` | `on`).
    /// Default: `mentions` — only bot messages that @mention this bot pass through.
    /// Self-echo is always blocked regardless of this setting.
    #[serde(default)]
    pub allow_bots: Option<String>,
    /// Fleet bot allowlist — Telegram user IDs (as strings) that bypass the
    /// Layer 2 mention filter. Lets fleet titans reply-chain coordinate without
    /// triggering bot-loop prevention. External bots still gated by `allow_bots`.
    /// Empty (default) = standard `allow_bots` behavior, no allowlist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fleet_bot_ids: Vec<String>,
    /// Resolved STT provider for voice messages. Populated at the gateway boundary
    /// from `zeus_core::Config`. Not serialized — this is runtime wiring, not config.
    #[serde(skip)]
    pub stt_provider: Option<crate::telegram_voice::SttProvider>,
    /// Resolved TTS provider for voice replies. Populated at the gateway boundary
    /// from `zeus_core::Config`. Not serialized — this is runtime wiring, not config.
    #[serde(skip)]
    pub tts_provider: Option<crate::telegram_voice::TtsProvider>,
}

fn default_max_message_length() -> usize {
    4000
}
fn default_rate_limit() -> usize {
    30
}
fn default_true() -> bool {
    true
}
fn default_webhook_path() -> String {
    "/telegram/webhook".to_string()
}

// ============================================================================
// Message Intent Detection
// ============================================================================

/// Auto-detected intent of an incoming Telegram message
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageIntent {
    /// Bot command (e.g., /status, /help, /session abc)
    Command { name: String, args: Vec<String> },
    /// Regular chat message
    Chat { text: String },
    /// Media message (photo, document, audio, video, etc.)
    Media {
        media_type: MediaType,
        caption: Option<String>,
    },
    /// Inline keyboard callback button press
    Callback { action: String, data: String },
    /// Voice message
    VoiceMessage { file_id: Option<String> },
}

/// Type of media in a Telegram message
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaType {
    Photo,
    Document,
    Audio,
    Video,
    Sticker,
    Voice,
    VideoNote,
    Location,
    Contact,
}

impl std::fmt::Display for MediaType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Photo => write!(f, "photo"),
            Self::Document => write!(f, "document"),
            Self::Audio => write!(f, "audio"),
            Self::Video => write!(f, "video"),
            Self::Sticker => write!(f, "sticker"),
            Self::Voice => write!(f, "voice"),
            Self::VideoNote => write!(f, "video_note"),
            Self::Location => write!(f, "location"),
            Self::Contact => write!(f, "contact"),
        }
    }
}

/// Detect intent from a raw Telegram update JSON
pub fn detect_intent(update: &serde_json::Value) -> MessageIntent {
    // Callback query
    if let Some(callback) = update.get("callback_query") {
        let data = callback["data"].as_str().unwrap_or("").to_string();
        let parts: Vec<&str> = data.splitn(2, ':').collect();
        let action = parts.first().unwrap_or(&"").to_string();
        let extra = parts.get(1).unwrap_or(&"").to_string();
        return MessageIntent::Callback {
            action,
            data: extra,
        };
    }

    // Message
    if let Some(msg) = update.get("message") {
        // Voice message
        if msg.get("voice").is_some() {
            let file_id = msg["voice"]["file_id"].as_str().map(|s| s.to_string());
            return MessageIntent::VoiceMessage { file_id };
        }

        // Photo
        if msg.get("photo").is_some() {
            let caption = msg["caption"].as_str().map(|s| s.to_string());
            return MessageIntent::Media {
                media_type: MediaType::Photo,
                caption,
            };
        }

        // Document
        if msg.get("document").is_some() {
            let caption = msg["caption"].as_str().map(|s| s.to_string());
            return MessageIntent::Media {
                media_type: MediaType::Document,
                caption,
            };
        }

        // Audio
        if msg.get("audio").is_some() {
            let caption = msg["caption"].as_str().map(|s| s.to_string());
            return MessageIntent::Media {
                media_type: MediaType::Audio,
                caption,
            };
        }

        // Video
        if msg.get("video").is_some() {
            let caption = msg["caption"].as_str().map(|s| s.to_string());
            return MessageIntent::Media {
                media_type: MediaType::Video,
                caption,
            };
        }

        // Sticker
        if msg.get("sticker").is_some() {
            return MessageIntent::Media {
                media_type: MediaType::Sticker,
                caption: None,
            };
        }

        // Video note (circle video)
        if msg.get("video_note").is_some() {
            return MessageIntent::Media {
                media_type: MediaType::VideoNote,
                caption: None,
            };
        }

        // Location
        if msg.get("location").is_some() {
            return MessageIntent::Media {
                media_type: MediaType::Location,
                caption: None,
            };
        }

        // Contact
        if msg.get("contact").is_some() {
            return MessageIntent::Media {
                media_type: MediaType::Contact,
                caption: None,
            };
        }

        // Text message - check for commands
        if let Some(text) = msg["text"].as_str() {
            let text = text.trim();
            if let Some(stripped) = text.strip_prefix('/') {
                let parts: Vec<&str> = stripped.split_whitespace().collect();
                let name = parts
                    .first()
                    .map(|s| s.split('@').next().unwrap_or(s)) // remove @botname suffix
                    .unwrap_or("")
                    .to_lowercase();
                let args: Vec<String> = parts.iter().skip(1).map(|s| s.to_string()).collect();
                return MessageIntent::Command { name, args };
            }

            return MessageIntent::Chat {
                text: text.to_string(),
            };
        }
    }

    // Fallback
    MessageIntent::Chat {
        text: String::new(),
    }
}

// ============================================================================
// Inline Keyboard Builder
// ============================================================================

/// Fluent builder for Telegram inline keyboard markup
#[derive(Debug, Clone, Default)]
pub struct InlineKeyboardBuilder {
    rows: Vec<Vec<(String, String)>>,
}

impl InlineKeyboardBuilder {
    /// Create a new empty keyboard builder
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    /// Add a row of buttons: `[("label", "callback_data"), ...]`
    pub fn row(mut self, buttons: Vec<(&str, &str)>) -> Self {
        let row: Vec<(String, String)> = buttons
            .into_iter()
            .map(|(text, data)| (text.to_string(), data.to_string()))
            .collect();
        self.rows.push(row);
        self
    }

    /// Add a single button as its own row
    pub fn button(self, text: &str, callback_data: &str) -> Self {
        self.row(vec![(text, callback_data)])
    }

    /// Add approve/reject buttons (common pattern)
    pub fn approve_reject(self, id: &str) -> Self {
        self.row(vec![
            ("\u{2705} Approve", &format!("approve:{}", id)),
            ("\u{274c} Reject", &format!("reject:{}", id)),
        ])
    }

    /// Add approve/auto/reject/read buttons (full approval layout)
    pub fn approval_full(self, id: &str) -> Self {
        self.row(vec![
            ("\u{2705} Approve", &format!("approve:{}", id)),
            ("\u{1f916} Auto", &format!("auto:{}", id)),
        ])
        .row(vec![
            ("\u{274c} Reject", &format!("reject:{}", id)),
            ("\u{1f4d6} Read", &format!("read:{}", id)),
        ])
    }

    /// Add yes/no buttons
    pub fn yes_no(self, yes_data: &str, no_data: &str) -> Self {
        self.row(vec![("Yes", yes_data), ("No", no_data)])
    }

    /// Add confirm/cancel buttons
    pub fn confirm_cancel(self) -> Self {
        self.row(vec![("Confirm", "confirm"), ("Cancel", "cancel")])
    }

    /// Build into a JSON value for the Telegram API
    pub fn build(&self) -> serde_json::Value {
        let keyboard: Vec<Vec<serde_json::Value>> = self
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|(text, data)| {
                        serde_json::json!({
                            "text": text,
                            "callback_data": data
                        })
                    })
                    .collect()
            })
            .collect();

        serde_json::json!({ "inline_keyboard": keyboard })
    }

    /// Check if the keyboard has any buttons
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Get the number of rows
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

// ============================================================================
// Session Router (Group/DM routing with thread awareness)
// ============================================================================

/// Session key for group/DM routing
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RelaySessionKey {
    /// Chat ID (positive for DMs, negative for groups)
    pub chat_id: i64,
    /// User ID (present for DMs, absent for group-level routing)
    pub user_id: Option<i64>,
}

impl RelaySessionKey {
    /// Create a DM session key (chat_id:user_id)
    pub fn dm(chat_id: i64, user_id: i64) -> Self {
        Self {
            chat_id,
            user_id: Some(user_id),
        }
    }

    /// Create a group session key (chat_id only)
    pub fn group(chat_id: i64) -> Self {
        Self {
            chat_id,
            user_id: None,
        }
    }

    /// Determine if this is a group chat (negative chat_id)
    pub fn is_group(&self) -> bool {
        self.chat_id < 0
    }

    /// Create from a chat_id, auto-detecting DM vs group
    pub fn auto(chat_id: i64, user_id: i64) -> Self {
        if chat_id < 0 {
            // Group chat - route by group
            Self::group(chat_id)
        } else {
            // DM - route by user
            Self::dm(chat_id, user_id)
        }
    }

    /// String representation for HashMap keys
    pub fn to_key_string(&self) -> String {
        if let Some(uid) = self.user_id {
            format!("{}:{}", self.chat_id, uid)
        } else {
            self.chat_id.to_string()
        }
    }
}

impl std::fmt::Display for RelaySessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(uid) = self.user_id {
            write!(f, "dm:{}:{}", self.chat_id, uid)
        } else {
            write!(f, "group:{}", self.chat_id)
        }
    }
}

/// Per-session state tracked by the router
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    /// Router key string
    pub key: String,
    /// Chat ID
    pub chat_id: i64,
    /// User ID (if DM)
    pub user_id: Option<i64>,
    /// Whether this is a group chat
    pub is_group: bool,
    /// Associated Zeus session ID
    pub zeus_session_id: Option<String>,
    /// Last activity timestamp
    pub last_activity: DateTime<Utc>,
    /// Total message count
    pub message_count: u64,
    /// Last message ID (for reply threading)
    pub last_message_id: Option<i64>,
    /// Last bot response message ID (for thread awareness)
    pub last_bot_message_id: Option<i64>,
}

/// Routes Telegram chats to Zeus sessions with group/DM awareness
pub struct SessionRouter {
    sessions: RwLock<HashMap<String, SessionState>>,
    /// Default Zeus session ID for new chats
    default_session: RwLock<Option<String>>,
}

impl SessionRouter {
    /// Create a new session router
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            default_session: RwLock::new(None),
        }
    }

    /// Set the default Zeus session for new chats
    pub async fn set_default_session(&self, session_id: Option<String>) {
        *self.default_session.write().await = session_id;
    }

    /// Get or create session state for a chat
    pub async fn get_or_create(&self, key: &RelaySessionKey) -> SessionState {
        let key_str = key.to_key_string();
        let mut sessions = self.sessions.write().await;

        if let Some(state) = sessions.get_mut(&key_str) {
            state.last_activity = Utc::now();
            state.message_count += 1;
            return state.clone();
        }

        let default = self.default_session.read().await.clone();

        let state = SessionState {
            key: key_str.clone(),
            chat_id: key.chat_id,
            user_id: key.user_id,
            is_group: key.is_group(),
            zeus_session_id: default,
            last_activity: Utc::now(),
            message_count: 1,
            last_message_id: None,
            last_bot_message_id: None,
        };

        sessions.insert(key_str, state.clone());
        state
    }

    /// Update the Zeus session ID for a chat
    pub async fn set_zeus_session(&self, key: &RelaySessionKey, session_id: &str) {
        let key_str = key.to_key_string();
        let mut sessions = self.sessions.write().await;
        if let Some(state) = sessions.get_mut(&key_str) {
            state.zeus_session_id = Some(session_id.to_string());
        }
    }

    /// Record a bot message ID for reply threading
    pub async fn set_bot_message_id(&self, key: &RelaySessionKey, message_id: i64) {
        let key_str = key.to_key_string();
        let mut sessions = self.sessions.write().await;
        if let Some(state) = sessions.get_mut(&key_str) {
            state.last_bot_message_id = Some(message_id);
        }
    }

    /// Record an incoming message ID
    pub async fn set_last_message_id(&self, key: &RelaySessionKey, message_id: i64) {
        let key_str = key.to_key_string();
        let mut sessions = self.sessions.write().await;
        if let Some(state) = sessions.get_mut(&key_str) {
            state.last_message_id = Some(message_id);
        }
    }

    /// List all active sessions
    pub async fn list_sessions(&self) -> Vec<SessionState> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
    }

    /// Get session count
    pub async fn session_count(&self) -> usize {
        self.sessions.read().await.len()
    }

    /// Remove inactive sessions older than the given duration
    pub async fn cleanup_inactive(&self, max_age: std::time::Duration) -> usize {
        let cutoff = Utc::now() - chrono::Duration::from_std(max_age).unwrap_or_default();
        let mut sessions = self.sessions.write().await;
        let before = sessions.len();
        sessions.retain(|_, state| state.last_activity > cutoff);
        before - sessions.len()
    }
}

impl Default for SessionRouter {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Incoming Message + Command Types (existing, enhanced)
// ============================================================================

/// Incoming message from Telegram
#[derive(Debug, Clone)]
pub struct TelegramIncoming {
    pub chat_id: i64,
    pub user_id: i64,
    pub username: Option<String>,
    pub first_name: Option<String>,
    pub text: String,
    pub timestamp: i64,
    pub message_id: i64,
    pub is_callback: bool,
    pub callback_id: Option<String>,
    /// Reply-to message ID (thread awareness)
    pub reply_to_message_id: Option<i64>,
    /// Detected intent
    pub intent: Option<MessageIntent>,
    /// Is this from a group chat?
    pub is_group: bool,
}

/// Command parsed from Telegram message
#[derive(Debug, Clone)]
pub enum RelayCommand {
    /// /status - Show all sessions
    Status,
    /// /help - Show help
    Help,
    /// /session <id> - Switch to a different session
    SwitchSession(String),
    /// Regular message to forward to session
    Message(String),
    /// Interactive button callback
    Callback { action: String, data: String },
}

// ============================================================================
// Smart Telegram Relay
// ============================================================================

/// Type alias for message forwarding callback
type MessageCallback =
    Arc<RwLock<Option<Box<dyn Fn(String) -> tokio::task::JoinHandle<String> + Send + Sync>>>>;

/// Smart Telegram Relay with bidirectional messaging, intent detection,
/// inline keyboards, and group/DM routing.
pub struct TelegramRelay {
    config: TelegramRelayConfig,
    client: reqwest::Client,
    messages: Arc<Mutex<VecDeque<TelegramIncoming>>>,
    running: Arc<AtomicBool>,
    last_update_id: Arc<AtomicI64>,
    last_poll: Arc<Mutex<Option<DateTime<Utc>>>>,
    poll_handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    allowed_users: Vec<String>,
    max_queue: usize,
    target_session: Arc<RwLock<Option<String>>>,
    message_callback: MessageCallback,
    /// Session router for group/DM routing
    router: Arc<SessionRouter>,
    /// Bot username (auto-detected on first poll)
    bot_username: Arc<RwLock<Option<String>>>,
    /// Bot user ID (auto-detected on first poll via getMe)
    bot_user_id: Arc<RwLock<Option<i64>>>,
    /// Bot message filter mode (OpenClaw parity)
    allow_bots: AllowBotsMode,
    /// Fleet bot allowlist — Telegram user IDs that bypass the Layer 2 mention
    /// filter. Lets fleet titans coordinate via reply-chains. Parsed from
    /// `config.fleet_bot_ids` (Vec<String>) into Vec<i64> for fast `from.id`
    /// comparison. Empty = no allowlist (standard `allow_bots` behavior).
    fleet_bot_ids: Vec<i64>,
    /// Rate limiter: sender_key -> timestamps
    rate_limits: Arc<Mutex<HashMap<String, Vec<DateTime<Utc>>>>>,
    /// Voice handler for STT/TTS (auto-initialized from env vars)
    voice_handler: Option<Arc<crate::telegram_voice::TelegramVoiceHandler>>,
}

impl TelegramRelay {
    /// Create a new relay from config
    pub fn new(config: TelegramRelayConfig) -> Self {
        let allowed_users = config
            .allowed_users
            .clone()
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();

        // Target session: only when explicitly configured in config.toml
        // Never auto-detect tmux — relay is session-agnostic
        let target_session = config.target_session.clone();

        if let Some(ref session) = target_session {
            info!("Telegram relay targeting tmux session: {}", session);
        }

        let bot_username = config.bot_username.clone();

        // Initialize voice handler from providers resolved at the gateway boundary.
        // No env var reads — providers come from `zeus_core::Config` via `TelegramRelayConfig`.
        let voice_handler = match (config.stt_provider.clone(), config.tts_provider.clone()) {
            (Some(stt), tts) => {
                let handler = crate::telegram_voice::TelegramVoiceHandler::new(
                    &config.bot_token,
                    stt,
                    tts,
                    crate::telegram_voice::TelegramVoiceConfig::default(),
                );
                info!("Telegram voice handler initialized (STT enabled)");
                Some(Arc::new(handler))
            }
            (None, _) => {
                debug!(
                    "Telegram voice handler not available: no STT provider in config.toml (voice messages will be skipped)"
                );
                None
            }
        };

        let allow_bots = AllowBotsMode::from_config(config.allow_bots.as_deref());

        // Parse fleet_bot_ids from String to i64 — config takes String for TOML
        // ergonomics (large IDs survive both string and integer literals), but
        // runtime compares against `msg["from"]["id"].as_i64()` for speed.
        // Invalid entries are logged and dropped; doesn't fail the relay startup.
        let fleet_bot_ids: Vec<i64> = config
            .fleet_bot_ids
            .iter()
            .filter_map(|s| match s.trim().parse::<i64>() {
                Ok(id) => Some(id),
                Err(e) => {
                    warn!(
                        "Telegram fleet_bot_ids: skipping invalid ID '{}': {}",
                        s, e
                    );
                    None
                }
            })
            .collect();
        if !fleet_bot_ids.is_empty() {
            info!(
                count = fleet_bot_ids.len(),
                "Telegram fleet_bot_ids allowlist active — those bots bypass Layer 2 mention filter"
            );
        }

        Self {
            config,
            client: reqwest::Client::new(),
            messages: Arc::new(Mutex::new(VecDeque::new())),
            running: Arc::new(AtomicBool::new(false)),
            last_update_id: Arc::new(AtomicI64::new(0)),
            last_poll: Arc::new(Mutex::new(None)),
            poll_handle: tokio::sync::Mutex::new(None),
            allowed_users,
            max_queue: 100,
            target_session: Arc::new(RwLock::new(target_session)),
            message_callback: Arc::new(RwLock::new(None)),
            router: Arc::new(SessionRouter::new()),
            bot_username: Arc::new(RwLock::new(bot_username)),
            bot_user_id: Arc::new(RwLock::new(None)),
            allow_bots,
            fleet_bot_ids,
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
            voice_handler,
        }
    }

    /// Get the session router
    pub fn router(&self) -> &Arc<SessionRouter> {
        &self.router
    }

    /// Get the API base URL
    fn api_url(&self) -> String {
        format!("https://api.telegram.org/bot{}", self.config.bot_token)
    }

    // ========================================================================
    // Sending messages
    // ========================================================================

    /// Send a text message to the configured chat
    pub async fn send_message(&self, text: &str) -> Result<()> {
        self.send_message_to(&self.config.chat_id, text).await
    }

    /// Send a message to a specific chat, auto-chunking if needed
    pub async fn send_message_to(&self, chat_id: &str, text: &str) -> Result<()> {
        let chunks = split_long_message(text, self.config.max_message_length);

        for chunk in &chunks {
            self.send_single_message(chat_id, chunk, None).await?;
        }

        Ok(())
    }

    /// Send a single message (no chunking) with optional reply_to
    async fn send_single_message(
        &self,
        chat_id: &str,
        text: &str,
        reply_to: Option<i64>,
    ) -> Result<Option<i64>> {
        let url = format!("{}/sendMessage", self.api_url());
        let text = crate::sanitize::strip_markdown(text);
        let text = text.as_str();

        let mut payload = serde_json::json!({
            "chat_id": chat_id,
            "text": text
        });

        if let Some(reply_to) = reply_to {
            payload["reply_parameters"] = serde_json::json!({
                "message_id": reply_to
            });
        }

        let response = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                zeus_core::Error::channel(format!("Failed to send Telegram message: {}", e))
            })?;

        let body: serde_json::Value = response.json().await.unwrap_or_default();

        if body["ok"].as_bool() != Some(true) {
            // Retry without Markdown on parse error
            debug!("Markdown parse failed, retrying as plain text");
            let mut fallback = serde_json::json!({
                "chat_id": chat_id,
                "text": text
            });
            if let Some(reply_to) = reply_to {
                fallback["reply_parameters"] = serde_json::json!({
                    "message_id": reply_to
                });
            }

            let response = self
                .client
                .post(&url)
                .json(&fallback)
                .send()
                .await
                .map_err(|e| {
                    zeus_core::Error::channel(format!("Failed to send message (fallback): {}", e))
                })?;

            let body: serde_json::Value = response.json().await.unwrap_or_default();
            return Ok(body["result"]["message_id"].as_i64());
        }

        Ok(body["result"]["message_id"].as_i64())
    }

    /// Send a message as a reply to a specific message (thread awareness)
    pub async fn send_reply(
        &self,
        chat_id: &str,
        text: &str,
        reply_to_message_id: i64,
    ) -> Result<Option<i64>> {
        let chunks = split_long_message(text, self.config.max_message_length);
        let mut last_msg_id = None;

        for (i, chunk) in chunks.iter().enumerate() {
            // Only reply_to on the first chunk
            let reply_to = if i == 0 {
                Some(reply_to_message_id)
            } else {
                None
            };
            last_msg_id = self.send_single_message(chat_id, chunk, reply_to).await?;
        }

        Ok(last_msg_id)
    }

    /// Send a message with inline keyboard buttons
    pub async fn send_message_with_buttons(
        &self,
        text: &str,
        buttons: Vec<Vec<(&str, &str)>>,
    ) -> Result<()> {
        let keyboard = InlineKeyboardBuilder::new();
        let mut kb = keyboard;
        for row in buttons {
            kb = kb.row(row);
        }
        self.send_with_keyboard(&self.config.chat_id, text, &kb, None)
            .await
    }

    /// Send a message with an InlineKeyboardBuilder
    pub async fn send_with_keyboard(
        &self,
        chat_id: &str,
        text: &str,
        keyboard: &InlineKeyboardBuilder,
        reply_to: Option<i64>,
    ) -> Result<()> {
        let url = format!("{}/sendMessage", self.api_url());
        let text = crate::sanitize::strip_markdown(text);

        let mut payload = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "reply_markup": keyboard.build()
        });

        if let Some(reply_to) = reply_to {
            payload["reply_parameters"] = serde_json::json!({
                "message_id": reply_to
            });
        }

        self.client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                zeus_core::Error::channel(format!("Failed to send message with buttons: {}", e))
            })?;

        Ok(())
    }

    /// Edit a previously sent message
    pub async fn edit_message(&self, chat_id: &str, message_id: i64, new_text: &str) -> Result<()> {
        let url = format!("{}/editMessageText", self.api_url());
        let new_text = crate::sanitize::strip_markdown(new_text);

        let response = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "message_id": message_id,
                "text": new_text
            }))
            .send()
            .await
            .map_err(|e| zeus_core::Error::channel(format!("Failed to edit message: {}", e)))?;

        if !response.status().is_success() {
            // Retry without Markdown
            self.client
                .post(&url)
                .json(&serde_json::json!({
                    "chat_id": chat_id,
                    "message_id": message_id,
                    "text": new_text
                }))
                .send()
                .await
                .map_err(|e| {
                    zeus_core::Error::channel(format!("Failed to edit message (fallback): {}", e))
                })?;
        }

        Ok(())
    }

    /// Send a typing indicator to a chat
    pub async fn send_typing(&self, chat_id: &str) -> Result<()> {
        let url = format!("{}/sendChatAction", self.api_url());

        self.client
            .post(&url)
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "action": "typing"
            }))
            .send()
            .await
            .map_err(|e| {
                zeus_core::Error::channel(format!("Failed to send typing indicator: {}", e))
            })?;

        Ok(())
    }

    /// Send a photo to a chat
    pub async fn send_photo(
        &self,
        chat_id: &str,
        photo_url: &str,
        caption: Option<&str>,
    ) -> Result<()> {
        let url = format!("{}/sendPhoto", self.api_url());

        let mut payload = serde_json::json!({
            "chat_id": chat_id,
            "photo": photo_url
        });

        if let Some(caption) = caption {
            payload["caption"] = serde_json::Value::String(caption.to_string());
        }

        self.client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| zeus_core::Error::channel(format!("Failed to send photo: {}", e)))?;

        Ok(())
    }

    /// Send a document to a chat
    pub async fn send_document(
        &self,
        chat_id: &str,
        document_url: &str,
        caption: Option<&str>,
    ) -> Result<()> {
        let url = format!("{}/sendDocument", self.api_url());

        let mut payload = serde_json::json!({
            "chat_id": chat_id,
            "document": document_url
        });

        if let Some(caption) = caption {
            payload["caption"] = serde_json::Value::String(caption.to_string());
        }

        self.client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| zeus_core::Error::channel(format!("Failed to send document: {}", e)))?;

        Ok(())
    }

    /// Answer a callback query
    pub async fn answer_callback(&self, callback_id: &str, text: Option<&str>) -> Result<()> {
        let url = format!("{}/answerCallbackQuery", self.api_url());

        let mut params = serde_json::json!({
            "callback_query_id": callback_id
        });

        if let Some(text) = text {
            params["text"] = serde_json::Value::String(text.to_string());
        }

        self.client
            .post(&url)
            .json(&params)
            .send()
            .await
            .map_err(|e| zeus_core::Error::channel(format!("Failed to answer callback: {}", e)))?;

        Ok(())
    }

    // ========================================================================
    // Message callback + lifecycle
    // ========================================================================

    /// Set the message callback for forwarding to Zeus agent
    pub async fn set_message_callback<F>(&self, callback: F)
    where
        F: Fn(String) -> tokio::task::JoinHandle<String> + Send + Sync + 'static,
    {
        let mut cb = self.message_callback.write().await;
        *cb = Some(Box::new(callback));
    }

    /// Start the background polling loop
    pub async fn start(&self) -> Result<()> {
        if self.is_running() {
            return Err(zeus_core::Error::channel(
                "Relay is already running".to_string(),
            ));
        }

        self.running.store(true, Ordering::SeqCst);

        // Always fetch bot identity at startup. fetch_bot_username caches BOTH
        // bot_username AND bot_user_id — and bot_user_id underpins Layer 1
        // self-echo. Previously this was gated on bot_username.is_none(), so
        // if config pre-set bot_username, bot_user_id silently stayed None and
        // Layer 1 silently disabled. Errors are now logged instead of swallowed.
        if self.bot_user_id.read().await.is_none() {
            match self.fetch_bot_username().await {
                Ok(username) => {
                    info!("Auto-detected bot identity: @{} (user_id cached)", username);
                    if self.bot_username.read().await.is_none() {
                        *self.bot_username.write().await = Some(username);
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to fetch bot identity at startup — Layer 1 self-echo will not fire: {}",
                        e
                    );
                }
            }
        }

        let running = self.running.clone();
        let messages = self.messages.clone();
        let last_update_id = self.last_update_id.clone();
        let last_poll = self.last_poll.clone();
        let client = self.client.clone();
        let tg_api = self.api_url();
        let max_queue = self.max_queue;
        let allowed_users = self.allowed_users.clone();
        let message_callback = self.message_callback.clone();
        let _default_chat_id = self.config.chat_id.clone();
        let router = self.router.clone();
        let bot_username = self.bot_username.clone();
        let bot_user_id = self.bot_user_id.clone();
        let allow_bots = self.allow_bots;
        let fleet_bot_ids = self.fleet_bot_ids.clone();
        // Build group policy from config:
        // 1. Explicit `policy` in config takes priority
        // 2. `require_mention_in_groups` sets MentionOnly
        // 3. Default: Open (bots see all group messages)
        let policy_config =
            self.config
                .policy
                .clone()
                .unwrap_or_else(|| zeus_core::ChannelPolicyConfig {
                    group: if self.config.require_mention_in_groups {
                        zeus_core::GroupPolicy::MentionOnly
                    } else {
                        zeus_core::GroupPolicy::Open
                    },
                    ..Default::default()
                });
        let enable_groups = self.config.enable_groups;
        let rate_limits = self.rate_limits.clone();
        let rate_limit_per_minute = self.config.rate_limit_per_minute;
        let voice_handler = self.voice_handler.clone();
        let target_session = self.target_session.clone();

        // ── Webhook mode ──────────────────────────────────────────────────
        if let Some(port) = self.config.webhook_port {
            // Register with Telegram if a public URL is configured.
            if self.config.webhook_url.is_some()
                && let Err(e) = self.register_webhook().await
            {
                warn!("Failed to register Telegram webhook: {e}");
            }
            let handle = Self::start_webhook_listener(
                port,
                self.config.webhook_path.clone(),
                messages,
                self.max_queue,
                self.message_callback.clone(),
            )
            .await?;
            *self.poll_handle.lock().await = Some(handle);
            return Ok(());
        }

        // ── Long-polling mode (default) ───────────────────────────────────
        let handle = tokio::spawn(async move {
            info!("Telegram relay started polling");
            // LRU dedup: keep last N update_ids to prevent duplicate processing
            // while bounding memory. VecDeque for eviction order + HashSet for O(1) lookup.
            const SEEN_MAX: usize = 500;
            let mut seen_updates: HashSet<i64> = HashSet::new();
            let mut seen_order: VecDeque<i64> = VecDeque::new();
            // ── Reconnect/backoff state (V1 hardening, Stream B.3) ────────────
            // Tracks consecutive poll/API failures to drive exponential backoff
            // on transient outages, rate-limits (429), and auth errors (401).
            // Reset to 0 on any successful `body["ok"] == true` poll.
            let mut consecutive_errors: u32 = 0;
            // Backoff schedule: 5s → 10s → 30s → 60s cap (saturates).
            fn compute_backoff_secs(n: u32) -> u64 {
                match n {
                    0 | 1 => 5,
                    2 => 10,
                    3 => 30,
                    _ => 60,
                }
            }

            while running.load(Ordering::SeqCst) {
                let offset = last_update_id.load(Ordering::SeqCst) + 1;
                let url = format!("{}/getUpdates", tg_api);

                let poll_result = client
                    .post(&url)
                    .json(&serde_json::json!({
                        "offset": offset,
                        "timeout": 2,
                        "allowed_updates": ["message", "callback_query"]
                    }))
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await;

                match poll_result {
                    Ok(response) => {
                        // Parse body once; branch on Telegram API `ok` flag.
                        // Stream B.3: explicit !ok handling for 429/401/other,
                        // consecutive-error counter + exponential backoff.
                        let body = match response.json::<serde_json::Value>().await {
                            Ok(b) => b,
                            Err(e) => {
                                consecutive_errors = consecutive_errors.saturating_add(1);
                                let backoff = compute_backoff_secs(consecutive_errors);
                                warn!(
                                    "Telegram poll body-parse error (#{}): {} — backing off {}s",
                                    consecutive_errors, e, backoff
                                );
                                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                                continue;
                            }
                        };
                        if body["ok"].as_bool() != Some(true) {
                            // Telegram API error response: parse error_code, special-case
                            // 429 (rate-limit, honor retry_after) and 401 (auth, fatal).
                            let error_code = body["error_code"].as_i64().unwrap_or(0);
                            let description =
                                body["description"].as_str().unwrap_or("(no description)");
                            consecutive_errors = consecutive_errors.saturating_add(1);
                            let backoff = match error_code {
                                429 => {
                                    let retry_after = body["parameters"]["retry_after"]
                                        .as_u64()
                                        .unwrap_or_else(|| {
                                            compute_backoff_secs(consecutive_errors)
                                        });
                                    warn!(
                                        "Telegram API 429 rate-limit (#{}): {} — honoring retry_after {}s",
                                        consecutive_errors, description, retry_after
                                    );
                                    retry_after
                                }
                                401 => {
                                    error!(
                                        "Telegram API 401 auth failure (#{}): {} — extended backoff 60s, check bot token",
                                        consecutive_errors, description
                                    );
                                    60
                                }
                                _ => {
                                    let b = compute_backoff_secs(consecutive_errors);
                                    warn!(
                                        "Telegram API error {} (#{}): {} — backing off {}s",
                                        error_code, consecutive_errors, description, b
                                    );
                                    b
                                }
                            };
                            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                            continue;
                        }
                        // Happy path: ok==true — reset error counter, process updates.
                        consecutive_errors = 0;
                        if let Some(updates) = body["result"].as_array() {
                            if !updates.is_empty() {
                                info!("Telegram relay received {} update(s)", updates.len());
                            }
                            for update in updates {
                                let uid = update["update_id"].as_i64().unwrap_or(0);
                                last_update_id.store(uid, Ordering::SeqCst);

                                // Deduplicate (LRU-keep-last-N)
                                if !seen_updates.insert(uid) {
                                    debug!("Skipping duplicate update_id: {}", uid);
                                    continue;
                                }
                                seen_order.push_back(uid);
                                if seen_order.len() > SEEN_MAX {
                                    if let Some(oldest) = seen_order.pop_front() {
                                        seen_updates.remove(&oldest);
                                    }
                                }

                                // Update last poll time
                                *last_poll.lock().expect("lock poisoned") = Some(Utc::now());

                                // Detect message intent
                                let intent = detect_intent(update);

                                // Handle message
                                if let Some(msg) = update.get("message") {
                                    // Extract text early — needed by bot filter Layer 2
                                    let text = msg["text"]
                                        .as_str()
                                        .or_else(|| msg["caption"].as_str())
                                        .unwrap_or("")
                                        .to_string();

                                    // ── Layer 1: Self-echo (always block our own messages) ──
                                    if let Some(our_id) = *bot_user_id.read().await
                                        && msg["from"]["id"].as_i64() == Some(our_id)
                                    {
                                        debug!("Skipping self-echo message");
                                        continue;
                                    }

                                    // ── Layer 2: Bot message filter (OpenClaw allowBots parity) ──
                                    // Telegram has no @everyone/@here equivalent from bots.
                                    if msg["from"]["is_bot"].as_bool() == Some(true) {
                                        // Fleet allowlist bypass — fleet titans coordinate via reply-chains
                                        // and bypass Layer 2 mention requirements. Configured via
                                        // [channels.telegram] fleet_bot_ids = ["123", ...] in config.toml.
                                        // External bots (not in list) still gated by allow_bots mode below.
                                        let in_fleet = msg["from"]["id"]
                                            .as_i64()
                                            .map(|id| fleet_bot_ids.contains(&id))
                                            .unwrap_or(false);
                                        if in_fleet {
                                            debug!(
                                                from_id = ?msg["from"]["id"].as_i64(),
                                                "Bypassing Layer 2 mention filter — sender in fleet_bot_ids allowlist"
                                            );
                                        } else {
                                        match allow_bots {
                                            AllowBotsMode::Off => {
                                                debug!("Skipping bot message (allow_bots = off)");
                                                continue;
                                            }
                                            AllowBotsMode::Mentions => {
                                                let bot_name = bot_username.read().await;
                                                let bot_name_lower = bot_name.as_ref().map(|s| s.to_lowercase());

                                                // Bot-author + reply-to-self gate (guard-completion mirror).
                                                //
                                                // Within this block, sender is_bot==true by construction (gated at :1275).
                                                // `is_reply_to_self` true when the message is a reply to one of OUR
                                                // prior messages — Telegram clients auto-prefix reply text with
                                                // `@bot_username`, which previously fired Check 1 (structured entity)
                                                // and Check 3 (text substring), bypassing Check 2's existing guard
                                                // and producing bot-to-bot tag-loops.
                                                //
                                                // Per zeus106 Round-4 substrate decomposition + three-seat convergence
                                                // (zeus106 + Z112 + zeus-spark), extending the bot-author suppression
                                                // semantic from sole-Check-2 coverage to all-three-checks coverage.
                                                let is_reply_to_self = msg["reply_to_message"]["from"]["id"].as_i64()
                                                    == *bot_user_id.read().await;

                                                // Check 1: structured entity mention containing our username
                                                let is_mentioned = if let Some(entities) =
                                                    msg.get("entities").and_then(|e| e.as_array())
                                                {
                                                    entities.iter().any(|ent| {
                                                        ent.get("type").and_then(|t| t.as_str()) == Some("mention")
                                                            && bot_name_lower.as_ref().is_some_and(|name| {
                                                                let offset = ent.get("offset").and_then(|o| o.as_u64()).unwrap_or(0) as usize;
                                                                let length = ent.get("length").and_then(|l| l.as_u64()).unwrap_or(0) as usize;
                                                                let mention_text = text.chars().skip(offset).take(length).collect::<String>();
                                                                mention_text.to_lowercase() == format!("@{}", name)
                                                            })
                                                    })
                                                } else {
                                                    false
                                                };

                                                // Check 2: reply-chain implicit mention — RE-ENABLED 2026-05-06.
                                                //
                                                // Patch A in caea66a7 (today, ~13h ago) hard-disabled this to prevent
                                                // bot-bot infinite loops (rekpatdcoord/qtumdev "silent silent" thread).
                                                // That fix turned out to be load-bearing in the wrong direction:
                                                // fleet titans coordinating via Telegram reply-chains depend on this
                                                // path to receive each other's messages. Disabling it dropped all
                                                // bot→bot reply-chain coordination → halted fleet productivity on
                                                // Telegram for hours.
                                                //
                                                // Reverting Patch A per merakizzz's directive 2026-05-06. The original
                                                // bot-loop concern is being addressed via the architectural fix
                                                // (extract `prepare_cook_context` from `process_autonomous`, fleet
                                                // session aliasing) which makes the loops impossible at a higher
                                                // layer (no shared cook context = no degenerate echo).
                                                let implicit_mention = msg["reply_to_message"]["from"]["id"].as_i64()
                                                    == *bot_user_id.read().await;

                                                // Check 1.5: structured `text_mention` entity with embedded user.id.
                                                // Telegram clients convert @-mentions of users-without-public-username
                                                // into `text_mention` entities carrying a `user` object with numeric id,
                                                // rather than emitting a plain `mention` entity (which requires public
                                                // @username). Discord-side equivalent is `<@BOT_ID>` numeric-id text.
                                                // Check 1 alone misses this shape entirely → bot-author messages tagging
                                                // us via text_mention would fall through both Check 1 (wrong entity type)
                                                // and Check 3 (no `@username` substring present in text).
                                                let text_mention_entity = if let Some(entities) =
                                                    msg.get("entities").and_then(|e| e.as_array())
                                                {
                                                    if let Some(our_id) = *bot_user_id.read().await {
                                                        entities.iter().any(|ent| {
                                                            ent.get("type").and_then(|t| t.as_str())
                                                                == Some("text_mention")
                                                                && ent
                                                                    .get("user")
                                                                    .and_then(|u| u.get("id"))
                                                                    .and_then(|i| i.as_i64())
                                                                    == Some(our_id)
                                                        })
                                                    } else {
                                                        false
                                                    }
                                                } else {
                                                    false
                                                };

                                                // Check 3: case-insensitive text @<bot_username>
                                                let text_mention = bot_name_lower.as_ref().is_some_and(|name| {
                                                    text.to_lowercase().contains(&format!("@{}", name))
                                                });

                                                // Suppress Check 1 (structured entity) and Check 3 (text substring)
                                                // when this is a bot-author replying to one of our own messages —
                                                // closes the multi-check-OR-guard-coverage gap that allowed
                                                // titan-tag-loops to fire through Telegram's auto-@-prefix on replies.
                                                // Check 2 already-guarded (`implicit_mention` keys on bot_user_id).
                                                let is_mentioned_effective = is_mentioned && !is_reply_to_self;
                                                let text_mention_entity_effective =
                                                    text_mention_entity && !is_reply_to_self;
                                                let text_mention_effective = text_mention && !is_reply_to_self;

                                                if !is_mentioned_effective
                                                    && !text_mention_entity_effective
                                                    && !implicit_mention
                                                    && !text_mention_effective
                                                {
                                                    debug!(
                                                        is_reply_to_self = is_reply_to_self,
                                                        "Skipping bot message (allow_bots = mentions, no mention found)"
                                                    );
                                                    continue;
                                                }
                                            }
                                            AllowBotsMode::On => {} // allow all bot messages through
                                        }
                                        } // close `else` (in_fleet bypass)
                                    }

                                    let chat_id_val = msg["chat"]["id"].as_i64().unwrap_or(0);
                                    let chat_type =
                                        msg["chat"]["type"].as_str().unwrap_or("private");
                                    let is_group =
                                        chat_type == "group" || chat_type == "supergroup";
                                    let user_id = msg["from"]["id"].as_i64().unwrap_or(0);
                                    let msg_id = msg["message_id"].as_i64().unwrap_or(0);
                                    let username =
                                        msg["from"]["username"].as_str().map(|s| s.to_string());
                                    let first_name = msg["from"]["first_name"]
                                        .as_str()
                                        .unwrap_or("User")
                                        .to_string();
                                    let timestamp = msg["date"].as_i64().unwrap_or(0);
                                    let reply_to_msg_id =
                                        msg["reply_to_message"]["message_id"].as_i64();

                                    // Skip group messages if groups disabled
                                    if is_group && !enable_groups {
                                        continue;
                                    }

                                    // Policy-based group filtering (MentionOnly / Allowlist)
                                    if is_group {
                                        match policy_config.group {
                                            zeus_core::GroupPolicy::MentionOnly => {
                                                // Check if bot is @mentioned in message entities
                                                let is_mention = if let Some(entities) =
                                                    msg.get("entities").and_then(|e| e.as_array())
                                                {
                                                    let bot_name = bot_username.read().await;
                                                    entities.iter().any(|ent| {
                                                        ent.get("type").and_then(|t| t.as_str())
                                                            == Some("mention")
                                                            && bot_name.as_ref().is_some_and(
                                                                |name| {
                                                                    text.to_lowercase().contains(
                                                                        &format!(
                                                                            "@{}",
                                                                            name.to_lowercase()
                                                                        ),
                                                                    )
                                                                },
                                                            )
                                                    })
                                                } else {
                                                    false
                                                };
                                                if !is_mention {
                                                    debug!(
                                                        "Skipping group message (MentionOnly policy, no @mention)"
                                                    );
                                                    continue;
                                                }
                                            }
                                            zeus_core::GroupPolicy::Allowlist => {
                                                let chat_str = chat_id_val.to_string();
                                                if !policy_config
                                                    .allow_groups
                                                    .iter()
                                                    .any(|id| id == &chat_str)
                                                {
                                                    debug!(
                                                        "Skipping group {} (not in allowlist)",
                                                        chat_id_val
                                                    );
                                                    continue;
                                                }
                                            }
                                            zeus_core::GroupPolicy::Disabled => {
                                                debug!(
                                                    "Skipping group message (groups disabled by policy)"
                                                );
                                                continue;
                                            }
                                            zeus_core::GroupPolicy::Open => {} // all messages pass
                                        }
                                    }

                                    // ── Layer 3: ChannelPolicy (DM / group access control) ──
                                    // Bot-to-bot messages bypass L3 to preserve relay semantics
                                    // (commit 32a6b61c). Non-bot senders are subject to policy.
                                    let is_from_bot = msg["from"]["is_bot"].as_bool().unwrap_or(false);
                                    if !is_from_bot {
                                        let policy = ChannelPolicy::new(policy_config.clone());
                                        let policy_result = if is_group {
                                            // Recompute whether bot is @mentioned for L3 check
                                            let is_mention = if let Some(entities) =
                                                msg.get("entities").and_then(|e| e.as_array())
                                            {
                                                let bot_name = bot_username.read().await;
                                                entities.iter().any(|ent| {
                                                    ent.get("type").and_then(|t| t.as_str())
                                                        == Some("mention")
                                                        && bot_name.as_ref().is_some_and(|name| {
                                                            ent.get("text")
                                                                .and_then(|t| t.as_str())
                                                                .is_some_and(|text| {
                                                                    text.eq_ignore_ascii_case(&format!("@{}", name))
                                                                })
                                                        })
                                                })
                                            } else {
                                                false
                                            };
                                            policy.check_group(
                                                &chat_id_val.to_string(),
                                                &user_id.to_string(),
                                                is_mention,
                                            )
                                        } else {
                                            policy.check_dm(&user_id.to_string())
                                        };
                                        if let PolicyResult::Denied(ref reason) = policy_result {
                                            debug!("Layer 3 policy denied: {}", reason);
                                            continue;
                                        }
                                    }

                                    if text.is_empty()
                                        && !matches!(
                                            intent,
                                            MessageIntent::Media { .. }
                                                | MessageIntent::VoiceMessage { .. }
                                        )
                                    {
                                        continue;
                                    }

                                    // Security check
                                    if !allowed_users.is_empty() {
                                        let username_lower = username
                                            .as_deref()
                                            .map(|s| s.to_lowercase())
                                            .unwrap_or_default();
                                        if !allowed_users.iter().any(|u| u == &username_lower) {
                                            debug!(
                                                "Ignoring message from unauthorized user: {}",
                                                username_lower
                                            );
                                            continue;
                                        }
                                    }

                                    // Rate limiting
                                    {
                                        let rate_key = format!("{}:{}", chat_id_val, user_id);
                                        let mut limits = rate_limits.lock().expect("lock poisoned");
                                        let timestamps = limits.entry(rate_key).or_default();
                                        let cutoff = Utc::now() - chrono::Duration::seconds(60);
                                        timestamps.retain(|ts| *ts > cutoff);
                                        if timestamps.len() >= rate_limit_per_minute {
                                            debug!("Rate limited user {}", user_id);
                                            continue;
                                        }
                                        timestamps.push(Utc::now());
                                    }

                                    // Route through session router
                                    let session_key = RelaySessionKey::auto(chat_id_val, user_id);
                                    let _session_state = router.get_or_create(&session_key).await;
                                    router.set_last_message_id(&session_key, msg_id).await;

                                    // Keep all mentions intact so agents see who else was tagged
                                    let cleaned_text = text.clone();

                                    let incoming = TelegramIncoming {
                                        chat_id: chat_id_val,
                                        user_id,
                                        username: username.clone(),
                                        first_name: Some(first_name.clone()),
                                        text: cleaned_text.clone(),
                                        timestamp,
                                        message_id: msg_id,
                                        is_callback: false,
                                        callback_id: None,
                                        reply_to_message_id: reply_to_msg_id,
                                        intent: Some(intent.clone()),
                                        is_group,
                                    };

                                    debug!(
                                        "Relay received {:?} from {} in {}",
                                        intent, user_id, chat_id_val
                                    );

                                    // Queue the message
                                    {
                                        let mut queue = messages.lock().expect("lock poisoned");
                                        if queue.len() >= max_queue {
                                            queue.pop_front();
                                        }
                                        queue.push_back(incoming);
                                    }

                                    // Forward to tmux session (relay to interactive Claude Code)
                                    {
                                        let explicit_target = target_session.read().await.clone();
                                        if let Some(session) =
                                            resolve_tmux_target(&explicit_target).await
                                        {
                                            // For media messages, download the file and prefix with type label
                                            let is_media =
                                                matches!(intent, MessageIntent::Media { .. });
                                            if is_media {
                                                let dl_client = client.clone();
                                                let dl_tg_api = tg_api.clone();
                                                let dl_msg = msg.clone();
                                                let dl_first = first_name.clone();
                                                let dl_text = cleaned_text.clone();
                                                let dl_group = is_group;
                                                let dl_chat = chat_id_val;
                                                let dl_session = session.clone();
                                                tokio::spawn(async move {
                                                    if let Some((ftype, file_id, orig_name, _mime)) =
                                                        extract_media_info(&dl_msg)
                                                        && !file_id.is_empty()
                                                    {
                                                        let type_label = match ftype.as_str() {
                                                            "photo" => "Photo",
                                                            "document" => "Document",
                                                            "audio" => "Audio",
                                                            "video" => "Video",
                                                            _ => "File",
                                                        };
                                                        let cap_suffix = if dl_text.is_empty() {
                                                            String::new()
                                                        } else {
                                                            format!(" {}", dl_text)
                                                        };
                                                        match download_telegram_file(
                                                            &dl_client,
                                                            &dl_tg_api,
                                                            &file_id,
                                                            msg_id,
                                                            &ftype,
                                                            orig_name.as_deref(),
                                                        )
                                                        .await
                                                        {
                                                            Ok(path) => {
                                                                let display = if ftype == "photo" {
                                                                    format!(
                                                                        "[{}]{}",
                                                                        type_label, cap_suffix
                                                                    )
                                                                } else {
                                                                    let name = orig_name
                                                                        .as_deref()
                                                                        .unwrap_or(type_label);
                                                                    format!(
                                                                        "[{}: {}]{}",
                                                                        type_label,
                                                                        name,
                                                                        cap_suffix
                                                                    )
                                                                };
                                                                let fwd_text = if dl_group {
                                                                    format!(
                                                                        "(Telegram file from {} in {}) {} {}",
                                                                        dl_first,
                                                                        dl_chat,
                                                                        path,
                                                                        display
                                                                    )
                                                                } else {
                                                                    format!(
                                                                        "(Telegram file from {}) {} {}",
                                                                        dl_first, path, display
                                                                    )
                                                                };
                                                                forward_to_tmux(
                                                                    &dl_session,
                                                                    &fwd_text,
                                                                )
                                                                .await;
                                                            }
                                                            Err(e) => {
                                                                warn!(
                                                                    "{} download failed: {}",
                                                                    type_label, e
                                                                );
                                                                let fwd_text = if dl_group {
                                                                    format!(
                                                                        "(Telegram group {} from {}) [{} download failed: {}]{}",
                                                                        dl_chat,
                                                                        dl_first,
                                                                        type_label,
                                                                        e,
                                                                        cap_suffix
                                                                    )
                                                                } else {
                                                                    format!(
                                                                        "(Telegram from {}) [{} download failed: {}]{}",
                                                                        dl_first,
                                                                        type_label,
                                                                        e,
                                                                        cap_suffix
                                                                    )
                                                                };
                                                                forward_to_tmux(
                                                                    &dl_session,
                                                                    &fwd_text,
                                                                )
                                                                .await;
                                                            }
                                                        }
                                                    }
                                                });
                                            } else {
                                                // Text message — forward directly
                                                let fwd_text = if is_group {
                                                    format!(
                                                        "(Telegram group {} from {}) {}",
                                                        chat_id_val, first_name, cleaned_text
                                                    )
                                                } else {
                                                    format!(
                                                        "(Telegram from {}) {}",
                                                        first_name, cleaned_text
                                                    )
                                                };
                                                tokio::spawn(async move {
                                                    forward_to_tmux(&session, &fwd_text).await;
                                                });
                                            }
                                        }
                                    }

                                    // Forward to Zeus agent if callback is set
                                    // Runs independently of tmux forwarding — both can be active
                                    if let Some(ref cb) = *message_callback.read().await {
                                        // Handle voice messages: download + transcribe
                                        let (mut final_text, is_voice) =
                                            if let MessageIntent::VoiceMessage { ref file_id } =
                                                intent
                                            {
                                                if let Some(ref vh) = voice_handler {
                                                    if let Some(fid) = file_id {
                                                        let kind = crate::telegram_voice::VoiceMessageKind::Voice {
                                                        file_id: fid.clone(),
                                                        duration: 0,
                                                        mime_type: "audio/ogg".to_string(),
                                                    };
                                                        match vh.process_inbound(&kind).await {
                                                            Ok(transcript) => {
                                                                info!(
                                                                    "Voice transcribed: {} chars",
                                                                    transcript.len()
                                                                );
                                                                (
                                                                    format!(
                                                                        "[Voice message] {}",
                                                                        transcript
                                                                    ),
                                                                    true,
                                                                )
                                                            }
                                                            Err(e) => {
                                                                warn!(
                                                                    "Voice transcription failed: {}",
                                                                    e
                                                                );
                                                                (cleaned_text.clone(), false)
                                                            }
                                                        }
                                                    } else {
                                                        (cleaned_text.clone(), false)
                                                    }
                                                } else {
                                                    (cleaned_text.clone(), false)
                                                }
                                            } else {
                                                (cleaned_text.clone(), false)
                                            };

                                        // ── Media attachment forwarding to agent callback ──
                                        //
                                        // Mirror of the tmux media path above: extract media info,
                                        // download via Bot API, and inject a media descriptor +
                                        // local file path into the agent's text envelope.
                                        //
                                        // The callback signature is `Fn(String) -> ... -> String`,
                                        // so we encode the attachment as text the agent can parse:
                                        //   `[Document: foo.pdf] /tmp/zeus-tg/<msg_id>-foo.pdf <caption>`
                                        // — same shape tmux gets. The agent can then read the file
                                        // off disk to inspect/use the attachment.
                                        //
                                        // Without this, file-only messages produce empty `final_text`
                                        // and get dropped by the empty-check below — the symptom
                                        // observed before this fix ("can you see this file?" arriving
                                        // at the agent as text-only, no attachment metadata).
                                        if matches!(intent, MessageIntent::Media { .. }) {
                                            if let Some((ftype, file_id, orig_name, _mime)) =
                                                extract_media_info(msg)
                                                && !file_id.is_empty()
                                            {
                                                let type_label = match ftype.as_str() {
                                                    "photo" => "Photo",
                                                    "document" => "Document",
                                                    "audio" => "Audio",
                                                    "video" => "Video",
                                                    _ => "File",
                                                };
                                                let cap_suffix = if final_text.is_empty() {
                                                    String::new()
                                                } else {
                                                    format!(" {}", final_text)
                                                };
                                                match download_telegram_file(
                                                    &client,
                                                    &tg_api,
                                                    &file_id,
                                                    msg_id,
                                                    &ftype,
                                                    orig_name.as_deref(),
                                                )
                                                .await
                                                {
                                                    Ok(path) => {
                                                        if ftype == "photo" {
                                                            // For photos, try to inline base64
                                                            // so vision-capable models can "see".
                                                            match tokio::fs::read(&path).await {
                                                                Ok(bytes) => {
                                                                    let b64 = STANDARD.encode(&bytes);
                                                                    final_text = format!(
                                                                        "data:image/jpeg;base64,{}{}",
                                                                        b64, cap_suffix
                                                                    );
                                                                }
                                                                Err(e) => {
                                                                    warn!("Photo base64 inline failed: {}", e);
                                                                    final_text = format!(
                                                                        "[{}] {}{}",
                                                                        type_label, path, cap_suffix
                                                                    );
                                                                }
                                                            }
                                                        } else {
                                                            let name = orig_name
                                                                .as_deref()
                                                                .unwrap_or(type_label);
                                                            let display = format!(
                                                                "[{}: {}]",
                                                                type_label, name
                                                            );
                                                            final_text = format!(
                                                                "{} {}{}",
                                                                display, path, cap_suffix
                                                            );
                                                        }
                                                    }
                                                    Err(e) => {
                                                        warn!(
                                                            "{} download failed (agent path): {}",
                                                            type_label, e
                                                        );
                                                        final_text = format!(
                                                            "[{} download failed: {}]{}",
                                                            type_label, e, cap_suffix
                                                        );
                                                    }
                                                }
                                            }
                                        }

                                        // Skip if no text content after voice + media processing
                                        if final_text.is_empty() {
                                            continue;
                                        }

                                        // Forward voice transcriptions to tmux too
                                        if is_voice {
                                            let explicit_target =
                                                target_session.read().await.clone();
                                            if let Some(session) =
                                                resolve_tmux_target(&explicit_target).await
                                            {
                                                let fwd_text = if is_group {
                                                    format!(
                                                        "(Telegram group {} from {}) {}",
                                                        chat_id_val, first_name, final_text
                                                    )
                                                } else {
                                                    format!(
                                                        "(Telegram from {}) {}",
                                                        first_name, final_text
                                                    )
                                                };
                                                tokio::spawn(async move {
                                                    forward_to_tmux(&session, &fwd_text).await;
                                                });
                                            }
                                        }

                                        let formatted_msg = if is_group {
                                            format!(
                                                "(Telegram group {} from {}) {}",
                                                chat_id_val, first_name, final_text
                                            )
                                        } else {
                                            format!("(Telegram from {}) {}", first_name, final_text)
                                        };
                                        let handle = cb(formatted_msg);

                                        // Send response back to Telegram
                                        let tg_api_clone = tg_api.clone();
                                        let client_clone = client.clone();
                                        let chat_id_str = chat_id_val.to_string();
                                        let reply_to_id = msg_id;
                                        let router_clone = router.clone();
                                        let sk_clone = session_key.clone();
                                        let vh_clone = voice_handler.clone();
                                        let was_voice = is_voice;
                                        tokio::spawn(async move {
                                            match handle.await {
                                                Ok(response) => {
                                                    if !response.is_empty() {
                                                        // Try voice reply for voice messages
                                                        if was_voice && let Some(ref vh) = vh_clone
                                                        {
                                                            match vh
                                                                .process_outbound(
                                                                    &chat_id_str,
                                                                    &response,
                                                                )
                                                                .await
                                                            {
                                                                Ok(true) => {
                                                                    info!(
                                                                        "Sent voice reply to {}",
                                                                        chat_id_str
                                                                    );
                                                                    // Still send text as fallback/caption
                                                                }
                                                                Ok(false) => {} // No TTS configured, fall through to text
                                                                Err(e) => warn!(
                                                                    "Voice reply failed: {}, sending text",
                                                                    e
                                                                ),
                                                            }
                                                        }

                                                        // Always send text reply
                                                        if let Ok(Some(bot_msg_id)) =
                                                            send_response_with_reply(
                                                                &client_clone,
                                                                &tg_api_clone,
                                                                &chat_id_str,
                                                                &response,
                                                                Some(reply_to_id),
                                                            )
                                                            .await
                                                        {
                                                            router_clone
                                                                .set_bot_message_id(
                                                                    &sk_clone, bot_msg_id,
                                                                )
                                                                .await;
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    error!("Agent response error: {}", e);
                                                }
                                            }
                                        });
                                    }
                                }

                                // Handle callback query
                                if let Some(callback) = update.get("callback_query") {
                                    let callback_id =
                                        callback["id"].as_str().unwrap_or("").to_string();
                                    let chat_id_val =
                                        callback["message"]["chat"]["id"].as_i64().unwrap_or(0);
                                    let user_id = callback["from"]["id"].as_i64().unwrap_or(0);
                                    let username = callback["from"]["username"]
                                        .as_str()
                                        .map(|s| s.to_string());
                                    let first_name = callback["from"]["first_name"]
                                        .as_str()
                                        .unwrap_or("User")
                                        .to_string();
                                    let data = callback["data"].as_str().unwrap_or("").to_string();
                                    let chat_type = callback["message"]["chat"]["type"]
                                        .as_str()
                                        .unwrap_or("private");
                                    let is_group =
                                        chat_type == "group" || chat_type == "supergroup";

                                    // Security check
                                    if !allowed_users.is_empty() {
                                        let username_lower = username
                                            .as_deref()
                                            .map(|s| s.to_lowercase())
                                            .unwrap_or_default();
                                        if !allowed_users.iter().any(|u| u == &username_lower) {
                                            debug!(
                                                "Ignoring callback from unauthorized user: {}",
                                                username_lower
                                            );
                                            continue;
                                        }
                                    }

                                    debug!("Relay received callback: {}", &data);

                                    let incoming = TelegramIncoming {
                                        chat_id: chat_id_val,
                                        user_id,
                                        username,
                                        first_name: Some(first_name),
                                        text: data.clone(),
                                        timestamp: Utc::now().timestamp(),
                                        message_id: 0,
                                        is_callback: true,
                                        callback_id: Some(callback_id.clone()),
                                        reply_to_message_id: None,
                                        intent: Some(intent),
                                        is_group,
                                    };

                                    // Queue the callback
                                    {
                                        let mut queue = messages.lock().expect("lock poisoned");
                                        if queue.len() >= max_queue {
                                            queue.pop_front();
                                        }
                                        queue.push_back(incoming);
                                    }

                                    // Answer the callback
                                    let _ = answer_callback_query(
                                        &client,
                                        &tg_api,
                                        &callback_id,
                                        Some("\u{2713}"),
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        // Network/transport error: increment counter, exponential backoff.
                        consecutive_errors = consecutive_errors.saturating_add(1);
                        let backoff = compute_backoff_secs(consecutive_errors);
                        warn!(
                            "Telegram poll transport error (#{}): {} — backing off {}s",
                            consecutive_errors, e, backoff
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                    }
                }

                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            info!("Telegram relay stopped polling");
        });

        *self.poll_handle.lock().await = Some(handle);

        Ok(())
    }

    /// Register this bot's webhook URL with Telegram's `setWebhook` API.
    ///
    /// Requires both `webhook_url` and a valid `bot_token` in config.
    /// Returns `Ok(())` on success or a descriptive error on failure.
    pub async fn register_webhook(&self) -> Result<()> {
        let url = self.config.webhook_url.as_deref().ok_or_else(|| {
            zeus_core::Error::channel("webhook_url must be set to register a webhook".to_string())
        })?;

        let set_webhook_url = format!("{}/setWebhook", self.api_url());
        let resp = self
            .client
            .post(&set_webhook_url)
            .json(&serde_json::json!({ "url": url }))
            .send()
            .await
            .map_err(|e| zeus_core::Error::channel(format!("setWebhook request failed: {e}")))?;

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| zeus_core::Error::channel(format!("setWebhook parse failed: {e}")))?;

        if body["ok"].as_bool() != Some(true) {
            return Err(zeus_core::Error::channel(format!(
                "Telegram setWebhook error: {}",
                body["description"].as_str().unwrap_or("unknown")
            )));
        }

        info!(webhook_url = %url, "Telegram webhook registered");
        Ok(())
    }

    /// Build the full setWebhook registration URL (useful for diagnostics / tests).
    pub fn webhook_register_url(&self) -> Option<String> {
        self.config
            .webhook_url
            .as_ref()
            .map(|public_url| format!("{}/setWebhook?url={}", self.api_url(), public_url))
    }

    /// Spawn an axum HTTP listener for incoming Telegram webhook POSTs.
    ///
    /// Updates are received, lightly parsed, and forwarded to the relay's
    /// internal `messages` queue and `message_callback` — identical to what
    /// the long-polling loop does.
    async fn start_webhook_listener(
        port: u16,
        path: String,
        messages: Arc<Mutex<VecDeque<TelegramIncoming>>>,
        max_queue: usize,
        message_callback: MessageCallback,
    ) -> Result<tokio::task::JoinHandle<()>> {
        use axum::{Router, body::Bytes, extract::State, http::StatusCode, routing::post};

        type WebhookState = (
            Arc<Mutex<VecDeque<TelegramIncoming>>>,
            usize,
            MessageCallback,
        );

        async fn webhook_handler(
            State((messages, max_queue, callback)): State<WebhookState>,
            body: Bytes,
        ) -> StatusCode {
            let Ok(update) = serde_json::from_slice::<serde_json::Value>(&body) else {
                return StatusCode::BAD_REQUEST;
            };
            let Some(msg) = update.get("message") else {
                // Non-message updates (callback_query, etc.) — acknowledge silently
                return StatusCode::OK;
            };

            let chat_id = msg["chat"]["id"].as_i64().unwrap_or(0);
            let user_id = msg["from"]["id"].as_i64().unwrap_or(0);
            let username = msg["from"]["username"].as_str().map(str::to_string);
            let first_name = msg["from"]["first_name"].as_str().map(str::to_string);
            let text = msg["text"]
                .as_str()
                .or_else(|| msg["caption"].as_str())
                .unwrap_or("")
                .to_string();
            let message_id = msg["message_id"].as_i64().unwrap_or(0);
            let timestamp = msg["date"].as_i64().unwrap_or(0);
            let is_group = matches!(
                msg["chat"]["type"].as_str().unwrap_or("private"),
                "group" | "supergroup"
            );
            let reply_to_message_id = msg["reply_to_message"]["message_id"].as_i64();

            let incoming = TelegramIncoming {
                chat_id,
                user_id,
                username,
                first_name,
                text: text.clone(),
                timestamp,
                message_id,
                is_callback: false,
                callback_id: None,
                reply_to_message_id,
                intent: None,
                is_group,
            };

            // Push to queue (same as polling loop)
            {
                let mut q = messages.lock().expect("lock poisoned");
                if q.len() < max_queue {
                    q.push_back(incoming);
                }
            }

            // Invoke forwarding callback
            if !text.is_empty() {
                let cb = callback.read().await;
                if let Some(ref f) = *cb {
                    f(text);
                }
            }

            StatusCode::OK
        }

        let state: WebhookState = (messages, max_queue, message_callback);
        let app = Router::new()
            .route(&path, post(webhook_handler))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
            .await
            .map_err(|e| {
                zeus_core::Error::channel(format!(
                    "Failed to bind webhook listener on port {port}: {e}"
                ))
            })?;

        info!(port, path = %path, "Telegram webhook listener started");

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                error!("Telegram webhook listener error: {e}");
            }
        });

        Ok(handle)
    }

    /// Stop the relay
    pub async fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.poll_handle.lock().await.take() {
            handle.abort();
        }
    }

    /// Whether the relay is currently running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Number of queued messages
    pub fn queued_count(&self) -> usize {
        self.messages.lock().expect("lock poisoned").len()
    }

    /// Last poll timestamp
    pub fn last_poll_time(&self) -> Option<DateTime<Utc>> {
        *self.last_poll.lock().expect("lock poisoned")
    }

    /// Get the target session
    pub async fn target_session(&self) -> Option<String> {
        self.target_session.read().await.clone()
    }

    /// Set the target session
    pub async fn set_target_session(&self, session: Option<String>) {
        *self.target_session.write().await = session;
    }

    /// Drain messages from queue
    pub fn drain_messages(&self, limit: usize) -> Vec<TelegramIncoming> {
        let mut queue = self.messages.lock().expect("lock poisoned");
        let count = std::cmp::min(limit, queue.len());
        queue.drain(..count).collect()
    }

    /// Drain all messages from queue
    pub fn drain_all(&self) -> Vec<TelegramIncoming> {
        let mut queue = self.messages.lock().expect("lock poisoned");
        queue.drain(..).collect()
    }

    /// Parse command from message text
    pub fn parse_command(&self, text: &str) -> RelayCommand {
        let text = text.trim();

        if let Some(stripped) = text.strip_prefix('/') {
            let parts: Vec<&str> = stripped.split_whitespace().collect();
            match parts.first().map(|s| s.to_lowercase()).as_deref() {
                Some("status") => RelayCommand::Status,
                Some("help") => RelayCommand::Help,
                Some("session") if parts.len() > 1 => {
                    RelayCommand::SwitchSession(parts[1].to_string())
                }
                _ => RelayCommand::Message(text.to_string()),
            }
        } else {
            RelayCommand::Message(text.to_string())
        }
    }

    /// Detect intent from a TelegramIncoming message
    pub fn detect_incoming_intent(incoming: &TelegramIncoming) -> MessageIntent {
        if incoming.is_callback {
            let parts: Vec<&str> = incoming.text.splitn(2, ':').collect();
            return MessageIntent::Callback {
                action: parts.first().unwrap_or(&"").to_string(),
                data: parts.get(1).unwrap_or(&"").to_string(),
            };
        }

        let text = incoming.text.trim();
        if let Some(stripped) = text.strip_prefix('/') {
            let parts: Vec<&str> = stripped.split_whitespace().collect();
            let name = parts
                .first()
                .map(|s| s.split('@').next().unwrap_or(s))
                .unwrap_or("")
                .to_lowercase();
            let args: Vec<String> = parts.iter().skip(1).map(|s| s.to_string()).collect();
            MessageIntent::Command { name, args }
        } else {
            MessageIntent::Chat {
                text: text.to_string(),
            }
        }
    }

    // ========================================================================
    // Status & approval helpers
    // ========================================================================

    /// Send a /status response showing all sessions
    pub async fn send_status(&self, sessions: Vec<(String, f32)>) -> Result<()> {
        let mut status_text = String::from("*Zeus Sessions Status*\n\n");

        if sessions.is_empty() {
            status_text.push_str("No active sessions found.");
        } else {
            for (session_id, context_pct) in sessions {
                let bar = progress_bar(context_pct);
                status_text.push_str(&format!("`{}` {} {:.1}%\n", session_id, bar, context_pct));
            }
        }

        let explicit = self.target_session.read().await.clone();
        if let Some(target) = &explicit {
            status_text.push_str(&format!("\n*Current target:* `{}` (explicit)", target));
        } else if let Some(detected) = detect_active_tmux_session().await {
            status_text.push_str(&format!(
                "\n*Current target:* `{}` (auto-detected)",
                detected
            ));
        } else {
            status_text.push_str("\n*Current target:* none (no tmux sessions found)");
        }

        // Add router stats
        let session_count = self.router.session_count().await;
        if session_count > 0 {
            status_text.push_str(&format!("\n*Active chats:* {}", session_count));
        }

        self.send_message(&status_text).await
    }

    /// Send approval request with interactive buttons
    pub async fn send_approval_request(
        &self,
        session_id: &str,
        content: &str,
        context_pct: f32,
    ) -> Result<()> {
        let text = format!(
            "*Approval Needed* ({}%)\n\nSession: `{}`\n\n```\n{}\n```",
            context_pct as u32,
            session_id,
            content.chars().take(500).collect::<String>()
        );

        let keyboard = InlineKeyboardBuilder::new().approval_full(session_id);

        self.send_with_keyboard(&self.config.chat_id, &text, &keyboard, None)
            .await
    }

    /// Send an error notification
    pub async fn send_error(&self, session_id: Option<&str>, error_msg: &str) -> Result<()> {
        let text = if let Some(sid) = session_id {
            format!("*Error* in `{}`\n\n{}", sid, error_msg)
        } else {
            format!("*Error*\n\n{}", error_msg)
        };

        self.send_message(&text).await
    }

    /// Fetch bot username and user ID via getMe API.
    /// Also caches `bot_user_id` for self-echo detection.
    async fn fetch_bot_username(&self) -> Result<String> {
        let url = format!("{}/getMe", self.api_url());
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| zeus_core::Error::channel(format!("Failed to call getMe: {}", e)))?;

        let body: serde_json::Value = response.json().await.map_err(|e| {
            zeus_core::Error::channel(format!("Failed to parse getMe response: {}", e))
        })?;

        let result = &body["result"];

        // Cache bot user ID for Layer 1 self-echo detection
        if let Some(id) = result["id"].as_i64() {
            *self.bot_user_id.write().await = Some(id);
        }

        result["username"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| zeus_core::Error::channel("No username in getMe response".to_string()))
    }
}

// ============================================================================
// Streaming delivery support (EditableChannel)
// ============================================================================

#[async_trait::async_trait]
impl crate::streaming::EditableChannel for TelegramRelay {
    async fn send_initial(
        &self,
        to: &crate::ChannelSource,
        content: &str,
    ) -> zeus_core::Result<String> {
        let chat_id = to.chat_id.as_deref().unwrap_or(&to.user_id);

        // Send initial message, get message_id back
        let msg_id = self
            .send_single_message(chat_id, content, None)
            .await?
            .unwrap_or(0);

        Ok(msg_id.to_string())
    }

    async fn edit_message(
        &self,
        to: &crate::ChannelSource,
        msg_id: &str,
        content: &str,
    ) -> zeus_core::Result<()> {
        let chat_id = to.chat_id.as_deref().unwrap_or(&to.user_id);

        let message_id: i64 = msg_id
            .parse()
            .map_err(|_| zeus_core::Error::channel("Invalid message ID for edit"))?;

        TelegramRelay::edit_message(self, chat_id, message_id, content).await
    }

    fn supports_editing(&self) -> bool {
        true
    }
}

impl TelegramRelay {
    /// Send a streaming reply with coalesced message edits.
    ///
    /// Uses `StreamingDelivery` to batch rapid token chunks into fewer
    /// `editMessageText` API calls, staying under Telegram's rate limits
    /// (30 msgs/sec per chat).
    ///
    /// # Parameters
    /// - `chat_id`: Target chat ID
    /// - `rx`: Token stream receiver from LLM
    /// - `coalesce_ms`: Minimum milliseconds between edits (default: 500)
    /// - `min_chars`: Minimum characters before sending initial message (default: 50)
    ///
    /// # Returns
    /// The complete accumulated response text.
    pub async fn streaming_reply(
        &self,
        chat_id: &str,
        rx: &mut tokio::sync::mpsc::Receiver<String>,
        coalesce_ms: Option<u64>,
        min_chars: Option<usize>,
    ) -> zeus_core::Result<String> {
        let delivery = crate::streaming::StreamingDelivery::new()
            .with_coalesce_ms(coalesce_ms.unwrap_or(500))
            .with_min_chars(min_chars.unwrap_or(50));

        let to = crate::ChannelSource::with_chat("telegram", "relay", chat_id);

        let fallback_self = &self;
        let fallback_chat = chat_id.to_string();

        delivery
            .deliver(
                Some(self as &dyn crate::streaming::EditableChannel),
                |content: &str| {
                    let relay = fallback_self;
                    let chat = fallback_chat.clone();
                    let content = content.to_string();
                    async move { relay.send_message_to(&chat, &content).await }
                },
                &to,
                rx,
            )
            .await
    }
}

// ============================================================================
// Helper functions
// ============================================================================

/// Generate a progress bar for context percentage
fn progress_bar(pct: f32) -> String {
    let filled = (pct / 10.0) as usize;
    let filled = filled.min(10);
    let empty = 10 - filled;
    format!(
        "[{}{}]",
        "\u{2588}".repeat(filled),
        "\u{2591}".repeat(empty)
    )
}

/// Split a long message into chunks that fit Telegram's limit.
/// Tries to split at newline boundaries, falls back to char boundaries.
pub fn split_long_message(text: &str, max_length: usize) -> Vec<String> {
    if text.len() <= max_length {
        return vec![text.to_string()];
    }

    let mut messages = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if current.len() + line.len() + 1 > max_length {
            if !current.is_empty() {
                messages.push(current);
                current = String::new();
            }

            // If a single line is too long, split at char boundaries
            if line.len() > max_length {
                let mut remaining = line;
                while !remaining.is_empty() {
                    let split_at = remaining
                        .char_indices()
                        .take_while(|(i, _)| *i < max_length)
                        .last()
                        .map(|(i, c)| i + c.len_utf8())
                        .unwrap_or(max_length.min(remaining.len()));

                    messages.push(remaining[..split_at].to_string());
                    remaining = &remaining[split_at..];
                }
                continue;
            }
        }

        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    if !current.is_empty() {
        messages.push(current);
    }

    messages
}

/// Helper: Send a response message with optional reply_to
async fn send_response_with_reply(
    client: &reqwest::Client,
    tg_api: &str,
    chat_id: &str,
    text: &str,
    reply_to: Option<i64>,
) -> Result<Option<i64>> {
    let url = format!("{}/sendMessage", tg_api);
    let text = crate::sanitize::strip_markdown(text);
    let text = text.as_str();

    let mut payload = serde_json::json!({
        "chat_id": chat_id,
        "text": text
    });

    if let Some(reply_to) = reply_to {
        payload["reply_parameters"] = serde_json::json!({
            "message_id": reply_to
        });
    }

    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| zeus_core::Error::channel(format!("Failed to send response: {}", e)))?;

    let body: serde_json::Value = response.json().await.unwrap_or_default();

    if body["ok"].as_bool() != Some(true) {
        // Retry without Markdown
        let mut fallback = serde_json::json!({
            "chat_id": chat_id,
            "text": text
        });
        if let Some(reply_to) = reply_to {
            fallback["reply_parameters"] = serde_json::json!({
                "message_id": reply_to
            });
        }

        let response = client
            .post(&url)
            .json(&fallback)
            .send()
            .await
            .map_err(|e| {
                zeus_core::Error::channel(format!("Failed to send response (fallback): {}", e))
            })?;

        let body: serde_json::Value = response.json().await.unwrap_or_default();
        return Ok(body["result"]["message_id"].as_i64());
    }

    Ok(body["result"]["message_id"].as_i64())
}

/// Helper: Answer callback query (standalone for use in spawn)
async fn answer_callback_query(
    client: &reqwest::Client,
    tg_api: &str,
    callback_id: &str,
    text: Option<&str>,
) -> Result<()> {
    let url = format!("{}/answerCallbackQuery", tg_api);

    let mut params = serde_json::json!({
        "callback_query_id": callback_id
    });

    if let Some(text) = text {
        params["text"] = serde_json::Value::String(text.to_string());
    }

    client
        .post(&url)
        .json(&params)
        .send()
        .await
        .map_err(|e| zeus_core::Error::channel(format!("Failed to answer callback: {}", e)))?;

    Ok(())
}

// ============================================================================
// Tmux relay forwarding
// ============================================================================

/// Auto-detect the active tmux session for relay forwarding.
/// Finds the first attached session, or falls back to first listed session.
/// Runs at forwarding time (not boot), so it survives session renames/restarts.
async fn detect_active_tmux_session() -> Option<String> {
    let tmux = if std::path::Path::new("/opt/homebrew/bin/tmux").exists() {
        "/opt/homebrew/bin/tmux"
    } else if std::path::Path::new("/usr/local/bin/tmux").exists() {
        "/usr/local/bin/tmux"
    } else {
        "tmux"
    };

    let uid = std::process::Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "501".to_string());
    let socket_path = format!("/private/tmp/tmux-{}/default", uid);

    let output = tokio::process::Command::new(tmux)
        .args([
            "-S",
            &socket_path,
            "list-sessions",
            "-F",
            "#{session_name}:#{session_attached}",
        ])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut first_session = None;

    for line in stdout.lines() {
        if let Some((name, attached)) = line.rsplit_once(':') {
            if first_session.is_none() {
                first_session = Some(name.to_string());
            }
            if attached == "1" {
                return Some(name.to_string());
            }
        }
    }

    first_session
}

/// Resolve tmux session: explicit config > auto-detect attached > first available
async fn resolve_tmux_target(explicit: &Option<String>) -> Option<String> {
    if let Some(session) = explicit {
        return Some(session.clone());
    }
    detect_active_tmux_session().await
}

/// Forward a message to a tmux session by typing it via send-keys.
///
/// The formatted message is typed
/// literally into the target tmux pane and Enter is pressed, so incoming
/// Telegram messages appear in the interactive Claude Code session.
async fn forward_to_tmux(session: &str, text: &str) {
    // Resolve tmux binary — check common paths since launchd may have a limited PATH
    let tmux = if std::path::Path::new("/opt/homebrew/bin/tmux").exists() {
        "/opt/homebrew/bin/tmux"
    } else if std::path::Path::new("/usr/local/bin/tmux").exists() {
        "/usr/local/bin/tmux"
    } else {
        "tmux"
    };

    // Resolve tmux socket — launchd services don't inherit TMUX env var
    let uid = std::process::Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "501".to_string());
    let socket_path = format!("/private/tmp/tmux-{}/default", uid);

    let escaped = text.replace('\'', "'\\''");

    // Type the message literally into the tmux pane
    let result = tokio::process::Command::new(tmux)
        .args([
            "-S",
            &socket_path,
            "send-keys",
            "-t",
            session,
            "-l",
            &escaped,
        ])
        .output()
        .await;
    match &result {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("tmux send-keys failed (exit {}): {}", output.status, stderr);
            return;
        }
        Err(e) => {
            warn!("tmux send-keys spawn failed: {}", e);
            return;
        }
        _ => {}
    }

    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    // Press Enter
    let result = tokio::process::Command::new(tmux)
        .args(["-S", &socket_path, "send-keys", "-t", session, "C-m"])
        .output()
        .await;
    match &result {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("tmux Enter failed (exit {}): {}", output.status, stderr);
        }
        Err(e) => {
            warn!("tmux Enter spawn failed: {}", e);
        }
        _ => {}
    }

    info!("Forwarded to tmux {}: {} chars", session, text.len());
}

// ============================================================================
// File Download
// ============================================================================

/// Download a file from Telegram via getFile API, save to /tmp/telegram_files/
async fn download_telegram_file(
    client: &reqwest::Client,
    tg_api: &str,
    file_id: &str,
    message_id: i64,
    file_type: &str,
    original_name: Option<&str>,
) -> std::result::Result<String, String> {
    // Get file info from Telegram
    let file_info_resp = client
        .get(format!("{}/getFile", tg_api))
        .query(&[("file_id", file_id)])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("getFile request failed: {}", e))?;

    let file_info: serde_json::Value = file_info_resp
        .json()
        .await
        .map_err(|e| format!("getFile parse failed: {}", e))?;

    let result = file_info
        .get("result")
        .ok_or_else(|| "No result in getFile response".to_string())?;

    let remote_path = result["file_path"]
        .as_str()
        .ok_or_else(|| "No file_path in getFile response".to_string())?;

    // Check file size (Bot API limits downloads to 20MB)
    if let Some(file_size) = result["file_size"].as_i64()
        && file_size > 20 * 1024 * 1024
    {
        return Err(format!(
            "File too large ({:.1}MB, limit 20MB)",
            file_size as f64 / (1024.0 * 1024.0)
        ));
    }

    // Download the file
    let token = tg_api
        .strip_prefix("https://api.telegram.org/bot")
        .unwrap_or("");
    let download_url = format!("https://api.telegram.org/file/bot{}/{}", token, remote_path);

    let file_bytes = client
        .get(&download_url)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("Download failed: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("Read bytes failed: {}", e))?;

    // Determine extension
    let ext = original_name
        .and_then(|n| std::path::Path::new(n).extension())
        .or_else(|| std::path::Path::new(remote_path).extension())
        .and_then(|e| e.to_str())
        .unwrap_or(match file_type {
            "photo" => "jpg",
            "audio" => "ogg",
            "video" => "mp4",
            _ => "bin",
        });

    // Ensure directory exists
    let dir = "/tmp/telegram_files";
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("Failed to create {}: {}", dir, e))?;

    let local_path = if let Some(name) = original_name {
        let safe_name: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let safe_name = safe_name.trim_start_matches('.');
        let safe_name = if safe_name.is_empty() {
            "file.bin"
        } else {
            safe_name
        };
        format!("{}/{}_{}", dir, message_id, safe_name)
    } else {
        format!("{}/{}_{}.{}", dir, file_type, message_id, ext)
    };

    tokio::fs::write(&local_path, &file_bytes)
        .await
        .map_err(|e| format!("Failed to save to {}: {}", local_path, e))?;

    info!(
        "Downloaded Telegram file: {} ({} bytes)",
        local_path,
        file_bytes.len()
    );

    Ok(local_path)
}

/// Extract file_id and metadata from a media message
fn extract_media_info(
    msg: &serde_json::Value,
) -> Option<(String, String, Option<String>, Option<String>)> {
    if let Some(photo_arr) = msg.get("photo").and_then(|p| p.as_array()) {
        photo_arr.last().map(|largest| {
            (
                "photo".to_string(),
                largest["file_id"].as_str().unwrap_or("").to_string(),
                None,
                Some("image/jpeg".to_string()),
            )
        })
    } else if let Some(doc) = msg.get("document") {
        Some((
            "document".to_string(),
            doc["file_id"].as_str().unwrap_or("").to_string(),
            doc["file_name"].as_str().map(|s| s.to_string()),
            doc["mime_type"].as_str().map(|s| s.to_string()),
        ))
    } else if let Some(audio) = msg.get("audio") {
        Some((
            "audio".to_string(),
            audio["file_id"].as_str().unwrap_or("").to_string(),
            audio["file_name"].as_str().map(|s| s.to_string()),
            audio["mime_type"].as_str().map(|s| s.to_string()),
        ))
    } else if let Some(video_note) = msg.get("video_note") {
        Some((
            "video_note".to_string(),
            video_note["file_id"].as_str().unwrap_or("").to_string(),
            video_note["file_name"].as_str().map(|s| s.to_string()),
            video_note["mime_type"].as_str().map(|s| s.to_string()),
        ))
    } else {
        msg.get("video").map(|video| {
            (
                "video".to_string(),
                video["file_id"].as_str().unwrap_or("").to_string(),
                video["file_name"].as_str().map(|s| s.to_string()),
                video["mime_type"].as_str().map(|s| s.to_string()),
            )
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_relay() -> TelegramRelay {
        TelegramRelay::new(TelegramRelayConfig {
            bot_token: "test_token".to_string(),
            chat_id: "12345".to_string(),
            allowed_users: Some("testuser".to_string()),
            target_session: None,
            max_message_length: 4000,
            rate_limit_per_minute: 30,
            enable_groups: true,
            require_mention_in_groups: true,
            bot_username: None,
            policy: None,
            webhook_port: None,
            webhook_path: default_webhook_path(),
            webhook_url: None,
            stt_provider: None,
            tts_provider: None,
            allow_bots: None,
            fleet_bot_ids: vec![],
        })
    }

    #[test]
    fn test_relay_initialization() {
        let relay = create_test_relay();
        assert!(!relay.is_running());
        assert_eq!(relay.queued_count(), 0);
    }

    #[test]
    fn test_parse_status_command() {
        let relay = create_test_relay();
        assert!(matches!(
            relay.parse_command("/status"),
            RelayCommand::Status
        ));
    }

    #[test]
    fn test_parse_help_command() {
        let relay = create_test_relay();
        assert!(matches!(relay.parse_command("/help"), RelayCommand::Help));
    }

    #[test]
    fn test_parse_session_command() {
        let relay = create_test_relay();
        if let RelayCommand::SwitchSession(id) = relay.parse_command("/session abc123") {
            assert_eq!(id, "abc123");
        } else {
            panic!("Expected SwitchSession command");
        }
    }

    #[test]
    fn test_parse_regular_message() {
        let relay = create_test_relay();
        if let RelayCommand::Message(text) = relay.parse_command("Hello Zeus") {
            assert_eq!(text, "Hello Zeus");
        } else {
            panic!("Expected Message command");
        }
    }

    #[test]
    fn test_progress_bar() {
        assert_eq!(
            progress_bar(0.0),
            "[\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}]"
        );
        assert_eq!(
            progress_bar(50.0),
            "[\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2591}\u{2591}\u{2591}\u{2591}\u{2591}]"
        );
        assert_eq!(
            progress_bar(100.0),
            "[\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}]"
        );
    }

    #[test]
    fn test_api_url() {
        let relay = create_test_relay();
        assert_eq!(relay.api_url(), "https://api.telegram.org/bottest_token");
    }

    #[tokio::test]
    async fn test_target_session_management() {
        let relay = create_test_relay();
        let _initial = relay.target_session().await;

        relay
            .set_target_session(Some("test-session".to_string()))
            .await;
        assert_eq!(
            relay.target_session().await,
            Some("test-session".to_string())
        );

        relay.set_target_session(None).await;
        assert_eq!(relay.target_session().await, None);
    }

    #[test]
    fn test_message_queue() {
        let relay = create_test_relay();

        {
            let mut queue = relay.messages.lock().expect("lock should not be poisoned");
            queue.push_back(TelegramIncoming {
                chat_id: 123,
                user_id: 456,
                username: Some("test".to_string()),
                first_name: Some("Test".to_string()),
                text: "Hello".to_string(),
                timestamp: 0,
                message_id: 1,
                is_callback: false,
                callback_id: None,
                reply_to_message_id: None,
                intent: None,
                is_group: false,
            });
        }

        assert_eq!(relay.queued_count(), 1);

        let drained = relay.drain_all();
        assert_eq!(drained.len(), 1);
        assert_eq!(relay.queued_count(), 0);
    }

    // === Message Intent Detection ===

    #[test]
    fn test_detect_intent_command() {
        let update = serde_json::json!({
            "message": {
                "text": "/status",
                "from": { "id": 123 },
                "chat": { "id": 123 }
            }
        });

        let intent = detect_intent(&update);
        assert!(matches!(
            intent,
            MessageIntent::Command { ref name, .. } if name == "status"
        ));
    }

    #[test]
    fn test_detect_intent_command_with_args() {
        let update = serde_json::json!({
            "message": {
                "text": "/session abc123 --verbose",
                "from": { "id": 123 },
                "chat": { "id": 123 }
            }
        });

        let intent = detect_intent(&update);
        if let MessageIntent::Command { name, args } = intent {
            assert_eq!(name, "session");
            assert_eq!(args, vec!["abc123", "--verbose"]);
        } else {
            panic!("Expected Command intent");
        }
    }

    #[test]
    fn test_detect_intent_command_with_bot_suffix() {
        let update = serde_json::json!({
            "message": {
                "text": "/status@mybot",
                "from": { "id": 123 },
                "chat": { "id": 123 }
            }
        });

        let intent = detect_intent(&update);
        if let MessageIntent::Command { name, .. } = intent {
            assert_eq!(name, "status");
        } else {
            panic!("Expected Command intent");
        }
    }

    #[test]
    fn test_detect_intent_chat() {
        let update = serde_json::json!({
            "message": {
                "text": "Hello, how are you?",
                "from": { "id": 123 },
                "chat": { "id": 123 }
            }
        });

        let intent = detect_intent(&update);
        if let MessageIntent::Chat { text } = intent {
            assert_eq!(text, "Hello, how are you?");
        } else {
            panic!("Expected Chat intent");
        }
    }

    #[test]
    fn test_detect_intent_photo() {
        let update = serde_json::json!({
            "message": {
                "photo": [{ "file_id": "abc", "width": 100, "height": 100 }],
                "caption": "Check this out",
                "from": { "id": 123 },
                "chat": { "id": 123 }
            }
        });

        let intent = detect_intent(&update);
        if let MessageIntent::Media {
            media_type,
            caption,
        } = intent
        {
            assert_eq!(media_type, MediaType::Photo);
            assert_eq!(caption, Some("Check this out".to_string()));
        } else {
            panic!("Expected Media intent, got {:?}", intent);
        }
    }

    #[test]
    fn test_detect_intent_document() {
        let update = serde_json::json!({
            "message": {
                "document": { "file_id": "abc", "file_name": "test.pdf" },
                "from": { "id": 123 },
                "chat": { "id": 123 }
            }
        });

        let intent = detect_intent(&update);
        assert!(matches!(
            intent,
            MessageIntent::Media {
                media_type: MediaType::Document,
                ..
            }
        ));
    }

    #[test]
    fn test_detect_intent_voice() {
        let update = serde_json::json!({
            "message": {
                "voice": { "file_id": "voice123", "duration": 5 },
                "from": { "id": 123 },
                "chat": { "id": 123 }
            }
        });

        let intent = detect_intent(&update);
        if let MessageIntent::VoiceMessage { file_id } = intent {
            assert_eq!(file_id, Some("voice123".to_string()));
        } else {
            panic!("Expected VoiceMessage intent");
        }
    }

    #[test]
    fn test_detect_intent_callback() {
        let update = serde_json::json!({
            "callback_query": {
                "id": "cb123",
                "from": { "id": 456 },
                "data": "approve:session-xyz",
                "message": {
                    "chat": { "id": 789 }
                }
            }
        });

        let intent = detect_intent(&update);
        if let MessageIntent::Callback { action, data } = intent {
            assert_eq!(action, "approve");
            assert_eq!(data, "session-xyz");
        } else {
            panic!("Expected Callback intent");
        }
    }

    #[test]
    fn test_detect_intent_sticker() {
        let update = serde_json::json!({
            "message": {
                "sticker": { "file_id": "stk123" },
                "from": { "id": 123 },
                "chat": { "id": 123 }
            }
        });

        let intent = detect_intent(&update);
        assert!(matches!(
            intent,
            MessageIntent::Media {
                media_type: MediaType::Sticker,
                caption: None
            }
        ));
    }

    // === Inline Keyboard Builder ===

    #[test]
    fn test_keyboard_builder_empty() {
        let kb = InlineKeyboardBuilder::new();
        assert!(kb.is_empty());
        assert_eq!(kb.row_count(), 0);
    }

    #[test]
    fn test_keyboard_builder_single_row() {
        let kb = InlineKeyboardBuilder::new().row(vec![("Yes", "yes"), ("No", "no")]);

        assert!(!kb.is_empty());
        assert_eq!(kb.row_count(), 1);

        let json = kb.build();
        let keyboard = json["inline_keyboard"]
            .as_array()
            .expect("should be an array");
        assert_eq!(keyboard.len(), 1);
        assert_eq!(keyboard[0].as_array().expect("should be an array").len(), 2);
        assert_eq!(keyboard[0][0]["text"], "Yes");
        assert_eq!(keyboard[0][0]["callback_data"], "yes");
    }

    #[test]
    fn test_keyboard_builder_approval_full() {
        let kb = InlineKeyboardBuilder::new().approval_full("sess-123");

        assert_eq!(kb.row_count(), 2);

        let json = kb.build();
        let keyboard = json["inline_keyboard"]
            .as_array()
            .expect("should be an array");
        assert_eq!(keyboard.len(), 2);
        // First row: Approve + Auto
        assert_eq!(keyboard[0].as_array().expect("should be an array").len(), 2);
        assert!(
            keyboard[0][0]["callback_data"]
                .as_str()
                .expect("should be a string")
                .contains("approve:sess-123")
        );
        // Second row: Reject + Read
        assert_eq!(keyboard[1].as_array().expect("should be an array").len(), 2);
        assert!(
            keyboard[1][0]["callback_data"]
                .as_str()
                .expect("should be a string")
                .contains("reject:sess-123")
        );
    }

    #[test]
    fn test_keyboard_builder_confirm_cancel() {
        let kb = InlineKeyboardBuilder::new().confirm_cancel();

        assert_eq!(kb.row_count(), 1);
        let json = kb.build();
        let row = json["inline_keyboard"][0]
            .as_array()
            .expect("should be an array");
        assert_eq!(row[0]["text"], "Confirm");
        assert_eq!(row[1]["text"], "Cancel");
    }

    #[test]
    fn test_keyboard_builder_single_button() {
        let kb = InlineKeyboardBuilder::new().button("Click me", "action:click");

        assert_eq!(kb.row_count(), 1);
        let json = kb.build();
        let row = json["inline_keyboard"][0]
            .as_array()
            .expect("should be an array");
        assert_eq!(row.len(), 1);
        assert_eq!(row[0]["text"], "Click me");
    }

    #[test]
    fn test_keyboard_builder_chaining() {
        let kb = InlineKeyboardBuilder::new()
            .row(vec![("A", "a"), ("B", "b")])
            .button("C", "c")
            .yes_no("y", "n");

        assert_eq!(kb.row_count(), 3);
    }

    // === Session Router ===

    #[test]
    fn test_session_key_dm() {
        let key = RelaySessionKey::dm(12345, 67890);
        assert!(!key.is_group());
        assert_eq!(key.to_key_string(), "12345:67890");
        assert_eq!(key.to_string(), "dm:12345:67890");
    }

    #[test]
    fn test_session_key_group() {
        let key = RelaySessionKey::group(-1001234567890);
        assert!(key.is_group());
        assert_eq!(key.to_key_string(), "-1001234567890");
        assert_eq!(key.to_string(), "group:-1001234567890");
    }

    #[test]
    fn test_session_key_auto_dm() {
        let key = RelaySessionKey::auto(12345, 67890);
        assert!(!key.is_group());
        assert_eq!(key.user_id, Some(67890));
    }

    #[test]
    fn test_session_key_auto_group() {
        let key = RelaySessionKey::auto(-1001234567890, 67890);
        assert!(key.is_group());
        assert_eq!(key.user_id, None);
    }

    #[tokio::test]
    async fn test_session_router_get_or_create() {
        let router = SessionRouter::new();
        let key = RelaySessionKey::dm(123, 456);

        let state = router.get_or_create(&key).await;
        assert_eq!(state.chat_id, 123);
        assert_eq!(state.user_id, Some(456));
        assert!(!state.is_group);
        assert_eq!(state.message_count, 1);

        // Second call increments message count
        let state2 = router.get_or_create(&key).await;
        assert_eq!(state2.message_count, 2);
    }

    #[tokio::test]
    async fn test_session_router_group() {
        let router = SessionRouter::new();
        let key = RelaySessionKey::group(-100123);

        let state = router.get_or_create(&key).await;
        assert!(state.is_group);
        assert_eq!(state.user_id, None);
    }

    #[tokio::test]
    async fn test_session_router_set_zeus_session() {
        let router = SessionRouter::new();
        let key = RelaySessionKey::dm(123, 456);

        router.get_or_create(&key).await;
        router.set_zeus_session(&key, "session-abc").await;

        let sessions = router.list_sessions().await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].zeus_session_id, Some("session-abc".to_string()));
    }

    #[tokio::test]
    async fn test_session_router_default_session() {
        let router = SessionRouter::new();
        router
            .set_default_session(Some("default-sess".to_string()))
            .await;

        let key = RelaySessionKey::dm(123, 456);
        let state = router.get_or_create(&key).await;
        assert_eq!(state.zeus_session_id, Some("default-sess".to_string()));
    }

    #[tokio::test]
    async fn test_session_router_thread_awareness() {
        let router = SessionRouter::new();
        let key = RelaySessionKey::dm(123, 456);

        router.get_or_create(&key).await;
        router.set_last_message_id(&key, 42).await;
        router.set_bot_message_id(&key, 43).await;

        let sessions = router.list_sessions().await;
        assert_eq!(sessions[0].last_message_id, Some(42));
        assert_eq!(sessions[0].last_bot_message_id, Some(43));
    }

    #[tokio::test]
    async fn test_session_router_cleanup() {
        let router = SessionRouter::new();
        let key = RelaySessionKey::dm(123, 456);
        router.get_or_create(&key).await;

        // Should not clean up recent sessions
        let cleaned = router
            .cleanup_inactive(std::time::Duration::from_secs(3600))
            .await;
        assert_eq!(cleaned, 0);
        assert_eq!(router.session_count().await, 1);
    }

    // === Message Chunking ===

    #[test]
    fn test_split_short_message() {
        let chunks = split_long_message("Hello", 4000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello");
    }

    #[test]
    fn test_split_long_message() {
        let text = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
        let chunks = split_long_message(text, 15);
        assert!(chunks.len() > 1);
        // All chunks should be <= max_length
        for chunk in &chunks {
            assert!(chunk.len() <= 15);
        }
    }

    #[test]
    fn test_split_single_long_line() {
        let text = "a".repeat(100);
        let chunks = split_long_message(&text, 30);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= 30);
        }
        // Rejoin should give original
        assert_eq!(chunks.join(""), text);
    }

    #[test]
    fn test_split_empty_message() {
        let chunks = split_long_message("", 4000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "");
    }

    // === MediaType Display ===

    #[test]
    fn test_media_type_display() {
        assert_eq!(MediaType::Photo.to_string(), "photo");
        assert_eq!(MediaType::Document.to_string(), "document");
        assert_eq!(MediaType::Voice.to_string(), "voice");
        assert_eq!(MediaType::VideoNote.to_string(), "video_note");
    }

    // === Detect Incoming Intent ===

    #[test]
    fn test_detect_incoming_intent_command() {
        let incoming = TelegramIncoming {
            chat_id: 123,
            user_id: 456,
            username: None,
            first_name: None,
            text: "/help".to_string(),
            timestamp: 0,
            message_id: 1,
            is_callback: false,
            callback_id: None,
            reply_to_message_id: None,
            intent: None,
            is_group: false,
        };

        let intent = TelegramRelay::detect_incoming_intent(&incoming);
        assert!(matches!(
            intent,
            MessageIntent::Command { ref name, .. } if name == "help"
        ));
    }

    #[test]
    fn test_detect_incoming_intent_callback() {
        let incoming = TelegramIncoming {
            chat_id: 123,
            user_id: 456,
            username: None,
            first_name: None,
            text: "approve:session-1".to_string(),
            timestamp: 0,
            message_id: 0,
            is_callback: true,
            callback_id: Some("cb123".to_string()),
            reply_to_message_id: None,
            intent: None,
            is_group: false,
        };

        let intent = TelegramRelay::detect_incoming_intent(&incoming);
        if let MessageIntent::Callback { action, data } = intent {
            assert_eq!(action, "approve");
            assert_eq!(data, "session-1");
        } else {
            panic!("Expected Callback intent");
        }
    }

    #[test]
    fn test_detect_incoming_intent_chat() {
        let incoming = TelegramIncoming {
            chat_id: 123,
            user_id: 456,
            username: None,
            first_name: None,
            text: "Just chatting".to_string(),
            timestamp: 0,
            message_id: 1,
            is_callback: false,
            callback_id: None,
            reply_to_message_id: None,
            intent: None,
            is_group: false,
        };

        let intent = TelegramRelay::detect_incoming_intent(&incoming);
        if let MessageIntent::Chat { text } = intent {
            assert_eq!(text, "Just chatting");
        } else {
            panic!("Expected Chat intent");
        }
    }

    // === Config defaults ===

    #[test]
    fn test_config_serde_defaults() {
        // bot_token and chat_id now fall back to env vars, so they're optional in JSON
        let json = r#"{"bot_token":"tok","chat_id":"123"}"#;
        let config: TelegramRelayConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(config.max_message_length, 4000);
        assert_eq!(config.rate_limit_per_minute, 30);
        assert!(config.enable_groups);
        assert!(!config.require_mention_in_groups);
        assert!(config.bot_username.is_none());
    }

    #[test]
    fn test_config_serde_without_token() {
        // bot_token and chat_id can be omitted — they default to env vars
        let json = r#"{}"#;
        let config: TelegramRelayConfig =
            serde_json::from_str(json).expect("should parse with env fallback");
        // Values come from env (or empty if unset)
        assert_eq!(config.max_message_length, 4000);
    }

    #[test]
    fn test_config_custom_values() {
        let json = r#"{
            "bot_token": "tok",
            "chat_id": "123",
            "max_message_length": 2000,
            "rate_limit_per_minute": 10,
            "enable_groups": false,
            "require_mention_in_groups": false,
            "bot_username": "mybot"
        }"#;
        let config: TelegramRelayConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(config.max_message_length, 2000);
        assert_eq!(config.rate_limit_per_minute, 10);
        assert!(!config.enable_groups);
        assert!(!config.require_mention_in_groups);
        assert_eq!(config.bot_username, Some("mybot".to_string()));
    }

    #[test]
    fn test_webhook_config_defaults() {
        let json = r#"{"bot_token":"tok","chat_id":"123"}"#;
        let config: TelegramRelayConfig = serde_json::from_str(json).unwrap();
        assert!(
            config.webhook_port.is_none(),
            "webhook_port should default to None"
        );
        assert_eq!(config.webhook_path, "/telegram/webhook");
        assert!(config.webhook_url.is_none());
    }

    #[test]
    fn test_webhook_config_custom() {
        let json = r#"{
            "bot_token": "tok",
            "chat_id": "123",
            "webhook_port": 8443,
            "webhook_path": "/my/hook",
            "webhook_url": "https://example.com/my/hook"
        }"#;
        let config: TelegramRelayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.webhook_port, Some(8443));
        assert_eq!(config.webhook_path, "/my/hook");
        assert_eq!(
            config.webhook_url.as_deref(),
            Some("https://example.com/my/hook")
        );
    }

    #[test]
    fn test_webhook_config_roundtrip() {
        let json = r#"{
            "bot_token": "tok",
            "chat_id": "123",
            "webhook_port": 8443,
            "webhook_path": "/telegram/webhook",
            "webhook_url": "https://gt.zeuslab.ai/telegram/webhook"
        }"#;
        let config: TelegramRelayConfig = serde_json::from_str(json).unwrap();
        let serialized = serde_json::to_string(&config).unwrap();
        let back: TelegramRelayConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(back.webhook_port, Some(8443));
        assert_eq!(
            back.webhook_url.as_deref(),
            Some("https://gt.zeuslab.ai/telegram/webhook")
        );
    }

    #[test]
    fn test_webhook_register_url_construction() {
        let json = r#"{
            "bot_token": "TESTTOKEN",
            "chat_id": "123",
            "webhook_url": "https://example.com/telegram/webhook"
        }"#;
        let config: TelegramRelayConfig = serde_json::from_str(json).unwrap();
        let relay = TelegramRelay::new(config);
        let reg_url = relay.webhook_register_url().expect("should produce URL");
        assert!(reg_url.contains("TESTTOKEN"), "should embed bot token");
        assert!(reg_url.contains("setWebhook"), "should call setWebhook");
        assert!(reg_url.contains("example.com"), "should include public URL");
    }

    #[test]
    fn test_webhook_register_url_none_when_no_url() {
        let json = r#"{"bot_token":"tok","chat_id":"123"}"#;
        let config: TelegramRelayConfig = serde_json::from_str(json).unwrap();
        let relay = TelegramRelay::new(config);
        assert!(relay.webhook_register_url().is_none());
    }

    // ── Bot filter Layer 2 unit tests (OpenClaw parity) ───────────────────

    #[test]
    fn test_allow_bots_mode_from_config() {
        assert_eq!(AllowBotsMode::from_config(None), AllowBotsMode::Mentions);
        assert_eq!(AllowBotsMode::from_config(Some("off")), AllowBotsMode::Off);
        assert_eq!(AllowBotsMode::from_config(Some("false")), AllowBotsMode::Off);
        assert_eq!(AllowBotsMode::from_config(Some("mentions")), AllowBotsMode::Mentions);
        assert_eq!(AllowBotsMode::from_config(Some("on")), AllowBotsMode::On);
        assert_eq!(AllowBotsMode::from_config(Some("true")), AllowBotsMode::On);
        assert_eq!(AllowBotsMode::from_config(Some("all")), AllowBotsMode::On);
        assert_eq!(AllowBotsMode::from_config(Some("unknown")), AllowBotsMode::Mentions);
    }

    #[test]
    fn test_relay_allow_bots_defaults_to_mentions() {
        let relay = create_test_relay();
        assert_eq!(relay.allow_bots, AllowBotsMode::Mentions);
    }

    fn create_test_relay_with_allow_bots(mode: Option<String>) -> TelegramRelay {
        TelegramRelay::new(TelegramRelayConfig {
            bot_token: "test_token".to_string(),
            chat_id: "12345".to_string(),
            allowed_users: Some("testuser".to_string()),
            target_session: None,
            max_message_length: 4000,
            rate_limit_per_minute: 30,
            enable_groups: true,
            require_mention_in_groups: true,
            bot_username: None,
            policy: None,
            webhook_port: None,
            webhook_path: default_webhook_path(),
            webhook_url: None,
            stt_provider: None,
            tts_provider: None,
            allow_bots: mode,
            fleet_bot_ids: vec![],
        })
    }

    #[test]
    fn test_relay_allow_bots_off_from_config() {
        let relay = create_test_relay_with_allow_bots(Some("off".to_string()));
        assert_eq!(relay.allow_bots, AllowBotsMode::Off);
    }

    #[test]
    fn test_relay_allow_bots_on_from_config() {
        let relay = create_test_relay_with_allow_bots(Some("on".to_string()));
        assert_eq!(relay.allow_bots, AllowBotsMode::On);
    }

    // ── Fleet allowlist unit tests (Lane 1 — 2026-05-06) ─────────────────

    fn create_test_relay_with_fleet_ids(fleet_ids: Vec<String>) -> TelegramRelay {
        TelegramRelay::new(TelegramRelayConfig {
            bot_token: "test_token".to_string(),
            chat_id: "12345".to_string(),
            allowed_users: None,
            target_session: None,
            max_message_length: 4000,
            rate_limit_per_minute: 30,
            enable_groups: true,
            require_mention_in_groups: false,
            bot_username: None,
            policy: None,
            webhook_port: None,
            webhook_path: default_webhook_path(),
            webhook_url: None,
            stt_provider: None,
            tts_provider: None,
            allow_bots: None,
            fleet_bot_ids: fleet_ids,
        })
    }

    #[test]
    fn test_fleet_bot_ids_empty_default() {
        let relay = create_test_relay();
        assert!(relay.fleet_bot_ids.is_empty());
    }

    #[test]
    fn test_fleet_bot_ids_parses_valid_strings_to_i64() {
        let relay = create_test_relay_with_fleet_ids(vec![
            "123456789".to_string(),
            "987654321".to_string(),
        ]);
        assert_eq!(relay.fleet_bot_ids, vec![123456789_i64, 987654321_i64]);
    }

    #[test]
    fn test_fleet_bot_ids_skips_invalid_entries() {
        let relay = create_test_relay_with_fleet_ids(vec![
            "123456789".to_string(),
            "not-a-number".to_string(),
            "987654321".to_string(),
            "".to_string(),
        ]);
        assert_eq!(relay.fleet_bot_ids, vec![123456789_i64, 987654321_i64]);
    }

    #[test]
    fn test_fleet_bot_ids_trims_whitespace() {
        let relay = create_test_relay_with_fleet_ids(vec![
            " 123456789 ".to_string(),
            "  987654321".to_string(),
        ]);
        assert_eq!(relay.fleet_bot_ids, vec![123456789_i64, 987654321_i64]);
    }

    #[test]
    fn test_fleet_bot_ids_serde_roundtrip() {
        let json = r#"{
            "bot_token": "tok",
            "chat_id": "12345",
            "fleet_bot_ids": ["111", "222", "333"]
        }"#;
        let config: TelegramRelayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.fleet_bot_ids, vec!["111", "222", "333"]);
    }
}
