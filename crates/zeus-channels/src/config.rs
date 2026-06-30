//! Channel configuration

#[cfg(feature = "matrix")]
use crate::matrix::MatrixConfig;

use crate::{
    // Extended channels
    bluebubbles::BlueBubblesConfig,
    // Core channels
    discord::DiscordConfig,
    email::EmailConfig,
    feishu::FeishuConfig,
    googlechat::GoogleChatConfig,
    imessage::IMessageConfig,
    irc::IrcConfig,
    line::LineConfig,
    mattermost::MattermostConfig,
    mqtt::MqttConfig,
    nextcloud::NextcloudConfig,
    nostr::NostrConfig,
    signal::SignalConfig,
    slack::SlackConfig,
    sms::SmsConfig,
    teams::TeamsConfig,
    telegram::TelegramConfig,
    twilio_whatsapp::TwilioWhatsAppConfig,
    twitch::TwitchConfig,
    voice::VoiceChannelConfig,
    webchat::WebChatConfig,
    whatsapp::WhatsAppConfig,
    zalo::ZaloConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Media size limits ────────────────────────────────────────────────────────

/// Per-channel-type maximum attachment size in bytes.
///
/// Enforced by `MessagePipeline::validate_media_size()` before attachments are
/// processed. Prevents OOM on constrained hosts (e.g. FreeBSD jails).
///
/// # Defaults (bytes)
/// | Channel      | Default |
/// |---|---|
/// | telegram     | 50 MB   |
/// | discord      | 25 MB   |
/// | slack        | 1 GB    |
/// | email        | 25 MB   |
/// | whatsapp     | 100 MB  |
/// | signal       | 100 MB  |
/// | matrix       | 50 MB   |
/// | mattermost   | 50 MB   |
/// | *fallback*   | 50 MB   |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaLimits {
    /// Per-channel-type overrides. Key = channel type string (e.g. "telegram").
    #[serde(default)]
    pub per_channel: HashMap<String, u64>,
    /// Fallback limit for channel types not listed in `per_channel`.
    #[serde(default = "default_fallback_limit")]
    pub fallback_bytes: u64,
}

const MB: u64 = 1024 * 1024;
const GB: u64 = 1024 * MB;

fn default_fallback_limit() -> u64 {
    50 * MB
}

impl MediaLimits {
    /// Return the limit for a given channel type (falls back to `fallback_bytes`).
    pub fn limit_for(&self, channel_type: &str) -> u64 {
        self.per_channel
            .get(channel_type)
            .copied()
            .unwrap_or(self.fallback_bytes)
    }

    /// Validate an attachment size. Returns `Err` with a human-readable message
    /// when the attachment exceeds the configured limit.
    pub fn validate(&self, channel_type: &str, size_bytes: u64) -> Result<(), String> {
        let limit = self.limit_for(channel_type);
        if size_bytes > limit {
            Err(format!(
                "attachment size {} bytes exceeds {} limit of {} bytes",
                size_bytes, channel_type, limit
            ))
        } else {
            Ok(())
        }
    }
}

impl Default for MediaLimits {
    fn default() -> Self {
        let mut per_channel = HashMap::new();
        per_channel.insert("telegram".into(), 50 * MB);
        per_channel.insert("discord".into(), 25 * MB);
        per_channel.insert("slack".into(), GB);
        per_channel.insert("email".into(), 25 * MB);
        per_channel.insert("whatsapp".into(), 100 * MB);
        per_channel.insert("twilio_whatsapp".into(), 100 * MB);
        per_channel.insert("signal".into(), 100 * MB);
        per_channel.insert("matrix".into(), 50 * MB);
        per_channel.insert("mattermost".into(), 50 * MB);
        per_channel.insert("teams".into(), 250 * MB);
        per_channel.insert("imessage".into(), 100 * MB);
        Self {
            per_channel,
            fallback_bytes: default_fallback_limit(),
        }
    }
}

/// Channels configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelsConfig {
    // ============ Core Channels (8) ============
    /// Telegram configuration
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,

    /// Discord configuration
    #[serde(default)]
    pub discord: Option<DiscordConfig>,

    /// Slack configuration
    #[serde(default)]
    pub slack: Option<SlackConfig>,

    /// Email configuration
    #[serde(default)]
    pub email: Option<EmailConfig>,

    /// iMessage configuration
    #[serde(default)]
    pub imessage: Option<IMessageConfig>,

    /// WhatsApp configuration
    #[serde(default)]
    pub whatsapp: Option<WhatsAppConfig>,

    /// Signal configuration
    #[serde(default)]
    pub signal: Option<SignalConfig>,

    /// Matrix configuration (requires `matrix` feature)
    #[cfg(feature = "matrix")]
    #[serde(default)]
    pub matrix: Option<MatrixConfig>,

    // ============ Extended Channels (12) ============
    /// Microsoft Teams configuration
    #[serde(default)]
    pub teams: Option<TeamsConfig>,

    /// WebChat configuration (browser widget)
    #[serde(default)]
    pub webchat: Option<WebChatConfig>,

    /// Google Chat configuration
    #[serde(default)]
    pub googlechat: Option<GoogleChatConfig>,

    /// Mattermost configuration
    #[serde(default)]
    pub mattermost: Option<MattermostConfig>,

    /// IRC configuration
    #[serde(default)]
    pub irc: Option<IrcConfig>,

    /// Twitch configuration
    #[serde(default)]
    pub twitch: Option<TwitchConfig>,

    /// Nostr configuration
    #[serde(default)]
    pub nostr: Option<NostrConfig>,

    /// LINE configuration
    #[serde(default)]
    pub line: Option<LineConfig>,

    /// Nextcloud Talk configuration
    #[serde(default)]
    pub nextcloud: Option<NextcloudConfig>,

    /// BlueBubbles configuration (iMessage alternative)
    #[serde(default)]
    pub bluebubbles: Option<BlueBubblesConfig>,

    /// Feishu/Lark configuration
    #[serde(default)]
    pub feishu: Option<FeishuConfig>,

    /// Zalo configuration
    #[serde(default)]
    pub zalo: Option<ZaloConfig>,

    // ============ IoT / Machine Channels ============
    /// MQTT configuration (IoT/home automation)
    #[serde(default)]
    pub mqtt: Option<MqttConfig>,

    // ============ Twilio Channels ============
    /// SMS configuration (Twilio)
    #[serde(default)]
    pub sms: Option<SmsConfig>,

    /// Twilio WhatsApp configuration
    #[serde(default)]
    pub twilio_whatsapp: Option<TwilioWhatsAppConfig>,

    // ============ Voice Channel ============
    /// Voice (Twilio) configuration
    #[serde(default)]
    pub voice: Option<VoiceChannelConfig>,

    // ============ General Settings ============
    /// Message buffer size
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,
}

fn default_buffer_size() -> usize {
    1000
}
