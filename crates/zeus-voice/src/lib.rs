//! Zeus Voice - Voice call system with Twilio integration
//!
//! Enables the Zeus agent to:
//! - Initiate phone calls to users
//! - Handle real-time bidirectional audio via WebSocket media streams
//! - Use TTS to speak to users and STT to listen
//! - Maintain call state machines and transcripts
//! - Wake word detection for hands-free activation
//! - Full voice agent loop: STT → Agent → TTS → audio response

pub mod agent_loop;
pub mod audio;
pub mod call;
#[cfg(all(feature = "audio", not(target_os = "freebsd")))]
pub mod cpal_audio;
#[cfg(not(all(feature = "audio", not(target_os = "freebsd"))))]
pub mod cpal_audio_stub;
pub mod elevenlabs;
pub mod inbound;
pub mod media;
pub mod plivo;
pub mod provider;
pub mod stt;
pub mod talk_mode;
pub mod telnyx;
pub mod tts;
pub mod tunnel;
pub mod twilio;
pub mod vad;
pub mod wake_word;
pub mod webhook;

pub use agent_loop::{
    VoiceAgentHandler, VoiceAgentLoop, VoiceCommand, VoiceLoopConfig, VoiceTTSProvider,
};
pub use call::{CallManager, CallRecord, CallState, TranscriptEntry};
#[cfg(all(feature = "audio", not(target_os = "freebsd")))]
pub use cpal_audio::{CpalAudioInput, CpalAudioOutput};
#[cfg(not(all(feature = "audio", not(target_os = "freebsd"))))]
pub use cpal_audio_stub::{CpalAudioInput, CpalAudioOutput};
pub use elevenlabs::{ElevenLabsConfig, ElevenLabsProvider};
pub use inbound::{
    InboundCallWebhook, RecordingStatusWebhook, TranscriptionResult, TwilioVoiceConfig,
};
pub use media::MediaStreamHandler;
pub use plivo::{PlivoConfig, PlivoProvider};
pub use provider::VoiceCallProvider;
pub use talk_mode::{
    AudioInputProvider, AudioOutputProvider, TalkMode, TalkModeAgentHandler, TalkModeConfig,
    TalkModeEvent, TalkModeState, TalkModeStyle, TalkModeTurn, VoiceDirective,
};
pub use telnyx::{TelnyxConfig, TelnyxProvider};
pub use tts::{PIPER_VOICES, PiperTtsProvider, PiperVoice};
pub use tunnel::{TunnelConfig, TunnelManager, TunnelProvider, TunnelStatus};
pub use twilio::TwilioProvider;
pub use vad::{FrameAnalysis, VadConfig, VadEngine, VadEvent, VadState, VadStats};
pub use wake_word::{
    DetectorState, WakeWordConfig, WakeWordDetector, WakeWordDetectorBuilder, WakeWordEngine,
    WakeWordError, WakeWordEvent, WakeWordStats,
};
pub use webhook::WebhookServer;

use serde::{Deserialize, Serialize};

/// Voice call configuration
///
/// Maps to the `[voice]` section in config.toml:
///
/// ```toml
/// [voice]
/// provider = "twilio"
/// account_sid = "AC..."
/// auth_token = "..."
/// from_number = "+15551234567"
/// webhook_base_url = "https://your-domain.com"
/// webhook_port = 8090
/// tts_voice = "default"
/// tts_provider = "piper"
/// piper_url = "http://localhost:8104"
/// stt_provider = "groq"
/// streaming_tts = true
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    /// Voice call provider: "twilio", "telnyx", "plivo"
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Twilio Account SID
    #[serde(default)]
    pub account_sid: String,
    /// Twilio Auth Token
    #[serde(default)]
    pub auth_token: String,
    /// Twilio phone number (caller ID)
    #[serde(default)]
    pub from_number: String,
    /// Webhook base URL (must be publicly accessible)
    /// e.g., "https://your-domain.com" or ngrok tunnel
    #[serde(default)]
    pub webhook_base_url: String,
    /// Webhook port for receiving call events
    #[serde(default = "default_webhook_port")]
    pub webhook_port: u16,
    /// Default TTS voice identifier
    #[serde(default = "default_tts_voice")]
    pub tts_voice: String,
    /// TTS provider: "piper", "openai", "elevenlabs", "edge"
    #[serde(default = "default_tts_provider")]
    pub tts_provider: String,
    /// Piper TTS server URL (e.g., "http://localhost:8104")
    #[serde(default)]
    pub piper_url: Option<String>,
    /// STT provider: "groq" or "openai"
    #[serde(default)]
    pub stt_provider: Option<String>,
    /// Enable streaming TTS for lower latency
    #[serde(default = "default_streaming_tts")]
    pub streaming_tts: bool,

    /// Talk mode configuration (local microphone conversation loop)
    #[serde(default)]
    pub talk_mode: Option<talk_mode::TalkModeConfig>,

    /// ElevenLabs TTS configuration
    #[serde(default)]
    pub elevenlabs: Option<elevenlabs::ElevenLabsConfig>,
}

fn default_provider() -> String {
    "twilio".to_string()
}

fn default_tts_provider() -> String {
    "piper".to_string()
}

fn default_streaming_tts() -> bool {
    true
}

fn default_webhook_port() -> u16 {
    8090
}

fn default_tts_voice() -> String {
    "Polly.Amy".to_string()
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            account_sid: String::new(),
            auth_token: String::new(),
            from_number: String::new(),
            webhook_base_url: String::new(),
            webhook_port: default_webhook_port(),
            tts_voice: default_tts_voice(),
            tts_provider: default_tts_provider(),
            piper_url: None,
            stt_provider: None,
            streaming_tts: default_streaming_tts(),
            talk_mode: None,
            elevenlabs: None,
        }
    }
}

impl VoiceConfig {
    /// Apply environment variable overrides to the config.
    ///
    /// Checks the following env vars:
    /// - `TWILIO_ACCOUNT_SID` -> `account_sid`
    /// - `TWILIO_AUTH_TOKEN` -> `auth_token`
    /// - `TWILIO_PHONE_NUMBER` -> `from_number`
    /// - `ZEUS_PIPER_URL` -> `piper_url`
    /// - `ZEUS_TTS_PROVIDER` -> `tts_provider`
    /// - `ZEUS_STT_PROVIDER` -> `stt_provider`
    /// - `ELEVENLABS_API_KEY` -> `elevenlabs.api_key`
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(sid) = std::env::var("TWILIO_ACCOUNT_SID") {
            self.account_sid = sid;
        }
        if let Ok(token) = std::env::var("TWILIO_AUTH_TOKEN") {
            self.auth_token = token;
        }
        if let Ok(phone) = std::env::var("TWILIO_PHONE_NUMBER") {
            self.from_number = phone;
        }
        if let Ok(url) = std::env::var("ZEUS_PIPER_URL") {
            self.piper_url = Some(url);
        }
        if let Ok(provider) = std::env::var("ZEUS_TTS_PROVIDER") {
            self.tts_provider = provider;
        }
        if let Ok(provider) = std::env::var("ZEUS_STT_PROVIDER") {
            self.stt_provider = Some(provider);
        }
        if let Ok(key) = std::env::var("ELEVENLABS_API_KEY") {
            let elevenlabs = self
                .elevenlabs
                .get_or_insert_with(elevenlabs::ElevenLabsConfig::default);
            elevenlabs.api_key = Some(key);
        }
        self
    }

    /// Create a config from environment variables with defaults.
    ///
    /// Equivalent to `VoiceConfig::default().with_env_overrides()`.
    pub fn from_env() -> Self {
        Self::default().with_env_overrides()
    }

    /// Build a `VoiceLoopConfig` from this voice config.
    pub fn to_loop_config(&self) -> agent_loop::VoiceLoopConfig {
        agent_loop::VoiceLoopConfig {
            voice: self.tts_voice.clone(),
            streaming_tts: self.streaming_tts,
            stt_provider: self.stt_provider.clone(),
            piper_url: self.piper_url.clone(),
            tts_provider: self.tts_provider.clone(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voice_config_defaults() {
        let config = VoiceConfig::default();
        assert_eq!(config.provider, "twilio");
        assert_eq!(config.webhook_port, 8090);
        assert_eq!(config.tts_voice, "Polly.Amy");
        assert_eq!(config.tts_provider, "piper");
        assert!(config.streaming_tts);
        assert!(config.piper_url.is_none());
        assert!(config.stt_provider.is_none());
        assert!(config.account_sid.is_empty());
        assert!(config.auth_token.is_empty());
        assert!(config.from_number.is_empty());
        assert!(config.webhook_base_url.is_empty());
        assert!(config.talk_mode.is_none());
        assert!(config.elevenlabs.is_none());
    }

    #[test]
    fn test_voice_config_serialization_roundtrip() {
        let config = VoiceConfig {
            provider: "twilio".to_string(),
            account_sid: "AC1234567890".to_string(),
            auth_token: "secret_token".to_string(),
            from_number: "+15551234567".to_string(),
            webhook_base_url: "https://example.ngrok.io".to_string(),
            webhook_port: 9090,
            tts_voice: "Polly.Joanna".to_string(),
            tts_provider: "openai".to_string(),
            piper_url: Some("http://localhost:8104".to_string()),
            stt_provider: Some("groq".to_string()),
            streaming_tts: false,
            talk_mode: None,
            elevenlabs: None,
        };
        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        let deserialized: VoiceConfig =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(deserialized.account_sid, "AC1234567890");
        assert_eq!(deserialized.auth_token, "secret_token");
        assert_eq!(deserialized.from_number, "+15551234567");
        assert_eq!(deserialized.webhook_base_url, "https://example.ngrok.io");
        assert_eq!(deserialized.webhook_port, 9090);
        assert_eq!(deserialized.tts_voice, "Polly.Joanna");
        assert_eq!(deserialized.tts_provider, "openai");
        assert_eq!(
            deserialized.piper_url.as_deref(),
            Some("http://localhost:8104")
        );
        assert_eq!(deserialized.stt_provider.as_deref(), Some("groq"));
        assert!(!deserialized.streaming_tts);
    }

    #[test]
    fn test_voice_config_deserialize_with_defaults() {
        let json = r#"{
            "account_sid": "AC123",
            "auth_token": "tok",
            "from_number": "+1555",
            "webhook_base_url": "https://example.com"
        }"#;
        let config: VoiceConfig = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(config.account_sid, "AC123");
        assert_eq!(config.webhook_port, 8090);
        assert_eq!(config.tts_voice, "Polly.Amy");
        assert_eq!(config.tts_provider, "piper");
        assert!(config.streaming_tts);
    }

    #[test]
    fn test_voice_config_to_loop_config() {
        let config = VoiceConfig {
            tts_voice: "en-us-amy".to_string(),
            tts_provider: "piper".to_string(),
            piper_url: Some("http://localhost:8104".to_string()),
            stt_provider: Some("groq".to_string()),
            streaming_tts: true,
            ..Default::default()
        };
        let loop_config = config.to_loop_config();
        assert_eq!(loop_config.voice, "en-us-amy");
        assert_eq!(loop_config.tts_provider, "piper");
        assert!(loop_config.streaming_tts);
        assert_eq!(
            loop_config.piper_url.as_deref(),
            Some("http://localhost:8104")
        );
        assert_eq!(loop_config.stt_provider.as_deref(), Some("groq"));
    }

    #[test]
    fn test_from_env_defaults() {
        temp_env::with_vars(
            [
                ("TWILIO_ACCOUNT_SID", None::<&str>),
                ("TWILIO_AUTH_TOKEN", None),
                ("TWILIO_PHONE_NUMBER", None),
            ],
            || {
                let config = VoiceConfig::from_env();
                assert!(config.account_sid.is_empty());
                assert!(config.auth_token.is_empty());
                assert!(config.from_number.is_empty());
                assert_eq!(config.webhook_port, 8090);
                assert_eq!(config.tts_voice, "Polly.Amy");
            },
        );
    }

    #[test]
    fn test_with_env_overrides() {
        temp_env::with_vars(
            [
                ("TWILIO_ACCOUNT_SID", Some("AC_ENV_SID")),
                ("TWILIO_AUTH_TOKEN", Some("env_token_123")),
                ("TWILIO_PHONE_NUMBER", Some("+15559999999")),
            ],
            || {
                let config = VoiceConfig::default().with_env_overrides();
                assert_eq!(config.account_sid, "AC_ENV_SID");
                assert_eq!(config.auth_token, "env_token_123");
                assert_eq!(config.from_number, "+15559999999");
            },
        );
    }

    #[test]
    fn test_voice_config_env_partial_override() {
        temp_env::with_vars(
            [
                ("TWILIO_ACCOUNT_SID", None),
                ("TWILIO_AUTH_TOKEN", None),
                ("TWILIO_PHONE_NUMBER", Some("+15558888888")),
            ],
            || {
                let config = VoiceConfig {
                    account_sid: "AC_FROM_CONFIG".to_string(),
                    auth_token: "token_from_config".to_string(),
                    from_number: "+15550000000".to_string(),
                    webhook_base_url: "https://example.com".to_string(),
                    webhook_port: 9090,
                    tts_voice: "Polly.Joanna".to_string(),
                    ..Default::default()
                }
                .with_env_overrides();

                // account_sid and auth_token should remain from config (env vars not set)
                assert_eq!(config.account_sid, "AC_FROM_CONFIG");
                assert_eq!(config.auth_token, "token_from_config");
                // from_number should be overridden by env var
                assert_eq!(config.from_number, "+15558888888");
                // Other fields should be unchanged
                assert_eq!(config.webhook_base_url, "https://example.com");
                assert_eq!(config.webhook_port, 9090);
                assert_eq!(config.tts_voice, "Polly.Joanna");
            },
        );
    }
}
