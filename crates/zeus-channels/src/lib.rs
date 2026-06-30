#![recursion_limit = "256"]
//! Zeus Channels - Messaging Platform Adapters
//!
//! This crate provides adapters for various messaging platforms:
//!
//! ## Core Channels (8)
//! - Telegram (MTProto via grammers)
//! - Discord (serenity)
//! - Slack (Socket Mode + Web API)
//! - Email (lettre SMTP / async-imap IMAP)
//! - iMessage (AppleScript, macOS only)
//! - WhatsApp (Cloud API)
//! - Signal (signal-cli JSON-RPC)
//! - Matrix (Client-Server API via reqwest)
//!
//! ## Extended Channels (12)
//! - MS Teams (Microsoft Graph API + Bot Framework)
//! - WebChat (WebSocket browser widget)
//! - Google Chat (Google Workspace API)
//! - Mattermost (REST + WebSocket)
//! - IRC (tokio raw TCP)
//! - Twitch (IRC)
//! - Nostr (Relay WebSocket)
//! - LINE (Messaging API)
//! - Nextcloud Talk (Talk API)
//! - BlueBubbles (iMessage via BlueBubbles server)
//! - Feishu/Lark (Bot API)
//! - Zalo (Official Account API)
//! - MQTT (IoT/home automation via rumqttc)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use zeus_core::Result;

// Multi-account support
pub mod accounts;
pub mod debouncer;
pub mod filters;
pub use accounts::{AccountError, AccountId, AccountStore, ChannelAccount};

// Core adapter modules
pub mod chunker;
pub mod config;
pub mod discord;
pub mod discord_voice;
pub mod discovery;
pub mod email;
pub mod health;
pub mod imessage;
#[cfg(feature = "matrix")]
pub mod matrix;
pub mod mcp_bridge;
pub mod media;
pub mod media_extract;
pub mod pairing;
pub mod policy;
pub mod rich;
pub mod rich_render;
pub mod sanitize;
pub mod signal;
pub mod slack;
pub mod slack_mrkdwn;
pub mod slack_relay;
pub mod streaming;
pub mod telegram;
pub mod telegram_bot;
pub mod telegram_relay;
pub mod telegram_voice;
pub mod telegram_x_bridge;
pub mod tmux_forward;
pub mod voice_pipeline;
pub mod whatsapp;
pub mod whatsapp_additions;

// Threading / reply system
pub mod threading;
// Text direction detection (RTL/LTR)
pub mod plugins;
pub mod text_direction;

// Extended adapter modules
pub mod bluebubbles;
pub mod circuit_breaker;
pub mod feishu;
pub mod googlechat;
pub mod irc;
pub mod line;
pub mod mattermost;
pub mod mqtt;
pub mod nextcloud;
pub mod nostr;
pub mod pipeline;
pub mod sms;
pub mod teams;
pub mod twilio_whatsapp;
pub mod twitch;
pub mod voice;
pub mod webchat;
pub mod zalo;

// Social media adapters
pub mod instagram;
pub mod x;

// Core re-exports
pub use chunker::MessageChunker;
pub use config::ChannelsConfig;
pub use discord::{
    BotPresence, DiscordAdapter, DiscordConfig, DiscordEmbed, EmbedField, ReactionEvent,
    SlashCommand, SlashCommandInvocation, SlashCommandOption, SlashOptionKind,
};
pub use discord_voice::{DiscordVoiceConfig, DiscordVoiceSession, VoiceTranscript};
pub use discovery::{AdvertiseConfig, DiscoveredNode, DiscoveryManager};
pub use email::{EmailAdapter, EmailConfig};
pub use filters::AllowBotsMode;
pub use health::{
    ChannelHealth, ChannelHealthMonitor, ChannelHealthReport, HealthConfig, HealthStatus,
};
pub use imessage::{IMessageAdapter, IMessageConfig};
#[cfg(feature = "matrix")]
pub use matrix::{MatrixAdapter, MatrixConfig};
pub use media::{MediaPipeline, detect_mime_type};
pub use pairing::PairingManager;
pub use policy::{ChannelPolicy, PolicyResult};
pub use signal::{SignalAdapter, SignalConfig};
pub use slack::{
    Block, BlockElement, InteractiveAction, SelectOption, SlackAdapter, SlackChannelInfo,
    SlackConfig, SlackMessage, SlackSlashCommand, TextObject,
};
pub use slack_relay::{SlackIncoming, SlackRelay, SlackRelayConfig};
pub use streaming::{EditableChannel, StreamingDelivery};
pub use telegram::{PollOption, TelegramAdapter, TelegramConfig, TelegramPoll, TelegramPollResult};
pub use telegram_relay::{
    InlineKeyboardBuilder, MediaType, MessageIntent, RelayCommand, RelaySessionKey, SessionRouter,
    SessionState, TelegramIncoming, TelegramRelay, TelegramRelayConfig, detect_intent,
    split_long_message,
};
pub use telegram_voice::{
    SttProvider, TelegramVoiceConfig, TelegramVoiceHandler, TtsProvider, VoiceMessageKind,
    detect_voice_message,
};
pub use text_direction::{TextDirection, detect_direction, dir_attr};
pub use threading::{
    ReplyMode, ThreadContext, ThreadRouter, ThreadedReplyOptions, ThreadingConfig,
    inject_thread_context, normalize_thread_id,
};
pub use whatsapp::{WhatsAppAdapter, WhatsAppConfig, WhatsAppMode};

// Extended re-exports
pub use bluebubbles::{BlueBubblesAdapter, BlueBubblesConfig};
pub use feishu::{FeishuAdapter, FeishuConfig};
pub use googlechat::{GoogleChatAdapter, GoogleChatConfig};
pub use irc::{IrcAdapter, IrcConfig};
pub use line::{LineAdapter, LineConfig};
pub use mattermost::{MattermostAdapter, MattermostConfig};
pub use mqtt::{MqttAdapter, MqttConfig};
pub use nextcloud::{NextcloudAdapter, NextcloudConfig};
pub use nostr::{
    NostrAdapter, NostrConfig, NostrKeyPair, compute_event_id, create_signed_event, nip04_decrypt,
    nip04_encrypt, verify_event, verify_schnorr_signature,
};
pub use sms::{SmsAdapter, SmsConfig};
pub use teams::{TeamsAdapter, TeamsConfig};
pub use twilio_whatsapp::{TwilioWhatsAppAdapter, TwilioWhatsAppConfig};
pub use twitch::{TwitchAdapter, TwitchConfig};
pub use voice::{VoiceAdapter, VoiceChannelConfig};
pub use webchat::{WebChatAdapter, WebChatConfig};
pub use zalo::{ZaloAdapter, ZaloConfig};

// Social media re-exports
pub use instagram::{
    AccountInsights, CarouselItem, CreatePostOptions, InstagramAdapter, InstagramComment,
    InstagramConfig, InstagramPost, InstagramProfile, PostMetrics, PostType, UserTag,
};
pub use x::{
    CreateTweetOptions, MediaType as XMediaType, ThreadOptions, Tweet, TweetMedia, TweetMetrics,
    XAdapter, XConfig, XUserProfile,
};

/// Channel source identifier
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelSource {
    /// Channel type (telegram, discord, slack, etc.)
    pub channel_type: String,
    /// User/chat ID within the channel
    pub user_id: String,
    /// Chat/channel ID (for group chats)
    pub chat_id: Option<String>,
    /// Account ID — identifies which bot/credential set handled this message.
    /// `None` means the default (or only) account for this channel type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// Thread ID for threaded messaging (platform-specific)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Message ID to reply to (platform-specific)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    /// Classified sender type (human, bot, system, unknown)
    #[serde(default)]
    pub sender_type: zeus_core::SenderType,
}

impl ChannelSource {
    /// Create a new channel source
    pub fn new(channel_type: &str, user_id: &str) -> Self {
        Self {
            channel_type: channel_type.to_string(),
            user_id: user_id.to_string(),
            chat_id: None,
            account_id: None,
            thread_id: None,
            reply_to_message_id: None,
            sender_type: zeus_core::SenderType::Unknown,
        }
    }

    /// Create with chat ID
    pub fn with_chat(channel_type: &str, user_id: &str, chat_id: &str) -> Self {
        Self {
            channel_type: channel_type.to_string(),
            user_id: user_id.to_string(),
            chat_id: Some(chat_id.to_string()),
            account_id: None,
            thread_id: None,
            reply_to_message_id: None,
            sender_type: zeus_core::SenderType::Unknown,
        }
    }

    /// Create with a specific account ID
    pub fn with_account(mut self, account_id: &str) -> Self {
        self.account_id = Some(account_id.to_string());
        self
    }

    /// Builder: set thread ID for threaded replies
    pub fn with_thread(mut self, thread_id: &str) -> Self {
        self.thread_id = Some(thread_id.to_string());
        self
    }

    /// Builder: set reply-to message ID
    pub fn with_reply_to(mut self, message_id: &str) -> Self {
        self.reply_to_message_id = Some(message_id.to_string());
        self
    }

    /// Apply threaded reply options to this source
    pub fn with_threading(mut self, opts: &threading::ThreadedReplyOptions) -> Self {
        self.thread_id = opts.thread_id.clone();
        self.reply_to_message_id = opts.reply_to_message_id.clone();
        self
    }

    /// Builder: set sender type classification
    pub fn with_sender_type(mut self, sender_type: zeus_core::SenderType) -> Self {
        self.sender_type = sender_type;
        self
    }

    /// Get channel type
    pub fn channel_type(&self) -> &str {
        &self.channel_type
    }
}

/// Attachment received or sent via a channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelAttachment {
    /// URL of the attachment (for remote files)
    pub url: Option<String>,
    /// Raw bytes (for downloaded/inline attachments)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<u8>>,
    /// MIME type
    pub mime_type: String,
    /// Original filename
    pub filename: Option<String>,
}

impl ChannelAttachment {
    /// Create an attachment from a URL
    pub fn from_url(url: &str, mime_type: &str) -> Self {
        Self {
            url: Some(url.to_string()),
            data: None,
            mime_type: mime_type.to_string(),
            filename: None,
        }
    }

    /// Create an attachment from raw data
    pub fn from_data(data: Vec<u8>, mime_type: &str) -> Self {
        Self {
            url: None,
            data: Some(data),
            mime_type: mime_type.to_string(),
            filename: None,
        }
    }

    /// Create with filename
    pub fn with_filename(mut self, filename: &str) -> Self {
        self.filename = Some(filename.to_string());
        self
    }
}

/// Message type for distinguishing regular messages from receipts/events
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageType {
    /// Regular text message
    Text,
    /// Delivery receipt (message was delivered to recipient)
    DeliveryReceipt,
    /// Read receipt (message was read by recipient)
    ReadReceipt,
}

impl Default for MessageType {
    fn default() -> Self {
        Self::Text
    }
}

/// Incoming message from a channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    /// Unique message ID
    pub id: String,
    /// Source of the message
    pub source: ChannelSource,
    /// Message content
    pub content: String,
    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Attachments
    pub attachments: Vec<ChannelAttachment>,
    /// Thread context (if this message is part of a thread)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<threading::ThreadContext>,
    /// Detected text direction (RTL or LTR)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_dir: Option<text_direction::TextDirection>,
    /// Platform-specific message ID (for reactions, replies, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_message_id: Option<String>,
    /// Whether the agent was explicitly addressed in this message (mention, DM, etc.)
    /// Used by the agent loop to suppress context-only messages from triggering a cook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_addressed: Option<bool>,
}

impl ChannelMessage {
    /// Create a new channel message (auto-detects text direction)
    pub fn new(source: ChannelSource, content: String) -> Self {
        let text_dir = Some(text_direction::detect_direction(&content));
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            source,
            content,
            timestamp: chrono::Utc::now(),
            attachments: Vec::new(),
            thread: None,
            text_dir,
            platform_message_id: None,
            is_addressed: None,
        }
    }

    /// Create a new channel message with attachments (auto-detects text direction)
    pub fn with_attachments(
        source: ChannelSource,
        content: String,
        attachments: Vec<ChannelAttachment>,
    ) -> Self {
        let text_dir = Some(text_direction::detect_direction(&content));
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            source,
            content,
            timestamp: chrono::Utc::now(),
            attachments,
            thread: None,
            text_dir,
            platform_message_id: None,
            is_addressed: None,
        }
    }

    /// Builder: mark this message as addressed (agent was explicitly mentioned/DM'd)
    pub fn with_addressed(mut self, addressed: bool) -> Self {
        self.is_addressed = Some(addressed);
        self
    }

    /// Create a receipt message (delivery/read confirmation)
    pub fn receipt(source: ChannelSource, msg_type: MessageType, timestamps: Vec<u64>) -> Self {
        let content = format!(
            "[{} receipt for {} message(s)]",
            match msg_type {
                MessageType::DeliveryReceipt => "Delivery",
                MessageType::ReadReceipt => "Read",
                MessageType::Text => "Unknown",
            },
            timestamps.len()
        );
        Self::new(source, content)
    }

    /// Builder: attach thread context to this message
    pub fn with_thread(mut self, thread: threading::ThreadContext) -> Self {
        self.thread = Some(thread);
        self
    }

    /// Builder: set the platform-specific message ID
    pub fn with_platform_message_id(mut self, id: impl Into<String>) -> Self {
        self.platform_message_id = Some(id.into());
        self
    }
}

/// How a channel receives messages
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ReceiveMode {
    /// Channel does not support receiving messages
    None,
    /// Channel polls for messages at a fixed interval
    Polling { interval_secs: u64 },
    /// Channel uses long-polling with a timeout
    LongPolling { timeout_secs: u64 },
    /// Channel connects via WebSocket for real-time events
    WebSocket,
    /// Channel receives messages via HTTP webhook callbacks
    Webhook { path: String },
    /// Channel uses a native/proprietary protocol (e.g., MTProto)
    Native,
    /// Channel uses an external subprocess/daemon
    ExternalProcess,
}

impl ReceiveMode {
    /// Check if this mode supports receiving messages
    pub fn can_receive(&self) -> bool {
        !matches!(self, ReceiveMode::None)
    }

    /// Check if this mode requires a webhook server
    pub fn needs_webhook_server(&self) -> bool {
        matches!(self, ReceiveMode::Webhook { .. })
    }
}

/// Presence status for the bot on a channel
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceStatus {
    /// Bot is online and ready
    Online,
    /// Bot is processing / thinking
    Busy,
    /// Bot is idle / away
    Away,
    /// Bot is offline
    Offline,
}

impl std::fmt::Display for PresenceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Online => write!(f, "online"),
            Self::Busy => write!(f, "busy"),
            Self::Away => write!(f, "away"),
            Self::Offline => write!(f, "offline"),
        }
    }
}

/// Identity used when sending a message on behalf of a specific agent.
///
/// Tier 1 adapters (Discord, Slack) use `name` + `avatar_url` / `emoji`
/// via their native webhook/postMessage APIs for a distinct avatar per agent.
///
/// Tier 2 adapters (Telegram, Email, WhatsApp, Signal, iMessage, Matrix)
/// prepend `[name]` to the message text — the platform doesn't support
/// per-message identity, so the text prefix is the identity signal.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentSendIdentity {
    /// Display name shown as the message sender.
    pub name: String,
    /// Avatar / profile image URL (used by Tier 1 adapters: Discord, Slack).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// Emoji icon (Slack fallback when `avatar_url` is absent: `icon_emoji`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

impl AgentSendIdentity {
    /// Create a minimal identity with just a name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            avatar_url: None,
            emoji: None,
        }
    }

    /// Create an identity with a name and avatar URL.
    pub fn with_avatar(name: impl Into<String>, avatar_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            avatar_url: Some(avatar_url.into()),
            emoji: None,
        }
    }

    /// Format the default text prefix: `[AgentName] `.
    ///
    /// Tier 2 adapters prepend this to message content when the platform
    /// does not support native per-message identity.
    pub fn text_prefix(&self) -> String {
        format!("[{}] ", self.name)
    }

    /// Apply the text-prefix identity to content.
    ///
    /// Returns `"[AgentName] content"`.  Used by all Tier 2 adapters as the
    /// default `send_as` implementation.
    pub fn apply_prefix(&self, content: &str) -> String {
        format!("{}{}", self.text_prefix(), content)
    }
}

/// A messaging channel adapter
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Get the channel type
    fn channel_type(&self) -> &'static str;

    /// Optional account identifier for multi-bot routing.
    ///
    /// When multiple adapters share the same `channel_type` (e.g. two Discord
    /// bots), the account ID disambiguates them so outbound messages are sent
    /// through the correct bot token.
    fn account_id(&self) -> Option<&str> {
        None
    }

    /// Get the receive mode for this channel
    fn receive_mode(&self) -> ReceiveMode;

    /// Start receiving messages
    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()>;

    /// Stop receiving messages
    async fn stop(&self) -> Result<()>;

    /// Send a message
    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()>;

    /// Send a message with a specific agent identity.
    ///
    /// **Tier 1 adapters** (Discord, Slack) override this to use native
    /// webhook `username` / `avatar_url` for a distinct avatar per message.
    ///
    /// **Tier 2 adapters** (Telegram, Email, WhatsApp, Signal, iMessage,
    /// Matrix) use this default implementation, which prepends `[name]` to
    /// the message text.
    async fn send_as(
        &self,
        to: &ChannelSource,
        content: &str,
        identity: &AgentSendIdentity,
    ) -> Result<()> {
        self.send(to, &identity.apply_prefix(content)).await
    }

    /// Whether this adapter supports native per-message identity
    /// (Tier 1 — distinct avatar/username per message).
    ///
    /// Returns `true` only for adapters that override `send_as()` with
    /// native API support (Discord webhooks, Slack postMessage username).
    fn supports_native_identity(&self) -> bool {
        false
    }

    /// Send a message with threading/reply options.
    /// Default implementation falls back to `send()`, ignoring thread info.
    async fn send_threaded(
        &self,
        to: &ChannelSource,
        content: &str,
        _opts: &threading::ThreadedReplyOptions,
    ) -> Result<()> {
        self.send(to, content).await
    }

    /// Check if this channel supports threading
    fn supports_threading(&self) -> bool {
        false
    }

    /// Check if the channel is connected
    fn is_connected(&self) -> bool;

    /// Send a file attachment to a channel.
    /// `filename` is the display name, `data` is the raw bytes.
    /// Default returns an error for channels that don't support file sending.
    async fn send_file(
        &self,
        _to: &ChannelSource,
        _filename: &str,
        _data: &[u8],
        _caption: Option<&str>,
    ) -> Result<()> {
        Err(zeus_core::Error::Channel(format!(
            "File sending not supported on {} channel",
            self.channel_type()
        )))
    }

    /// Send a typing indicator to show the bot is composing a response.
    /// Default implementation is a no-op for channels that don't support it.
    async fn send_typing(&self, _to: &ChannelSource) -> Result<()> {
        Ok(())
    }

    /// Advertise this adapter's native rich-content capabilities.
    ///
    /// The gateway consults this before dispatching a [`rich::RichResponse`]
    /// to decide whether to attempt native rendering or pre-flatten to text.
    /// Default is `PLAIN_TEXT` — every adapter is at minimum text-capable.
    ///
    /// See `docs/design/gateway-rich-multimedia-responses-2026-05-22.md`.
    fn capabilities(&self) -> rich::ChannelCapabilities {
        rich::ChannelCapabilities::PLAIN_TEXT
    }

    /// Send a structured rich response. Default implementation degrades the
    /// entire response to plain text via [`rich::RichResponse::to_text`] and
    /// forwards through [`Self::send`].
    ///
    /// Tier-1 adapters (Discord, Slack) override this to use native rich
    /// primitives (embeds, blocks) — falling back to text-degradation per
    /// fragment for primitives they cannot natively render.
    async fn send_rich(
        &self,
        to: &ChannelSource,
        response: &rich::RichResponse,
    ) -> Result<()> {
        self.send(to, &response.to_text()).await
    }

    /// Set the bot's presence/status on this channel.
    /// Default implementation is a no-op for channels that don't support it.
    async fn set_presence(&self, _status: PresenceStatus) -> Result<()> {
        Ok(())
    }

    /// Check whether this channel supports typing indicators
    fn supports_typing(&self) -> bool {
        false
    }

    /// Check whether this channel supports presence/status
    fn supports_presence(&self) -> bool {
        false
    }

    /// Add a reaction emoji to a message.
    /// Requires channel_id and platform message_id.
    /// Default implementation is a no-op for channels that don't support it.
    async fn add_reaction(&self, _channel_id: &str, _message_id: &str, _emoji: &str) -> Result<()> {
        Ok(())
    }

    /// Remove a reaction emoji from a message.
    /// Default implementation is a no-op.
    async fn remove_reaction(
        &self,
        _channel_id: &str,
        _message_id: &str,
        _emoji: &str,
    ) -> Result<()> {
        Ok(())
    }

    /// Check whether this channel supports reactions
    fn supports_reactions(&self) -> bool {
        false
    }

    /// Handle an incoming webhook payload (for webhook-based channels)
    async fn handle_webhook(
        &self,
        _payload: &[u8],
        _tx: &mpsc::Sender<ChannelMessage>,
    ) -> Result<()> {
        Err(zeus_core::Error::Channel(format!(
            "Channel {} does not support webhooks",
            self.channel_type()
        )))
    }
}

/// Channel manager
pub struct ChannelManager {
    adapters: Vec<Box<dyn ChannelAdapter>>,
    tx: mpsc::Sender<ChannelMessage>,
    rx: std::sync::Mutex<Option<mpsc::Receiver<ChannelMessage>>>,
    health_monitor: Option<Arc<ChannelHealthMonitor>>,
    account_store: Option<Arc<AccountStore>>,
}

impl ChannelManager {
    /// Create a new channel manager
    pub fn new(buffer_size: usize) -> Self {
        let (tx, rx) = mpsc::channel(buffer_size);
        Self {
            adapters: Vec::new(),
            tx,
            rx: std::sync::Mutex::new(Some(rx)),
            health_monitor: None,
            account_store: None,
        }
    }

    /// Create a channel manager with an `AccountStore` for multi-account routing.
    pub fn with_account_store(buffer_size: usize, workspace_dir: &Path) -> Self {
        let (tx, rx) = mpsc::channel(buffer_size);
        Self {
            adapters: Vec::new(),
            tx,
            rx: std::sync::Mutex::new(Some(rx)),
            health_monitor: None,
            account_store: Some(Arc::new(AccountStore::new(workspace_dir))),
        }
    }

    /// Attach an existing `AccountStore` to this manager.
    pub fn set_account_store(&mut self, store: Arc<AccountStore>) {
        self.account_store = Some(store);
    }

    /// Get a reference to the account store, if configured.
    pub fn account_store(&self) -> Option<&Arc<AccountStore>> {
        self.account_store.as_ref()
    }

    /// Add a channel adapter
    pub fn add_adapter(&mut self, adapter: Box<dyn ChannelAdapter>) {
        self.adapters.push(adapter);
    }

    /// Start all channels
    pub async fn start_all(&self) -> Result<()> {
        let mut started = 0usize;
        for adapter in &self.adapters {
            match adapter.start(self.tx.clone()).await {
                Ok(()) => started += 1,
                Err(e) => {
                    tracing::warn!(
                        "Channel adapter '{}' failed to start: {} — skipping",
                        adapter.channel_type(),
                        e
                    );
                }
            }
        }
        if started == 0 && !self.adapters.is_empty() {
            tracing::warn!("No channel adapters started successfully");
        }
        Ok(())
    }

    /// Stop all channels
    pub async fn stop_all(&self) -> Result<()> {
        for adapter in &self.adapters {
            adapter.stop().await?;
        }
        Ok(())
    }

    /// Take the message receiver
    pub fn take_receiver(&self) -> Option<mpsc::Receiver<ChannelMessage>> {
        self.rx
            .lock()
            .expect("channel receiver mutex poisoned")
            .take()
    }

    /// Find the best adapter for outbound routing.
    ///
    /// When the `ChannelSource` carries an `account_id`, prefer the adapter
    /// whose `account_id()` matches.  Falls back to the first adapter with a
    /// matching `channel_type` (the primary adapter).
    fn find_adapter(&self, to: &ChannelSource) -> Option<&dyn ChannelAdapter> {
        let channel_type = to.channel_type();
        let mut fallback: Option<&dyn ChannelAdapter> = None;

        for adapter in &self.adapters {
            if adapter.channel_type() != channel_type {
                continue;
            }
            // Exact account match — return immediately
            if let Some(ref acct) = to.account_id
                && adapter.account_id() == Some(acct.as_str())
            {
                return Some(adapter.as_ref());
            }
            // Remember first type-match as fallback
            if fallback.is_none() {
                fallback = Some(adapter.as_ref());
            }
        }

        fallback
    }

    /// Check whether an outbound message should be silently suppressed.
    ///
    /// Two classes of internal housekeeping signal must never leak to users on
    /// external channels (Discord, Telegram, Slack, etc.):
    ///
    /// 1. **`HEARTBEAT_OK`** — the LLM heartbeat system emits this when no
    ///    action is needed.
    /// 2. **Goal lifecycle sentinels** — the gateway's goal scheduler posts
    ///    `[Goal completed: …]`, `[Goal ABANDONED — …]`, and `[Goal FAILED: …]`
    ///    markers (gateway.rs) to announce a goal's terminal state. These are
    ///    internal lifecycle markers, not titan responses; the titan's actual
    ///    work posts to the channel separately, so the sentinel is pure noise
    ///    (#200). The observability info-log at gateway.rs is kept.
    ///
    /// The TUI is exempt because it is a local developer interface.
    fn should_suppress_heartbeat_ok(to: &ChannelSource, content: &str) -> bool {
        if !Self::is_external_channel(to) {
            return false;
        }
        let trimmed = content.trim();
        trimmed.eq_ignore_ascii_case("HEARTBEAT_OK") || Self::is_goal_sentinel(trimmed)
    }

    /// Whether `content` is a gateway goal-lifecycle sentinel marker (#200).
    ///
    /// Matches the terminal-state markers built in `gateway.rs`:
    /// `[Goal completed: …]`, `[Goal ABANDONED — …]`, `[Goal FAILED: …]`.
    /// Case-insensitive on the state word so `abandoned`/`failed` variants are
    /// covered regardless of casing.
    fn is_goal_sentinel(trimmed: &str) -> bool {
        let Some(rest) = trimmed.strip_prefix("[Goal ") else {
            return false;
        };
        // The state word is the first token after "[Goal ", terminated by a
        // space, ':' or ']'. Lower-case it and match the known terminal states.
        let state: String = rest
            .chars()
            .take_while(|c| !matches!(c, ' ' | ':' | ']'))
            .flat_map(|c| c.to_lowercase())
            .collect();
        matches!(state.as_str(), "completed" | "abandoned" | "failed")
    }

    /// Whether the target is an external user-facing channel (TUI/local exempt).
    fn is_external_channel(to: &ChannelSource) -> bool {
        matches!(to.channel_type(), "discord" | "telegram" | "slack" | "irc" | "matrix" | "signal" | "whatsapp" | "email" | "teams" | "googlechat" | "mattermost" | "feishu" | "line" | "nostr" | "x" | "instagram" | "sms" | "webchat")
    }

    /// Send a message to a channel
    pub async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if Self::should_suppress_heartbeat_ok(to, content) {
            tracing::debug!(channel = to.channel_type(), "Suppressed internal sentinel outbound (heartbeat/goal-lifecycle)");
            return Ok(());
        }
        let adapter = self.find_adapter(to).ok_or_else(|| {
            zeus_core::Error::Channel(format!("No adapter for channel: {}", to.channel_type()))
        })?;
        adapter.send(to, content).await
    }

    /// Send a structured rich response to a channel (Cut 2 #88/#85).
    ///
    /// Routes to the adapter matching `to.channel_type()` and calls
    /// `send_rich()`. Tier-1 adapters (Discord, Slack) render natively via
    /// embeds/blocks + file attachments; other adapters degrade through the
    /// trait default to plain text via `RichResponse::to_text`.
    pub async fn send_rich(
        &self,
        to: &ChannelSource,
        response: &rich::RichResponse,
    ) -> Result<()> {
        let adapter = self.find_adapter(to).ok_or_else(|| {
            zeus_core::Error::Channel(format!("No adapter for channel: {}", to.channel_type()))
        })?;
        adapter.send_rich(to, response).await
    }

    /// Capability pre-flight for the rich producer (#88).
    ///
    /// Returns the advertised [`rich::ChannelCapabilities`] of the adapter that
    /// would handle `to`, so a producer can decide rich-vs-flatten *before*
    /// dispatch. Returns `None` when no adapter matches `to.channel_type()`.
    /// Tier-1 adapters (Discord/Slack) advertise `rich_content`; others default
    /// to `PLAIN_TEXT` and the `send_rich` trait-default degrades for them.
    pub fn capabilities(&self, to: &ChannelSource) -> Option<rich::ChannelCapabilities> {
        self.find_adapter(to).map(|a| a.capabilities())
    }

    /// Send a message with an agent identity.
    ///
    /// Routes to the adapter matching `to.channel_type()` and calls
    /// `send_as()`.  Tier 1 adapters (Discord, Slack) use native webhook
    /// identity; Tier 2 adapters prepend `[name]` to the content.
    pub async fn send_as(
        &self,
        to: &ChannelSource,
        content: &str,
        identity: &AgentSendIdentity,
    ) -> Result<()> {
        if Self::should_suppress_heartbeat_ok(to, content) {
            tracing::debug!(channel = to.channel_type(), "Suppressed internal sentinel outbound (send_as)");
            return Ok(());
        }
        let adapter = self.find_adapter(to).ok_or_else(|| {
            zeus_core::Error::Channel(format!("No adapter for channel: {}", to.channel_type()))
        })?;
        adapter.send_as(to, content, identity).await
    }

    /// Resolve an `AgentSendIdentity` from the `AccountStore` by account ID.
    ///
    /// Looks up the account, then builds an identity from its `label` field
    /// and optional `avatar_url` / `emoji` metadata keys.
    /// Returns `None` if no store is configured or the account doesn't exist.
    pub async fn resolve_identity(&self, account_id: &str) -> Option<AgentSendIdentity> {
        let store = self.account_store.as_ref()?;
        let account = store.get_account(account_id).await?;
        Some(AgentSendIdentity {
            name: account.label.clone(),
            avatar_url: account.metadata.get("avatar_url").cloned(),
            emoji: account.metadata.get("emoji").cloned(),
        })
    }

    /// Send a message using the identity associated with a `ChannelSource`'s `account_id`.
    ///
    /// If `to.account_id` is set and the `AccountStore` contains a matching account,
    /// the message is sent via `send_as()` with the resolved identity.
    /// Otherwise falls back to a plain `send()`.
    pub async fn send_for_account(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if let Some(identity) = self.resolve_identity_for(to).await {
            return self.send_as(to, content, &identity).await;
        }
        self.send(to, content).await
    }

    /// Resolve identity from the source's `account_id`, if present and found in the store.
    async fn resolve_identity_for(&self, source: &ChannelSource) -> Option<AgentSendIdentity> {
        let account_id = source.account_id.as_deref()?;
        self.resolve_identity(account_id).await
    }

    /// Send a message with threading/reply options
    pub async fn send_threaded(
        &self,
        to: &ChannelSource,
        content: &str,
        opts: &threading::ThreadedReplyOptions,
    ) -> Result<()> {
        if Self::should_suppress_heartbeat_ok(to, content) {
            tracing::debug!(channel = to.channel_type(), "Suppressed internal sentinel outbound (send_threaded)");
            return Ok(());
        }
        let adapter = self.find_adapter(to).ok_or_else(|| {
            zeus_core::Error::Channel(format!("No adapter for channel: {}", to.channel_type()))
        })?;
        adapter.send_threaded(to, content, opts).await
    }

    /// Send a file to a channel
    pub async fn send_file(
        &self,
        to: &ChannelSource,
        filename: &str,
        data: &[u8],
        caption: Option<&str>,
    ) -> Result<()> {
        let adapter = self.find_adapter(to).ok_or_else(|| {
            zeus_core::Error::Channel(format!("No adapter for channel: {}", to.channel_type()))
        })?;
        adapter.send_file(to, filename, data, caption).await
    }

    /// Send a typing indicator to a channel target
    pub async fn send_typing(&self, to: &ChannelSource) -> Result<()> {
        if let Some(adapter) = self.find_adapter(to) {
            return adapter.send_typing(to).await;
        }
        Ok(()) // silently ignore if no adapter found
    }

    /// Set presence across all connected channels
    pub async fn set_presence_all(&self, status: PresenceStatus) {
        for adapter in &self.adapters {
            if adapter.is_connected() && adapter.supports_presence() {
                let _ = adapter.set_presence(status).await;
            }
        }
    }

    /// Add a reaction to a message on the appropriate channel
    pub async fn add_reaction(
        &self,
        channel_type: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        for adapter in &self.adapters {
            if adapter.channel_type() == channel_type && adapter.supports_reactions() {
                return adapter.add_reaction(channel_id, message_id, emoji).await;
            }
        }
        Ok(()) // silently ignore if no adapter found
    }

    /// Remove a reaction from a message on the appropriate channel
    pub async fn remove_reaction(
        &self,
        channel_type: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        for adapter in &self.adapters {
            if adapter.channel_type() == channel_type && adapter.supports_reactions() {
                return adapter.remove_reaction(channel_id, message_id, emoji).await;
            }
        }
        Ok(())
    }

    /// Broadcast a message to multiple targets concurrently
    ///
    /// Returns a vec of (target, result) pairs for each target attempted.
    pub async fn broadcast(
        &self,
        content: &str,
        targets: &[ChannelSource],
    ) -> Vec<(ChannelSource, Result<()>)> {
        let mut results = Vec::new();

        for target in targets {
            let result = self.send(target, content).await;
            results.push((target.clone(), result));
        }

        results
    }

    /// Broadcast a message to ALL connected adapters.
    ///
    /// Iterates every adapter that reports `is_connected()`, builds a default
    /// `ChannelSource` for each, and sends concurrently.  Returns a vec of
    /// `(channel_type, Result)` pairs.
    pub async fn broadcast_all(&self, content: &str) -> Vec<(String, Result<()>)> {
        let mut results = Vec::new();

        for adapter in &self.adapters {
            if !adapter.is_connected() {
                continue;
            }
            let ct = adapter.channel_type().to_string();
            // Build a default target for this adapter — the adapter's
            // configured default destination (relay channel, group, inbox, etc.)
            let target = ChannelSource::new(&ct, "broadcast");
            let result = adapter.send(&target, content).await;
            results.push((ct, result));
        }

        results
    }

    /// List connected channels
    pub fn connected_channels(&self) -> Vec<&str> {
        self.adapters
            .iter()
            .filter(|a| a.is_connected())
            .map(|a| a.channel_type())
            .collect()
    }

    /// List channels that support typing indicators
    pub fn typing_capable_channels(&self) -> Vec<&str> {
        self.adapters
            .iter()
            .filter(|a| a.supports_typing())
            .map(|a| a.channel_type())
            .collect()
    }

    /// Set a health monitor for tracking adapter health
    pub fn set_health_monitor(&mut self, monitor: Arc<ChannelHealthMonitor>) {
        self.health_monitor = Some(monitor);
    }

    /// Get a clone of the inbound message sender.
    ///
    /// Useful for webhook handlers that need to inject messages
    /// from outside the adapter lifecycle.
    pub fn inbound_tx(&self) -> mpsc::Sender<ChannelMessage> {
        self.tx.clone()
    }

    /// Run a health check against all registered adapters, updating the monitor
    pub async fn health_check(&self) {
        if let Some(ref monitor) = self.health_monitor {
            for adapter in &self.adapters {
                let ct = adapter.channel_type();
                monitor.register(ct).await;
                monitor.record_check(ct, adapter.is_connected()).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_source() {
        let source = ChannelSource::new("telegram", "12345");
        assert_eq!(source.channel_type(), "telegram");
        assert_eq!(source.user_id, "12345");
    }

    #[test]
    fn test_channel_message() {
        let source = ChannelSource::new("discord", "user123");
        let msg = ChannelMessage::new(source.clone(), "Hello".to_string());
        assert_eq!(msg.content, "Hello");
        assert_eq!(msg.source.channel_type(), "discord");
        assert!(msg.attachments.is_empty());
    }

    #[test]
    fn test_receive_mode() {
        assert!(!ReceiveMode::None.can_receive());
        assert!(ReceiveMode::WebSocket.can_receive());
        assert!(
            ReceiveMode::Webhook {
                path: "/hook".to_string()
            }
            .needs_webhook_server()
        );
    }

    #[test]
    fn test_channel_attachment_from_url() {
        let att = ChannelAttachment::from_url("https://example.com/img.jpg", "image/jpeg");
        assert_eq!(att.url, Some("https://example.com/img.jpg".to_string()));
        assert!(att.data.is_none());
        assert_eq!(att.mime_type, "image/jpeg");
    }

    #[test]
    fn test_channel_attachment_from_data() {
        let att =
            ChannelAttachment::from_data(vec![1, 2, 3], "image/png").with_filename("test.png");
        assert!(att.url.is_none());
        assert_eq!(att.data.as_ref().expect("as_ref should succeed").len(), 3);
        assert_eq!(att.filename, Some("test.png".to_string()));
    }

    #[test]
    fn test_channel_message_with_attachments() {
        let source = ChannelSource::new("telegram", "12345");
        let att = ChannelAttachment::from_url("https://example.com/file.pdf", "application/pdf");
        let msg = ChannelMessage::with_attachments(source, "Check this".to_string(), vec![att]);
        assert_eq!(msg.attachments.len(), 1);
        assert_eq!(msg.attachments[0].mime_type, "application/pdf");
    }

    #[tokio::test]
    async fn test_broadcast_empty_targets() {
        let manager = ChannelManager::new(10);
        let results = manager.broadcast("hello", &[]).await;
        assert!(results.is_empty());
    }

    #[test]
    fn test_channel_source_clone_for_broadcast() {
        let targets = vec![
            ChannelSource::with_chat("telegram", "agent", "chat1"),
            ChannelSource::new("email", "user@example.com"),
            ChannelSource::with_chat("discord", "agent", "channel1"),
        ];

        // Verify clone works correctly for all targets
        let cloned: Vec<ChannelSource> = targets.iter().cloned().collect();
        assert_eq!(cloned.len(), 3);
        assert_eq!(cloned[0].channel_type(), "telegram");
        assert_eq!(cloned[0].chat_id, Some("chat1".to_string()));
        assert_eq!(cloned[1].channel_type(), "email");
        assert_eq!(cloned[1].user_id, "user@example.com");
        assert_eq!(cloned[1].chat_id, None);
        assert_eq!(cloned[2].channel_type(), "discord");
    }

    #[tokio::test]
    async fn test_broadcast_results_contain_all_targets() {
        let manager = ChannelManager::new(10);
        let targets = vec![
            ChannelSource::with_chat("telegram", "agent", "chat1"),
            ChannelSource::with_chat("slack", "agent", "channel1"),
        ];

        let results = manager.broadcast("test message", &targets).await;

        // Should have one result per target
        assert_eq!(results.len(), 2);

        // Results should correspond to the targets in order
        assert_eq!(results[0].0.channel_type(), "telegram");
        assert_eq!(results[1].0.channel_type(), "slack");

        // Both should fail because no adapters are registered
        assert!(results[0].1.is_err());
        assert!(results[1].1.is_err());
    }

    #[test]
    fn test_presence_status_display() {
        assert_eq!(PresenceStatus::Online.to_string(), "online");
        assert_eq!(PresenceStatus::Busy.to_string(), "busy");
        assert_eq!(PresenceStatus::Away.to_string(), "away");
        assert_eq!(PresenceStatus::Offline.to_string(), "offline");
    }

    #[test]
    fn test_presence_status_eq() {
        assert_eq!(PresenceStatus::Online, PresenceStatus::Online);
        assert_ne!(PresenceStatus::Online, PresenceStatus::Offline);
    }

    #[tokio::test]
    async fn test_send_typing_no_adapter() {
        let manager = ChannelManager::new(10);
        let target = ChannelSource::new("telegram", "12345");
        // Should succeed silently when no adapter found
        let result = manager.send_typing(&target).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_presence_all_empty() {
        let manager = ChannelManager::new(10);
        // Should not panic with no adapters
        manager.set_presence_all(PresenceStatus::Online).await;
    }

    #[test]
    fn test_typing_capable_channels_empty() {
        let manager = ChannelManager::new(10);
        assert!(manager.typing_capable_channels().is_empty());
    }

    // === Threading integration tests ===

    #[test]
    fn test_channel_source_with_thread() {
        let source = ChannelSource::new("slack", "user1").with_thread("thread-ts-123");
        assert_eq!(source.thread_id.as_deref(), Some("thread-ts-123"));
        assert!(source.reply_to_message_id.is_none());
    }

    #[test]
    fn test_channel_source_with_reply_to() {
        let source = ChannelSource::new("telegram", "user1").with_reply_to("msg-42");
        assert!(source.thread_id.is_none());
        assert_eq!(source.reply_to_message_id.as_deref(), Some("msg-42"));
    }

    #[test]
    fn test_channel_source_with_threading_options() {
        let opts = threading::ThreadedReplyOptions::in_thread_reply_to("t1", "m1");
        let source = ChannelSource::with_chat("slack", "user1", "channel1").with_threading(&opts);
        assert_eq!(source.thread_id.as_deref(), Some("t1"));
        assert_eq!(source.reply_to_message_id.as_deref(), Some("m1"));
        assert_eq!(source.chat_id.as_deref(), Some("channel1"));
    }

    #[test]
    fn test_channel_message_with_thread() {
        let source = ChannelSource::new("slack", "user1");
        let thread = threading::ThreadContext::new("thread-ts").with_parent_user("alice");
        let msg = ChannelMessage::new(source, "Hello thread".to_string()).with_thread(thread);
        assert!(msg.thread.is_some());
        assert_eq!(
            msg.thread
                .as_ref()
                .expect("as_ref should succeed")
                .thread_id,
            "thread-ts"
        );
        assert_eq!(
            msg.thread
                .as_ref()
                .expect("as_ref should succeed")
                .parent_user_id
                .as_deref(),
            Some("alice")
        );
    }

    #[test]
    fn test_channel_message_no_thread_default() {
        let source = ChannelSource::new("telegram", "user1");
        let msg = ChannelMessage::new(source, "Hello".to_string());
        assert!(msg.thread.is_none());
    }

    #[test]
    fn test_channel_source_serde_with_thread() {
        let source = ChannelSource::new("slack", "user1")
            .with_thread("ts-123")
            .with_reply_to("msg-0");
        let json = serde_json::to_string(&source).expect("should serialize to JSON");
        let back: ChannelSource = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(back.thread_id.as_deref(), Some("ts-123"));
        assert_eq!(back.reply_to_message_id.as_deref(), Some("msg-0"));
    }

    #[test]
    fn test_channel_source_serde_without_thread() {
        // Old JSON without thread fields should deserialize fine (defaults to None)
        let json = r#"{"channel_type":"telegram","user_id":"123","chat_id":null}"#;
        let source: ChannelSource = serde_json::from_str(json).expect("should parse successfully");
        assert!(source.thread_id.is_none());
        assert!(source.reply_to_message_id.is_none());
    }

    #[tokio::test]
    async fn test_send_threaded_no_adapter() {
        let manager = ChannelManager::new(10);
        let target = ChannelSource::new("slack", "user1");
        let opts = threading::ThreadedReplyOptions::in_thread("thread-1");
        let result = manager.send_threaded(&target, "hello", &opts).await;
        // Should fail — no adapter registered
        assert!(result.is_err());
    }

    // === Text direction auto-detection on ChannelMessage ===

    #[test]
    fn test_message_auto_detects_ltr() {
        let source = ChannelSource::new("telegram", "user1");
        let msg = ChannelMessage::new(source, "Hello world".to_string());
        assert_eq!(msg.text_dir, Some(text_direction::TextDirection::Ltr));
    }

    #[test]
    fn test_message_auto_detects_rtl_hebrew() {
        let source = ChannelSource::new("telegram", "user1");
        let msg = ChannelMessage::new(source, "שלום עולם".to_string());
        assert_eq!(msg.text_dir, Some(text_direction::TextDirection::Rtl));
    }

    #[test]
    fn test_message_auto_detects_rtl_arabic() {
        let source = ChannelSource::new("slack", "user1");
        let msg = ChannelMessage::new(source, "مرحبا بالعالم".to_string());
        assert_eq!(msg.text_dir, Some(text_direction::TextDirection::Rtl));
    }

    #[test]
    fn test_message_with_attachments_detects_direction() {
        let source = ChannelSource::new("discord", "user1");
        let att = ChannelAttachment::from_url("https://example.com/img.jpg", "image/jpeg");
        let msg = ChannelMessage::with_attachments(source, "אני שולח תמונה".to_string(), vec![att]);
        assert_eq!(msg.text_dir, Some(text_direction::TextDirection::Rtl));
    }

    #[test]
    fn test_message_serde_with_text_dir() {
        let source = ChannelSource::new("telegram", "user1");
        let msg = ChannelMessage::new(source, "שלום".to_string());
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        assert!(json.contains("\"text_dir\":\"rtl\""));
    }

    #[test]
    fn test_message_serde_without_text_dir() {
        // Old JSON without text_dir should deserialize fine
        let json = r#"{"id":"test","source":{"channel_type":"telegram","user_id":"123"},"content":"hello","timestamp":"2026-01-01T00:00:00Z","attachments":[]}"#;
        let msg: ChannelMessage = serde_json::from_str(json).expect("should parse successfully");
        assert!(msg.text_dir.is_none());
    }

    // ChannelHealthMonitor tests are in crates/zeus-channels/src/health.rs

    // -- AgentSendIdentity tests --------------------------------------------

    #[test]
    fn test_agent_identity_new() {
        let id = AgentSendIdentity::new("zeus106");
        assert_eq!(id.name, "zeus106");
        assert!(id.avatar_url.is_none());
        assert!(id.emoji.is_none());
    }

    #[test]
    fn test_agent_identity_with_avatar() {
        let id = AgentSendIdentity::with_avatar("zeus106", "https://example.com/avatar.png");
        assert_eq!(id.name, "zeus106");
        assert_eq!(
            id.avatar_url.as_deref(),
            Some("https://example.com/avatar.png")
        );
    }

    #[test]
    fn test_agent_identity_text_prefix() {
        let id = AgentSendIdentity::new("fbsd3");
        assert_eq!(id.text_prefix(), "[fbsd3] ");
    }

    #[test]
    fn test_agent_identity_apply_prefix() {
        let id = AgentSendIdentity::new("zeus107");
        assert_eq!(id.apply_prefix("hello world"), "[zeus107] hello world");
        assert_eq!(id.apply_prefix(""), "[zeus107] ");
    }

    #[test]
    fn test_agent_identity_default() {
        let id = AgentSendIdentity::default();
        assert_eq!(id.name, "");
        assert_eq!(id.apply_prefix("msg"), "[] msg");
    }

    #[test]
    fn test_agent_identity_serde_roundtrip() {
        let id = AgentSendIdentity {
            name: "fbsd1".to_string(),
            avatar_url: Some("https://example.com/img.png".to_string()),
            emoji: Some(":robot:".to_string()),
        };
        let json = serde_json::to_string(&id).unwrap();
        let back: AgentSendIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn test_agent_identity_serde_skips_none_fields() {
        let id = AgentSendIdentity::new("agent");
        let json = serde_json::to_string(&id).unwrap();
        assert!(!json.contains("avatar_url"));
        assert!(!json.contains("emoji"));
    }

    #[tokio::test]
    async fn test_send_as_default_applies_prefix() {
        // MockAdapter uses the default send_as() — verifies text-prefix path.
        struct MockAdapter {
            sent: std::sync::Mutex<Vec<String>>,
        }

        #[async_trait::async_trait]
        impl ChannelAdapter for MockAdapter {
            fn channel_type(&self) -> &'static str {
                "mock"
            }
            fn receive_mode(&self) -> ReceiveMode {
                ReceiveMode::Polling { interval_secs: 5 }
            }
            async fn start(&self, _tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
                Ok(())
            }
            async fn stop(&self) -> Result<()> {
                Ok(())
            }
            fn is_connected(&self) -> bool {
                true
            }
            async fn send(&self, _to: &ChannelSource, content: &str) -> Result<()> {
                self.sent.lock().unwrap().push(content.to_string());
                Ok(())
            }
        }

        let adapter = MockAdapter {
            sent: std::sync::Mutex::new(vec![]),
        };
        let dest = ChannelSource::new("mock", "user-1");
        let identity = AgentSendIdentity::new("zeus106");

        adapter.send_as(&dest, "hello", &identity).await.unwrap();

        let sent = adapter.sent.lock().unwrap();
        assert_eq!(sent[0], "[zeus106] hello");
    }

    #[test]
    fn test_adapter_default_no_native_identity() {
        struct MockAdapter;
        #[async_trait::async_trait]
        impl ChannelAdapter for MockAdapter {
            fn channel_type(&self) -> &'static str {
                "mock"
            }
            fn receive_mode(&self) -> ReceiveMode {
                ReceiveMode::Polling { interval_secs: 5 }
            }
            async fn start(&self, _tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
                Ok(())
            }
            async fn stop(&self) -> Result<()> {
                Ok(())
            }
            fn is_connected(&self) -> bool {
                true
            }
            async fn send(&self, _to: &ChannelSource, _content: &str) -> Result<()> {
                Ok(())
            }
        }
        assert!(!MockAdapter.supports_native_identity());
    }

    // === AccountStore + ChannelManager integration tests ===

    /// Mock adapter that records sent messages via a shared Arc buffer.
    struct RecordingAdapter {
        channel: &'static str,
        sent: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl RecordingAdapter {
        fn new(channel: &'static str, buffer: Arc<std::sync::Mutex<Vec<String>>>) -> Self {
            Self {
                channel,
                sent: buffer,
            }
        }
    }

    #[async_trait::async_trait]
    impl ChannelAdapter for RecordingAdapter {
        fn channel_type(&self) -> &'static str {
            self.channel
        }
        fn receive_mode(&self) -> ReceiveMode {
            ReceiveMode::None
        }
        async fn start(&self, _tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
            Ok(())
        }
        async fn stop(&self) -> Result<()> {
            Ok(())
        }
        fn is_connected(&self) -> bool {
            true
        }
        async fn send(&self, _to: &ChannelSource, content: &str) -> Result<()> {
            self.sent.lock().unwrap().push(content.to_string());
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_manager_with_account_store() {
        let dir = tempfile::TempDir::new().unwrap();
        let manager = ChannelManager::with_account_store(10, dir.path());
        assert!(manager.account_store().is_some());
    }

    #[tokio::test]
    async fn test_manager_set_account_store() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut manager = ChannelManager::new(10);
        assert!(manager.account_store().is_none());

        let store = Arc::new(AccountStore::new(dir.path()));
        manager.set_account_store(store);
        assert!(manager.account_store().is_some());
    }

    #[tokio::test]
    async fn test_resolve_identity_from_account() {
        let dir = tempfile::TempDir::new().unwrap();
        let manager = ChannelManager::with_account_store(10, dir.path());

        let store = manager.account_store().unwrap();
        let acct = ChannelAccount::new("telegram", "Zeus107 Bot")
            .with_metadata("avatar_url", "https://example.com/zeus107.png")
            .with_metadata("emoji", ":robot:");
        let acct_id = acct.id.clone();
        store.add_account(acct).await.unwrap();

        let identity = manager.resolve_identity(&acct_id).await.unwrap();
        assert_eq!(identity.name, "Zeus107 Bot");
        assert_eq!(
            identity.avatar_url.as_deref(),
            Some("https://example.com/zeus107.png")
        );
        assert_eq!(identity.emoji.as_deref(), Some(":robot:"));
    }

    #[tokio::test]
    async fn test_resolve_identity_missing_account() {
        let dir = tempfile::TempDir::new().unwrap();
        let manager = ChannelManager::with_account_store(10, dir.path());
        assert!(manager.resolve_identity("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_resolve_identity_no_store() {
        let manager = ChannelManager::new(10);
        assert!(manager.resolve_identity("any-id").await.is_none());
    }

    #[tokio::test]
    async fn test_send_for_account_with_identity() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut manager = ChannelManager::with_account_store(10, dir.path());

        // Add an account
        let store = manager.account_store().unwrap().clone();
        let acct = ChannelAccount::new("telegram", "fbsd3");
        let acct_id = acct.id.clone();
        store.add_account(acct).await.unwrap();

        // Add a recording adapter with shared buffer
        let buffer = Arc::new(std::sync::Mutex::new(vec![]));
        manager.add_adapter(Box::new(RecordingAdapter::new("telegram", buffer.clone())));

        // Send with account_id set → should resolve identity and prefix
        let target = ChannelSource::new("telegram", "chat123").with_account(&acct_id);
        manager.send_for_account(&target, "hello").await.unwrap();

        let sent = buffer.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], "[fbsd3] hello");
    }

    #[tokio::test]
    async fn test_send_for_account_no_account_id_falls_back() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut manager = ChannelManager::with_account_store(10, dir.path());

        let buffer = Arc::new(std::sync::Mutex::new(vec![]));
        manager.add_adapter(Box::new(RecordingAdapter::new("telegram", buffer.clone())));

        // No account_id → plain send
        let target = ChannelSource::new("telegram", "chat123");
        manager.send_for_account(&target, "hello").await.unwrap();

        let sent = buffer.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], "hello"); // no prefix
    }

    #[tokio::test]
    async fn test_send_for_account_unknown_account_falls_back() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut manager = ChannelManager::with_account_store(10, dir.path());

        let buffer = Arc::new(std::sync::Mutex::new(vec![]));
        manager.add_adapter(Box::new(RecordingAdapter::new("telegram", buffer.clone())));

        // account_id set but doesn't exist → falls back to plain send
        let target = ChannelSource::new("telegram", "chat123").with_account("nonexistent");
        manager.send_for_account(&target, "hello").await.unwrap();

        let sent = buffer.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], "hello"); // no prefix
    }

    #[tokio::test]
    async fn test_resolve_identity_minimal_metadata() {
        let dir = tempfile::TempDir::new().unwrap();
        let manager = ChannelManager::with_account_store(10, dir.path());

        let store = manager.account_store().unwrap();
        // Account with no avatar_url or emoji metadata
        let acct = ChannelAccount::new("discord", "Minimal Bot");
        let acct_id = acct.id.clone();
        store.add_account(acct).await.unwrap();

        let identity = manager.resolve_identity(&acct_id).await.unwrap();
        assert_eq!(identity.name, "Minimal Bot");
        assert!(identity.avatar_url.is_none());
        assert!(identity.emoji.is_none());
    }

    // ── HEARTBEAT_OK relay filter tests ──────────────────────────────

    #[test]
    fn test_heartbeat_ok_suppressed_on_discord() {
        let src = ChannelSource::new("discord", "123");
        assert!(ChannelManager::should_suppress_heartbeat_ok(&src, "HEARTBEAT_OK"));
    }

    #[test]
    fn test_heartbeat_ok_suppressed_on_telegram() {
        let src = ChannelSource::new("telegram", "123");
        assert!(ChannelManager::should_suppress_heartbeat_ok(&src, "HEARTBEAT_OK"));
    }

    #[test]
    fn test_heartbeat_ok_case_insensitive() {
        let src = ChannelSource::new("discord", "123");
        assert!(ChannelManager::should_suppress_heartbeat_ok(&src, "heartbeat_ok"));
        assert!(ChannelManager::should_suppress_heartbeat_ok(&src, "Heartbeat_Ok"));
    }

    #[test]
    fn test_heartbeat_ok_trimmed() {
        let src = ChannelSource::new("discord", "123");
        assert!(ChannelManager::should_suppress_heartbeat_ok(&src, "  HEARTBEAT_OK  "));
    }

    #[test]
    fn test_heartbeat_ok_not_suppressed_on_tui() {
        let src = ChannelSource::new("tui", "local");
        assert!(!ChannelManager::should_suppress_heartbeat_ok(&src, "HEARTBEAT_OK"));
    }

    #[test]
    fn test_heartbeat_ok_not_suppressed_on_api() {
        let src = ChannelSource::new("api", "local");
        assert!(!ChannelManager::should_suppress_heartbeat_ok(&src, "HEARTBEAT_OK"));
    }

    #[test]
    fn test_real_message_not_suppressed() {
        let src = ChannelSource::new("discord", "123");
        assert!(!ChannelManager::should_suppress_heartbeat_ok(&src, "Hello world"));
        assert!(!ChannelManager::should_suppress_heartbeat_ok(&src, "HEARTBEAT_OK extra text"));
        assert!(!ChannelManager::should_suppress_heartbeat_ok(&src, "Task completed: HEARTBEAT_OK"));
    }

    #[test]
    fn test_heartbeat_ok_suppressed_on_other_external_channels() {
        for ch in &["slack", "irc", "matrix", "signal", "whatsapp", "email", "teams"] {
            let src = ChannelSource::new(ch, "123");
            assert!(
                ChannelManager::should_suppress_heartbeat_ok(&src, "HEARTBEAT_OK"),
                "expected suppression on channel '{}'",
                ch
            );
        }
    }

    // ── #200: goal-lifecycle sentinel suppression ─────────────────────

    #[test]
    fn test_goal_sentinels_suppressed_on_discord() {
        let src = ChannelSource::new("discord", "123");
        // The four gateway.rs markers (exact prefixes from the format! sites).
        for marker in &[
            "[Goal completed: deploy the thing]\nshipped @ abc123",
            "[Goal ABANDONED — bounded-retry cap: poll #196]\nGave up after 25 of 25 attempts; the re-cook condition was never met. (#157 durable retry cap)",
            "[Goal ABANDONED — no-op cap: tidy memory]\nGave up after 5 consecutive no-op cooks; the goal produced no work. (#156 retry cap)",
            "[Goal FAILED: build main]\nError: core2 0.4.0 is yanked",
        ] {
            assert!(
                ChannelManager::should_suppress_heartbeat_ok(&src, marker),
                "expected suppression of goal sentinel: {:?}",
                &marker[..marker.find('\n').unwrap_or(marker.len())]
            );
        }
    }

    #[test]
    fn test_goal_sentinel_state_case_insensitive() {
        // is_goal_sentinel lower-cases the state word, so any casing matches.
        assert!(ChannelManager::is_goal_sentinel("[Goal completed: x]"));
        assert!(ChannelManager::is_goal_sentinel("[Goal COMPLETED: x]"));
        assert!(ChannelManager::is_goal_sentinel("[Goal abandoned — y]"));
        assert!(ChannelManager::is_goal_sentinel("[Goal Failed: z]"));
    }

    #[test]
    fn test_goal_sentinels_not_suppressed_on_tui() {
        let src = ChannelSource::new("tui", "local");
        assert!(!ChannelManager::should_suppress_heartbeat_ok(
            &src,
            "[Goal completed: x]\nshipped"
        ));
    }

    #[test]
    fn test_non_sentinel_goal_text_not_suppressed() {
        let src = ChannelSource::new("discord", "123");
        // Real titan prose that merely mentions a goal must NOT be suppressed.
        assert!(!ChannelManager::should_suppress_heartbeat_ok(
            &src,
            "Goal completed: I finished the report"
        )); // no leading '[Goal '
        assert!(!ChannelManager::should_suppress_heartbeat_ok(
            &src,
            "[Goal status: in progress]"
        )); // unknown state word
        assert!(!ChannelManager::is_goal_sentinel("[Goally completed: x]"));
        assert!(!ChannelManager::is_goal_sentinel("here is my [Goal completed: x]")); // not at start
    }

    // ── Cut 2 (#88/#85) sub-item 3: ChannelManager::send_rich routing ─

    /// Mock adapter that records what was dispatched (plain vs rich).
    /// Used to verify the no-regression contract: ChannelManager::send_rich
    /// routes to ChannelAdapter::send_rich, while ChannelManager::send still
    /// routes to ChannelAdapter::send.
    struct ProbeAdapter {
        ch_type: &'static str,
        rich_calls: std::sync::atomic::AtomicUsize,
        plain_calls: std::sync::atomic::AtomicUsize,
    }
    impl ProbeAdapter {
        fn new(ch_type: &'static str) -> Self {
            Self {
                ch_type,
                rich_calls: std::sync::atomic::AtomicUsize::new(0),
                plain_calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }
    #[async_trait::async_trait]
    impl ChannelAdapter for ProbeAdapter {
        fn channel_type(&self) -> &'static str { self.ch_type }
        fn receive_mode(&self) -> ReceiveMode { ReceiveMode::Polling { interval_secs: 60 } }
        async fn start(&self, _tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> Result<()> { Ok(()) }
        async fn stop(&self) -> Result<()> { Ok(()) }
        async fn send(&self, _to: &ChannelSource, _content: &str) -> Result<()> {
            self.plain_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        async fn send_rich(&self, _to: &ChannelSource, _r: &rich::RichResponse) -> Result<()> {
            self.rich_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn is_connected(&self) -> bool { true }
    }

    /// Helper: wrap an Arc-shared ProbeAdapter into a Box for ChannelManager,
    /// while keeping a clone of the Arc to inspect counters after dispatch.
    fn install_probe(cm: &mut ChannelManager, ch_type: &'static str) -> std::sync::Arc<ProbeAdapter> {
        let probe = std::sync::Arc::new(ProbeAdapter::new(ch_type));
        // ProbeAdapter is Send+Sync; wrap a clone-handle adapter that forwards.
        struct Forward(std::sync::Arc<ProbeAdapter>);
        #[async_trait::async_trait]
        impl ChannelAdapter for Forward {
            fn channel_type(&self) -> &'static str { self.0.channel_type() }
            fn receive_mode(&self) -> ReceiveMode { self.0.receive_mode() }
            async fn start(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> Result<()> {
                self.0.start(tx).await
            }
            async fn stop(&self) -> Result<()> { self.0.stop().await }
            async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
                self.0.send(to, content).await
            }
            async fn send_rich(&self, to: &ChannelSource, r: &rich::RichResponse) -> Result<()> {
                self.0.send_rich(to, r).await
            }
            fn is_connected(&self) -> bool { self.0.is_connected() }
        }
        cm.add_adapter(Box::new(Forward(probe.clone())));
        probe
    }

    #[tokio::test]
    async fn test_channel_manager_send_rich_routes_to_adapter_send_rich() {
        // Sub-item 3 acceptance: ChannelManager::send_rich dispatches to
        // ChannelAdapter::send_rich (not the plain send path).
        let mut cm = ChannelManager::new(16);
        let probe = install_probe(&mut cm, "probe-rich");

        let target = ChannelSource::new("probe-rich", "u1");
        let r = rich::RichResponse::new().text("rich payload");
        cm.send_rich(&target, &r).await.expect("send_rich must succeed");

        assert_eq!(probe.rich_calls.load(std::sync::atomic::Ordering::SeqCst), 1,
            "send_rich must call adapter.send_rich");
        assert_eq!(probe.plain_calls.load(std::sync::atomic::Ordering::SeqCst), 0,
            "send_rich must NOT fall through to adapter.send");
    }

    #[tokio::test]
    async fn test_channel_manager_send_plain_text_path_unchanged_regression() {
        // Regression guard: plain-text dispatch path must still route to
        // adapter.send (no leakage into send_rich).
        let mut cm = ChannelManager::new(16);
        let probe = install_probe(&mut cm, "probe-plain");

        let target = ChannelSource::new("probe-plain", "u1");
        cm.send(&target, "plain text").await.expect("send must succeed");

        assert_eq!(probe.plain_calls.load(std::sync::atomic::Ordering::SeqCst), 1,
            "send must call adapter.send");
        assert_eq!(probe.rich_calls.load(std::sync::atomic::Ordering::SeqCst), 0,
            "send must NOT call adapter.send_rich");
    }

    // ── #88 producer-wiring: capability pre-flight accessor ──

    #[test]
    fn test_capabilities_accessor_returns_adapter_caps() {
        // #88: producer consults ChannelManager::capabilities(to) before dispatch.
        // ProbeAdapter uses the default capabilities() => PLAIN_TEXT (no rich).
        let mut cm = ChannelManager::new(16);
        let _probe = install_probe(&mut cm, "probe-caps");

        let target = ChannelSource::new("probe-caps", "u1");
        let caps = cm.capabilities(&target).expect("adapter present => Some(caps)");
        assert!(!caps.rich_content,
            "default adapter advertises PLAIN_TEXT (no native rich) — producer must flatten");

        // No adapter for an unknown channel => None (producer treats as flatten).
        let missing = ChannelSource::new("no-such-channel", "u1");
        assert!(cm.capabilities(&missing).is_none(),
            "unknown channel => None capabilities");
    }
}

// Voice pipeline and MCP bridge
pub use circuit_breaker::{
    CircuitBreakerConfig, CircuitBreakerManager, CircuitHealth, CircuitState, RequestVerdict,
};
pub use mcp_bridge::{McpBridge, McpToolDef, McpToolResult};
pub use pipeline::{
    ChannelMetrics, MessagePipeline, MessagePriority, PipelineConfig, PipelineMessage,
    PipelineMetrics, PipelineVerdict,
};
pub use voice_pipeline::{VoicePipeline, VoicePipelineConfig};
