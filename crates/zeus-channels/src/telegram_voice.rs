//! Telegram voice message handler
//!
//! Provides detection, download, transcription (STT), and TTS reply for
//! Telegram voice messages. This module does NOT depend on `zeus-tts` —
//! it makes direct HTTP calls to Whisper/Groq/OpenAI APIs to avoid circular
//! crate dependencies.

use base64::Engine;
use reqwest::multipart;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use zeus_core::{Error, Result};

// ============================================================================
// Configuration
// ============================================================================

/// STT provider for transcription
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SttProvider {
    /// OpenAI Whisper API (or any OpenAI-compatible endpoint)
    Whisper {
        api_key: String,
        #[serde(default = "default_whisper_url")]
        api_url: String,
    },
    /// Groq Whisper (faster, hosted)
    Groq { api_key: String },
    /// Self-hosted whisper.cpp server (no auth required)
    WhisperCpp { api_url: String },
}

fn default_whisper_url() -> String {
    "https://api.openai.com/v1/audio/transcriptions".to_string()
}

impl SttProvider {
    /// Create a Whisper provider using the OpenAI API
    pub fn whisper(api_key: impl Into<String>) -> Self {
        Self::Whisper {
            api_key: api_key.into(),
            api_url: default_whisper_url(),
        }
    }

    /// Create a Whisper provider with a custom endpoint URL
    pub fn whisper_custom(api_key: impl Into<String>, api_url: impl Into<String>) -> Self {
        Self::Whisper {
            api_key: api_key.into(),
            api_url: api_url.into(),
        }
    }

    /// Create a Groq-hosted Whisper provider
    pub fn groq(api_key: impl Into<String>) -> Self {
        Self::Groq {
            api_key: api_key.into(),
        }
    }

    /// Create a whisper.cpp provider with a server URL (no auth needed)
    pub fn whisper_cpp(api_url: impl Into<String>) -> Self {
        Self::WhisperCpp {
            api_url: api_url.into(),
        }
    }

    /// Create an STT provider from Zeus config (single source of truth).
    ///
    /// Priority: `deployment.whisper_stt_url` (self-hosted, no auth) >
    /// `credentials.GROQ_API_KEY` > `credentials.OPENAI_API_KEY`.
    /// Returns `None` if none are set.
    pub fn from_config(config: &zeus_core::Config) -> Option<Self> {
        if let Some(url) = config
            .deployment
            .as_ref()
            .and_then(|d| d.whisper_stt_url.as_ref())
            && !url.is_empty()
        {
            return Some(Self::whisper_cpp(url.clone()));
        }
        if let Some(key) = config.credentials.get("GROQ_API_KEY")
            && !key.is_empty()
        {
            return Some(Self::groq(key.clone()));
        }
        if let Some(key) = config.credentials.get("OPENAI_API_KEY")
            && !key.is_empty()
        {
            return Some(Self::whisper(key.clone()));
        }
        None
    }

    /// Standalone transcription — no TelegramVoiceHandler needed.
    /// Sends audio bytes to the configured STT provider and returns text.
    pub async fn transcribe(&self, audio_bytes: &[u8], mime_type: &str) -> zeus_core::Result<String> {
        use reqwest::multipart;

        let http = reqwest::Client::new();
        let ext = mime_to_extension(mime_type);
        let filename = format!("voice.{}", ext);

        let file_part = multipart::Part::bytes(audio_bytes.to_vec())
            .file_name(filename)
            .mime_str(mime_type)
            .map_err(|e| zeus_core::Error::Channel(format!("Invalid MIME for multipart: {}", e)))?;

        let (url, auth, model) = match self {
            SttProvider::Whisper { api_key, api_url } => {
                (api_url.clone(), Some(api_key.clone()), Some("whisper-1"))
            }
            SttProvider::Groq { api_key } => {
                ("https://api.groq.com/openai/v1/audio/transcriptions".to_string(),
                 Some(api_key.clone()), Some("whisper-large-v3-turbo"))
            }
            SttProvider::WhisperCpp { api_url } => {
                (format!("{}/inference", api_url.trim_end_matches('/')), None, None)
            }
        };

        let mut form = multipart::Form::new().part("file", file_part);
        if let Some(m) = model {
            form = form.text("model", m.to_string());
        }

        let mut req = http.post(&url);
        if let Some(key) = auth {
            req = req.bearer_auth(key);
        }

        let resp = req.multipart(form).send().await
            .map_err(|e| zeus_core::Error::Channel(format!("STT request failed: {}", e)))?;

        let status = resp.status();
        let body: serde_json::Value = resp.json().await
            .map_err(|e| zeus_core::Error::Channel(format!("STT response parse failed: {}", e)))?;

        if !status.is_success() {
            let err = body.pointer("/error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(zeus_core::Error::Channel(format!("STT error ({}): {}", status, err)));
        }

        let text = body.get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if text.is_empty() {
            return Err(zeus_core::Error::Channel("STT returned empty text".into()));
        }

        tracing::info!(chars = text.len(), "STT transcription complete");
        Ok(text)
    }
}

/// TTS provider for voice response synthesis
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TtsProvider {
    /// OpenAI TTS API
    OpenAI {
        api_key: String,
        #[serde(default = "default_tts_model")]
        model: String,
        #[serde(default = "default_tts_voice")]
        voice: String,
    },
    /// Self-hosted Piper TTS server (returns base64 WAV)
    Piper {
        api_url: String,
        #[serde(default = "default_piper_voice")]
        voice: String,
    },
}

fn default_tts_model() -> String {
    "tts-1".to_string()
}

fn default_tts_voice() -> String {
    "nova".to_string()
}

fn default_piper_voice() -> String {
    "default".to_string()
}

impl TtsProvider {
    /// Create an OpenAI TTS provider with defaults
    pub fn openai(api_key: impl Into<String>) -> Self {
        Self::OpenAI {
            api_key: api_key.into(),
            model: default_tts_model(),
            voice: default_tts_voice(),
        }
    }

    /// Create an OpenAI TTS provider with custom model and voice
    pub fn openai_custom(
        api_key: impl Into<String>,
        model: impl Into<String>,
        voice: impl Into<String>,
    ) -> Self {
        Self::OpenAI {
            api_key: api_key.into(),
            model: model.into(),
            voice: voice.into(),
        }
    }

    /// Create a self-hosted Piper TTS provider
    pub fn piper(api_url: impl Into<String>) -> Self {
        Self::Piper {
            api_url: api_url.into(),
            voice: default_piper_voice(),
        }
    }

    /// Create a Piper TTS provider with custom voice
    pub fn piper_custom(api_url: impl Into<String>, voice: impl Into<String>) -> Self {
        Self::Piper {
            api_url: api_url.into(),
            voice: voice.into(),
        }
    }

    /// Create a TTS provider from Zeus config (single source of truth).
    ///
    /// Priority: `deployment.piper_tts_url` (self-hosted, no auth) >
    /// `credentials.OPENAI_API_KEY`. Returns `None` if none are set.
    pub fn from_config(config: &zeus_core::Config) -> Option<Self> {
        if let Some(url) = config.deployment.as_ref().map(|d| &d.piper_tts_url)
            && !url.is_empty()
        {
            return Some(Self::piper(url.clone()));
        }
        if let Some(key) = config.credentials.get("OPENAI_API_KEY")
            && !key.is_empty()
        {
            return Some(Self::openai(key.clone()));
        }
        None
    }
}

/// Configuration for Telegram voice message handling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramVoiceConfig {
    /// Enable voice message handling
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Whether to auto-reply with voice (if false, replies with text only)
    #[serde(default)]
    pub auto_voice_reply: bool,

    /// Maximum voice duration to process (seconds). Messages exceeding this are ignored.
    #[serde(default = "default_max_duration")]
    pub max_duration_secs: u32,

    /// STT provider for transcription (optional; resolved from env if absent)
    #[serde(default)]
    pub stt_provider: Option<SttProvider>,

    /// TTS provider for voice replies (optional; resolved from env if absent)
    #[serde(default)]
    pub tts_provider: Option<TtsProvider>,
}

fn default_true() -> bool {
    true
}

fn default_max_duration() -> u32 {
    120
}

impl Default for TelegramVoiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_voice_reply: false,
            max_duration_secs: default_max_duration(),
            stt_provider: None,
            tts_provider: None,
        }
    }
}

// ============================================================================
// Telegram voice update JSON structures
// ============================================================================

/// Minimal representation of a Telegram voice object from the Bot API
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TelegramVoice {
    /// Unique identifier for this file
    pub file_id: String,
    /// Unique identifier for this file (stable across re-uploads)
    #[serde(default)]
    pub file_unique_id: String,
    /// Duration of the audio in seconds
    #[serde(default)]
    pub duration: u32,
    /// MIME type (usually "audio/ogg")
    #[serde(default)]
    pub mime_type: Option<String>,
    /// File size in bytes
    #[serde(default)]
    pub file_size: Option<u64>,
}

/// Minimal representation of a Telegram audio object from the Bot API
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TelegramAudio {
    /// Unique identifier for this file
    pub file_id: String,
    /// Unique identifier for this file (stable across re-uploads)
    #[serde(default)]
    pub file_unique_id: String,
    /// Duration in seconds
    #[serde(default)]
    pub duration: u32,
    /// MIME type
    #[serde(default)]
    pub mime_type: Option<String>,
    /// File size in bytes
    #[serde(default)]
    pub file_size: Option<u64>,
}

/// Minimal representation of a Telegram video note (round video message)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TelegramVideoNote {
    /// Unique identifier for this file
    pub file_id: String,
    /// Unique identifier for this file (stable across re-uploads)
    #[serde(default)]
    pub file_unique_id: String,
    /// Duration in seconds
    #[serde(default)]
    pub duration: u32,
    /// Video width and height (diameter of the round video)
    #[serde(default)]
    pub length: u32,
    /// File size in bytes
    #[serde(default)]
    pub file_size: Option<u64>,
}

/// Result of voice message detection — what kind of voice content we found
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceMessageKind {
    /// Standard voice message (OGG/Opus)
    Voice {
        file_id: String,
        duration: u32,
        mime_type: String,
    },
    /// Audio file attachment
    Audio {
        file_id: String,
        duration: u32,
        mime_type: String,
    },
    /// Video note (round video) — audio track extracted
    VideoNote { file_id: String, duration: u32 },
}

impl VoiceMessageKind {
    pub fn file_id(&self) -> &str {
        match self {
            Self::Voice { file_id, .. } => file_id,
            Self::Audio { file_id, .. } => file_id,
            Self::VideoNote { file_id, .. } => file_id,
        }
    }

    pub fn duration(&self) -> u32 {
        match self {
            Self::Voice { duration, .. } => *duration,
            Self::Audio { duration, .. } => *duration,
            Self::VideoNote { duration, .. } => *duration,
        }
    }

    /// MIME type for the file (used for STT upload).
    /// Video notes don't have an explicit MIME; we assume OGG since Telegram re-encodes.
    pub fn mime_type(&self) -> &str {
        match self {
            Self::Voice { mime_type, .. } => mime_type,
            Self::Audio { mime_type, .. } => mime_type,
            Self::VideoNote { .. } => "audio/ogg",
        }
    }

    /// File extension appropriate for this kind
    pub fn extension(&self) -> &str {
        match self.mime_type() {
            "audio/ogg" => "ogg",
            "audio/mpeg" => "mp3",
            "audio/mp4" => "m4a",
            "audio/wav" | "audio/x-wav" => "wav",
            _ => "ogg",
        }
    }
}

/// Detect voice content from a Telegram Bot API message JSON object.
///
/// Checks for `voice`, `audio`, and `video_note` fields in priority order.
pub fn detect_voice_message(message: &serde_json::Value) -> Option<VoiceMessageKind> {
    // 1. Voice message (standard Telegram voice)
    if let Some(voice) = message.get("voice")
        && let Ok(v) = serde_json::from_value::<TelegramVoice>(voice.clone())
    {
        return Some(VoiceMessageKind::Voice {
            file_id: v.file_id,
            duration: v.duration,
            mime_type: v.mime_type.unwrap_or_else(|| "audio/ogg".to_string()),
        });
    }

    // 2. Audio file
    if let Some(audio) = message.get("audio")
        && let Ok(a) = serde_json::from_value::<TelegramAudio>(audio.clone())
    {
        return Some(VoiceMessageKind::Audio {
            file_id: a.file_id,
            duration: a.duration,
            mime_type: a.mime_type.unwrap_or_else(|| "audio/mpeg".to_string()),
        });
    }

    // 3. Video note (round video)
    if let Some(vn) = message.get("video_note")
        && let Ok(v) = serde_json::from_value::<TelegramVideoNote>(vn.clone())
    {
        return Some(VoiceMessageKind::VideoNote {
            file_id: v.file_id,
            duration: v.duration,
        });
    }

    None
}

// ============================================================================
// Telegram Voice Handler
// ============================================================================

/// Handles the full voice-message lifecycle for Telegram:
/// download -> transcribe -> (agent processes text) -> synthesize -> send voice reply
pub struct TelegramVoiceHandler {
    bot_token: String,
    http: reqwest::Client,
    stt_provider: SttProvider,
    tts_provider: Option<TtsProvider>,
    config: TelegramVoiceConfig,
}

impl TelegramVoiceHandler {
    /// Create a new handler
    pub fn new(
        bot_token: impl Into<String>,
        stt_provider: SttProvider,
        tts_provider: Option<TtsProvider>,
        config: TelegramVoiceConfig,
    ) -> Self {
        Self {
            bot_token: bot_token.into(),
            http: reqwest::Client::new(),
            stt_provider,
            tts_provider,
            config,
        }
    }

    /// Create from resolved Zeus config.
    ///
    /// STT/TTS providers are resolved from `zeus_core::Config` (single source of truth).
    /// Returns an error if no STT provider is configured.
    pub fn from_config(
        bot_token: impl Into<String>,
        zeus_config: &zeus_core::Config,
        config: TelegramVoiceConfig,
    ) -> Result<Self> {
        let stt = config
            .stt_provider
            .clone()
            .or_else(|| SttProvider::from_config(zeus_config))
            .ok_or_else(|| {
                Error::Channel(
                    "Telegram voice: no STT provider configured (set deployment.whisper_stt_url, credentials.GROQ_API_KEY, or credentials.OPENAI_API_KEY in config.toml)"
                        .to_string(),
                )
            })?;

        let tts = config
            .tts_provider
            .clone()
            .or_else(|| TtsProvider::from_config(zeus_config));

        Ok(Self::new(bot_token, stt, tts, config))
    }

    /// Check whether a voice message should be processed (enabled + within duration limit)
    pub fn should_process(&self, kind: &VoiceMessageKind) -> bool {
        if !self.config.enabled {
            return false;
        }
        kind.duration() <= self.config.max_duration_secs
    }

    /// Whether voice replies are enabled (auto_voice_reply + TTS provider available)
    pub fn can_reply_with_voice(&self) -> bool {
        self.config.auto_voice_reply && self.tts_provider.is_some()
    }

    // ------------------------------------------------------------------------
    // Download
    // ------------------------------------------------------------------------

    /// Build the Telegram Bot API URL for `getFile`
    fn get_file_url(&self, file_id: &str) -> String {
        format!(
            "https://api.telegram.org/bot{}/getFile?file_id={}",
            self.bot_token, file_id
        )
    }

    /// Build the Telegram file download URL from a file_path
    fn download_url(&self, file_path: &str) -> String {
        format!(
            "https://api.telegram.org/file/bot{}/{}",
            self.bot_token, file_path
        )
    }

    /// Download a voice file from Telegram by file_id.
    ///
    /// Steps:
    /// 1. Call `getFile` to resolve the file_path
    /// 2. Download the raw bytes from the file_path URL
    pub async fn download_voice_file(&self, file_id: &str) -> Result<Vec<u8>> {
        // Step 1: Resolve file_path
        let url = self.get_file_url(file_id);
        debug!(file_id = %file_id, "Requesting Telegram getFile");

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Telegram getFile request failed: {}", e)))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Telegram getFile parse failed: {}", e)))?;

        if !status.is_success() || body.get("ok") != Some(&serde_json::Value::Bool(true)) {
            let description = body
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("unknown error");
            return Err(Error::Channel(format!(
                "Telegram getFile failed ({}): {}",
                status, description
            )));
        }

        let file_path = body
            .pointer("/result/file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Error::Channel("Telegram getFile response missing file_path".to_string())
            })?;

        // Step 2: Download the file
        let download_url = self.download_url(file_path);
        debug!(file_path = %file_path, "Downloading Telegram voice file");

        let file_resp = self
            .http
            .get(&download_url)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Telegram file download failed: {}", e)))?;

        if !file_resp.status().is_success() {
            return Err(Error::Channel(format!(
                "Telegram file download HTTP {}: {}",
                file_resp.status(),
                download_url
            )));
        }

        let bytes = file_resp
            .bytes()
            .await
            .map_err(|e| Error::Channel(format!("Telegram file download read failed: {}", e)))?;

        info!(
            file_id = %file_id,
            size = bytes.len(),
            "Downloaded Telegram voice file"
        );

        Ok(bytes.to_vec())
    }

    // ------------------------------------------------------------------------
    // Transcribe (STT)
    // ------------------------------------------------------------------------

    /// Transcribe audio bytes to text using the configured STT provider.
    pub async fn transcribe(&self, audio_bytes: &[u8], mime_type: &str) -> Result<String> {
        match &self.stt_provider {
            SttProvider::Whisper { api_key, api_url } => {
                self.transcribe_whisper(audio_bytes, mime_type, api_key, api_url)
                    .await
            }
            SttProvider::Groq { api_key } => {
                self.transcribe_groq(audio_bytes, mime_type, api_key).await
            }
            SttProvider::WhisperCpp { api_url } => {
                self.transcribe_whisper_cpp(audio_bytes, mime_type, api_url)
                    .await
            }
        }
    }

    /// Transcribe via OpenAI-compatible Whisper API
    async fn transcribe_whisper(
        &self,
        audio_bytes: &[u8],
        mime_type: &str,
        api_key: &str,
        api_url: &str,
    ) -> Result<String> {
        let ext = mime_to_extension(mime_type);
        let filename = format!("voice.{}", ext);

        let file_part = multipart::Part::bytes(audio_bytes.to_vec())
            .file_name(filename)
            .mime_str(mime_type)
            .map_err(|e| Error::Channel(format!("Invalid MIME type for multipart: {}", e)))?;

        let form = multipart::Form::new()
            .part("file", file_part)
            .text("model", "whisper-1");

        debug!("Transcribing via Whisper API at {}", api_url);

        let resp = self
            .http
            .post(api_url)
            .bearer_auth(api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Whisper API request failed: {}", e)))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Whisper API response parse failed: {}", e)))?;

        if !status.is_success() {
            let error_msg = body
                .pointer("/error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(Error::Channel(format!(
                "Whisper API error ({}): {}",
                status, error_msg
            )));
        }

        let text = body
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if text.is_empty() {
            return Err(Error::Channel(
                "Whisper transcription returned empty text".into(),
            ));
        }

        info!(chars = text.len(), "Whisper transcription complete");
        Ok(text)
    }

    /// Transcribe via Groq Whisper API
    async fn transcribe_groq(
        &self,
        audio_bytes: &[u8],
        mime_type: &str,
        api_key: &str,
    ) -> Result<String> {
        let ext = mime_to_extension(mime_type);
        let filename = format!("voice.{}", ext);

        let file_part = multipart::Part::bytes(audio_bytes.to_vec())
            .file_name(filename)
            .mime_str(mime_type)
            .map_err(|e| Error::Channel(format!("Invalid MIME type for multipart: {}", e)))?;

        let form = multipart::Form::new()
            .part("file", file_part)
            .text("model", "whisper-large-v3");

        let url = "https://api.groq.com/openai/v1/audio/transcriptions";
        debug!("Transcribing via Groq Whisper API");

        let resp = self
            .http
            .post(url)
            .bearer_auth(api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Groq API request failed: {}", e)))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Groq API response parse failed: {}", e)))?;

        if !status.is_success() {
            let error_msg = body
                .pointer("/error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(Error::Channel(format!(
                "Groq STT API error ({}): {}",
                status, error_msg
            )));
        }

        let text = body
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if text.is_empty() {
            return Err(Error::Channel(
                "Groq transcription returned empty text".into(),
            ));
        }

        info!(chars = text.len(), "Groq transcription complete");
        Ok(text)
    }

    /// Transcribe via self-hosted whisper.cpp server (no auth)
    async fn transcribe_whisper_cpp(
        &self,
        audio_bytes: &[u8],
        mime_type: &str,
        api_url: &str,
    ) -> Result<String> {
        let ext = mime_to_extension(mime_type);
        let filename = format!("voice.{}", ext);

        let file_part = multipart::Part::bytes(audio_bytes.to_vec())
            .file_name(filename)
            .mime_str(mime_type)
            .map_err(|e| Error::Channel(format!("Invalid MIME type for multipart: {}", e)))?;

        let form = multipart::Form::new()
            .part("file", file_part)
            .text("response_format", "json");

        let url = format!("{}/inference", api_url.trim_end_matches('/'));
        debug!("Transcribing via whisper.cpp at {}", url);

        let resp = self
            .http
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("whisper.cpp request failed: {}", e)))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("whisper.cpp response parse failed: {}", e)))?;

        if !status.is_success() {
            let error_msg = body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(Error::Channel(format!(
                "whisper.cpp error ({}): {}",
                status, error_msg
            )));
        }

        let text = body
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if text.is_empty() {
            return Err(Error::Channel(
                "whisper.cpp transcription returned empty text".into(),
            ));
        }

        info!(chars = text.len(), "whisper.cpp transcription complete");
        Ok(text)
    }

    // ------------------------------------------------------------------------
    // Synthesize (TTS)
    // ------------------------------------------------------------------------

    /// Synthesize text into OGG/Opus audio bytes suitable for Telegram voice messages.
    ///
    /// Returns `None` if no TTS provider is configured.
    pub async fn synthesize(&self, text: &str) -> Result<Option<Vec<u8>>> {
        let provider = match &self.tts_provider {
            Some(p) => p,
            None => {
                debug!("No TTS provider configured; skipping synthesis");
                return Ok(None);
            }
        };

        match provider {
            TtsProvider::OpenAI {
                api_key,
                model,
                voice,
            } => {
                let audio = self.synthesize_openai(text, api_key, model, voice).await?;
                Ok(Some(audio))
            }
            TtsProvider::Piper { api_url, voice } => {
                let audio = self.synthesize_piper(text, api_url, voice).await?;
                Ok(Some(audio))
            }
        }
    }

    /// Synthesize via OpenAI TTS API, requesting Opus output
    async fn synthesize_openai(
        &self,
        text: &str,
        api_key: &str,
        model: &str,
        voice: &str,
    ) -> Result<Vec<u8>> {
        let url = "https://api.openai.com/v1/audio/speech";

        let body = serde_json::json!({
            "model": model,
            "input": text,
            "voice": voice,
            "response_format": "opus"
        });

        debug!(model = %model, voice = %voice, "Synthesizing via OpenAI TTS");

        let resp = self
            .http
            .post(url)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("OpenAI TTS request failed: {}", e)))?;

        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "OpenAI TTS error ({}): {}",
                status, err_body
            )));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| Error::Channel(format!("OpenAI TTS response read failed: {}", e)))?;

        info!(size = bytes.len(), "OpenAI TTS synthesis complete");
        Ok(bytes.to_vec())
    }

    /// Synthesize via self-hosted Piper TTS server.
    ///
    /// Piper returns JSON with `audio_base64` (base64-encoded WAV).
    /// We decode it and return the raw WAV bytes.
    async fn synthesize_piper(&self, text: &str, api_url: &str, voice: &str) -> Result<Vec<u8>> {
        let url = format!("{}/synthesize", api_url.trim_end_matches('/'));

        let mut body = serde_json::json!({
            "text": text,
            "speed": 1.0
        });
        if voice != "default" && !voice.is_empty() {
            body["voice"] = serde_json::json!(voice);
        }

        debug!(voice = %voice, "Synthesizing via Piper TTS at {}", url);

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Piper TTS request failed: {}", e)))?;

        let status = resp.status();
        let resp_body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Channel(format!("Piper TTS response parse failed: {}", e)))?;

        if !status.is_success() {
            let detail = resp_body
                .get("detail")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(Error::Channel(format!(
                "Piper TTS error ({}): {}",
                status, detail
            )));
        }

        let audio_b64 = resp_body
            .get("audio_base64")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Channel("Piper TTS response missing audio_base64".to_string()))?;

        let audio_bytes = base64::engine::general_purpose::STANDARD
            .decode(audio_b64)
            .map_err(|e| Error::Channel(format!("Piper TTS base64 decode failed: {}", e)))?;

        info!(size = audio_bytes.len(), "Piper TTS synthesis complete");
        Ok(audio_bytes)
    }

    // ------------------------------------------------------------------------
    // Send voice reply
    // ------------------------------------------------------------------------

    /// Send a voice message to a Telegram chat via the Bot API `sendVoice` endpoint.
    pub async fn send_voice(
        &self,
        chat_id: &str,
        audio: &[u8],
        caption: Option<&str>,
    ) -> Result<()> {
        let url = format!("https://api.telegram.org/bot{}/sendVoice", self.bot_token);

        let voice_part = multipart::Part::bytes(audio.to_vec())
            .file_name("reply.ogg")
            .mime_str("audio/ogg")
            .map_err(|e| Error::Channel(format!("sendVoice multipart error: {}", e)))?;

        let mut form = multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("voice", voice_part);

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        debug!(chat_id = %chat_id, "Sending Telegram voice reply");

        let resp = self
            .http
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("sendVoice request failed: {}", e)))?;

        let status = resp.status();
        if !status.is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let description = body
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("unknown error");
            return Err(Error::Channel(format!(
                "sendVoice failed ({}): {}",
                status, description
            )));
        }

        info!(chat_id = %chat_id, "Voice reply sent successfully");
        Ok(())
    }

    /// Send an audio file to a Telegram chat via `sendAudio` (for WAV/non-OGG formats).
    pub async fn send_audio(
        &self,
        chat_id: &str,
        audio: &[u8],
        caption: Option<&str>,
    ) -> Result<()> {
        let url = format!("https://api.telegram.org/bot{}/sendAudio", self.bot_token);

        let audio_part = multipart::Part::bytes(audio.to_vec())
            .file_name("reply.wav")
            .mime_str("audio/wav")
            .map_err(|e| Error::Channel(format!("sendAudio multipart error: {}", e)))?;

        let mut form = multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("audio", audio_part);

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        debug!(chat_id = %chat_id, "Sending Telegram audio reply");

        let resp = self
            .http
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("sendAudio request failed: {}", e)))?;

        let status = resp.status();
        if !status.is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let description = body
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("unknown error");
            return Err(Error::Channel(format!(
                "sendAudio failed ({}): {}",
                status, description
            )));
        }

        info!(chat_id = %chat_id, "Audio reply sent successfully");
        Ok(())
    }

    // ------------------------------------------------------------------------
    // Full pipeline convenience
    // ------------------------------------------------------------------------

    /// Full voice-message processing pipeline:
    /// 1. Download voice file
    /// 2. Transcribe to text
    /// 3. Return the transcribed text (caller handles agent processing)
    ///
    /// This is the inbound half. The caller should:
    /// - Feed the text through the agent loop
    /// - Call `synthesize()` + `send_voice()` for the response if `can_reply_with_voice()`
    pub async fn process_inbound(&self, kind: &VoiceMessageKind) -> Result<String> {
        if !self.should_process(kind) {
            return Err(Error::Channel(format!(
                "Voice message exceeds max duration ({} > {} secs)",
                kind.duration(),
                self.config.max_duration_secs
            )));
        }

        // Download
        let audio_bytes = self.download_voice_file(kind.file_id()).await?;

        // Transcribe
        let text = self.transcribe(&audio_bytes, kind.mime_type()).await?;

        if text.is_empty() {
            warn!(file_id = %kind.file_id(), "Transcription returned empty text");
            return Err(Error::Channel(
                "Voice message transcription returned empty text".to_string(),
            ));
        }

        info!(
            file_id = %kind.file_id(),
            duration = kind.duration(),
            chars = text.len(),
            "Voice message transcribed"
        );

        Ok(text)
    }

    /// Process the outbound response: synthesize + send voice reply.
    ///
    /// If TTS is not configured or `auto_voice_reply` is false, this is a no-op
    /// and returns `Ok(false)`.
    pub async fn process_outbound(&self, chat_id: &str, response_text: &str) -> Result<bool> {
        if !self.can_reply_with_voice() {
            return Ok(false);
        }

        let audio = self.synthesize(response_text).await?;
        if let Some(audio_bytes) = audio {
            // Piper returns WAV — use sendAudio. OpenAI returns OGG/Opus — use sendVoice.
            let is_piper = matches!(&self.tts_provider, Some(TtsProvider::Piper { .. }));
            if is_piper {
                self.send_audio(chat_id, &audio_bytes, None).await?;
            } else {
                self.send_voice(chat_id, &audio_bytes, None).await?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Map a MIME type to a file extension for STT multipart uploads
fn mime_to_extension(mime_type: &str) -> &str {
    match mime_type {
        "audio/ogg" => "ogg",
        "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/m4a" => "m4a",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/webm" => "webm",
        "audio/flac" => "flac",
        _ => "ogg",
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- Config tests --------------------------------------------------------

    #[test]
    fn test_config_default() {
        let config = TelegramVoiceConfig::default();
        assert!(config.enabled);
        assert!(!config.auto_voice_reply);
        assert_eq!(config.max_duration_secs, 120);
        assert!(config.stt_provider.is_none());
        assert!(config.tts_provider.is_none());
    }

    #[test]
    fn test_config_serialization() {
        let config = TelegramVoiceConfig {
            enabled: true,
            auto_voice_reply: true,
            max_duration_secs: 60,
            stt_provider: Some(SttProvider::groq("gsk_test")),
            tts_provider: Some(TtsProvider::openai("sk_test")),
        };
        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        let parsed: TelegramVoiceConfig =
            serde_json::from_str(&json).expect("should parse successfully");
        assert!(parsed.enabled);
        assert!(parsed.auto_voice_reply);
        assert_eq!(parsed.max_duration_secs, 60);
        assert!(parsed.stt_provider.is_some());
        assert!(parsed.tts_provider.is_some());
    }

    #[test]
    fn test_config_deserialize_minimal() {
        let json = r#"{"enabled":false}"#;
        let config: TelegramVoiceConfig =
            serde_json::from_str(json).expect("should parse successfully");
        assert!(!config.enabled);
        assert!(!config.auto_voice_reply);
        assert_eq!(config.max_duration_secs, 120);
    }

    // -- SttProvider tests ---------------------------------------------------

    #[test]
    fn test_stt_whisper_creation() {
        let stt = SttProvider::whisper("sk-test-key");
        match &stt {
            SttProvider::Whisper { api_key, api_url } => {
                assert_eq!(api_key, "sk-test-key");
                assert!(api_url.contains("openai.com"));
            }
            _ => panic!("Expected Whisper variant"),
        }
    }

    #[test]
    fn test_stt_whisper_custom_url() {
        let stt = SttProvider::whisper_custom("key", "https://custom.api.com/transcribe");
        match &stt {
            SttProvider::Whisper { api_url, .. } => {
                assert_eq!(api_url, "https://custom.api.com/transcribe");
            }
            _ => panic!("Expected Whisper variant"),
        }
    }

    #[test]
    fn test_stt_groq_creation() {
        let stt = SttProvider::groq("gsk_test");
        match &stt {
            SttProvider::Groq { api_key } => {
                assert_eq!(api_key, "gsk_test");
            }
            _ => panic!("Expected Groq variant"),
        }
    }

    #[test]
    fn test_stt_provider_serialization() {
        let whisper = SttProvider::whisper("sk-key");
        let json = serde_json::to_string(&whisper).expect("should serialize to JSON");
        let parsed: SttProvider = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed, whisper);

        let groq = SttProvider::groq("gsk-key");
        let json = serde_json::to_string(&groq).expect("should serialize to JSON");
        let parsed: SttProvider = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed, groq);
    }

    // -- TtsProvider tests ---------------------------------------------------

    #[test]
    fn test_tts_openai_creation() {
        let tts = TtsProvider::openai("sk-test");
        match &tts {
            TtsProvider::OpenAI {
                api_key,
                model,
                voice,
            } => {
                assert_eq!(api_key, "sk-test");
                assert_eq!(model, "tts-1");
                assert_eq!(voice, "nova");
            }
            _ => panic!("Expected OpenAI variant"),
        }
    }

    #[test]
    fn test_tts_openai_custom() {
        let tts = TtsProvider::openai_custom("key", "tts-1-hd", "alloy");
        match &tts {
            TtsProvider::OpenAI { model, voice, .. } => {
                assert_eq!(model, "tts-1-hd");
                assert_eq!(voice, "alloy");
            }
            _ => panic!("Expected OpenAI variant"),
        }
    }

    #[test]
    fn test_tts_provider_serialization() {
        let tts = TtsProvider::openai("sk-key");
        let json = serde_json::to_string(&tts).expect("should serialize to JSON");
        let parsed: TtsProvider = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed, tts);
    }

    // -- Voice detection tests -----------------------------------------------

    #[test]
    fn test_detect_voice_message() {
        let msg = json!({
            "voice": {
                "file_id": "AwACAgIAAxkBAAI",
                "file_unique_id": "AgADdQAD",
                "duration": 5,
                "mime_type": "audio/ogg",
                "file_size": 12345
            }
        });

        let kind = detect_voice_message(&msg);
        assert!(kind.is_some());
        let kind = kind.expect("operation should succeed");
        assert_eq!(kind.file_id(), "AwACAgIAAxkBAAI");
        assert_eq!(kind.duration(), 5);
        assert_eq!(kind.mime_type(), "audio/ogg");
        assert!(matches!(kind, VoiceMessageKind::Voice { .. }));
    }

    #[test]
    fn test_detect_audio_message() {
        let msg = json!({
            "audio": {
                "file_id": "CQACAgIAAxkBAAJ",
                "file_unique_id": "AgADeQAD",
                "duration": 180,
                "mime_type": "audio/mpeg",
                "file_size": 2_000_000
            }
        });

        let kind = detect_voice_message(&msg);
        assert!(kind.is_some());
        let kind = kind.expect("operation should succeed");
        assert_eq!(kind.file_id(), "CQACAgIAAxkBAAJ");
        assert_eq!(kind.duration(), 180);
        assert_eq!(kind.mime_type(), "audio/mpeg");
        assert!(matches!(kind, VoiceMessageKind::Audio { .. }));
    }

    #[test]
    fn test_detect_video_note_message() {
        let msg = json!({
            "video_note": {
                "file_id": "DQACAgIAAxkBAAK",
                "file_unique_id": "AgADfQAD",
                "duration": 10,
                "length": 240,
                "file_size": 500_000
            }
        });

        let kind = detect_voice_message(&msg);
        assert!(kind.is_some());
        let kind = kind.expect("operation should succeed");
        assert_eq!(kind.file_id(), "DQACAgIAAxkBAAK");
        assert_eq!(kind.duration(), 10);
        assert_eq!(kind.mime_type(), "audio/ogg");
        assert!(matches!(kind, VoiceMessageKind::VideoNote { .. }));
    }

    #[test]
    fn test_detect_voice_priority_over_audio() {
        // When both voice and audio are present, voice should win
        let msg = json!({
            "voice": {
                "file_id": "voice_id",
                "file_unique_id": "vu",
                "duration": 3
            },
            "audio": {
                "file_id": "audio_id",
                "file_unique_id": "au",
                "duration": 300
            }
        });

        let kind = detect_voice_message(&msg).expect("operation should succeed");
        assert_eq!(kind.file_id(), "voice_id");
        assert!(matches!(kind, VoiceMessageKind::Voice { .. }));
    }

    #[test]
    fn test_detect_no_voice_content() {
        let msg = json!({
            "text": "Hello, world!",
            "photo": []
        });

        assert!(detect_voice_message(&msg).is_none());
    }

    #[test]
    fn test_detect_voice_no_mime_type_defaults() {
        let msg = json!({
            "voice": {
                "file_id": "test_id",
                "file_unique_id": "tu",
                "duration": 2
            }
        });

        let kind = detect_voice_message(&msg).expect("operation should succeed");
        assert_eq!(kind.mime_type(), "audio/ogg");
    }

    #[test]
    fn test_detect_audio_no_mime_defaults_to_mpeg() {
        let msg = json!({
            "audio": {
                "file_id": "test_id",
                "file_unique_id": "tu",
                "duration": 60
            }
        });

        let kind = detect_voice_message(&msg).expect("operation should succeed");
        assert_eq!(kind.mime_type(), "audio/mpeg");
    }

    // -- VoiceMessageKind tests ----------------------------------------------

    #[test]
    fn test_voice_kind_extension_ogg() {
        let kind = VoiceMessageKind::Voice {
            file_id: "x".to_string(),
            duration: 1,
            mime_type: "audio/ogg".to_string(),
        };
        assert_eq!(kind.extension(), "ogg");
    }

    #[test]
    fn test_voice_kind_extension_mp3() {
        let kind = VoiceMessageKind::Audio {
            file_id: "x".to_string(),
            duration: 1,
            mime_type: "audio/mpeg".to_string(),
        };
        assert_eq!(kind.extension(), "mp3");
    }

    #[test]
    fn test_voice_kind_extension_m4a() {
        let kind = VoiceMessageKind::Audio {
            file_id: "x".to_string(),
            duration: 1,
            mime_type: "audio/mp4".to_string(),
        };
        assert_eq!(kind.extension(), "m4a");
    }

    #[test]
    fn test_voice_kind_extension_wav() {
        let kind = VoiceMessageKind::Audio {
            file_id: "x".to_string(),
            duration: 1,
            mime_type: "audio/wav".to_string(),
        };
        assert_eq!(kind.extension(), "wav");
    }

    #[test]
    fn test_voice_kind_extension_unknown_defaults_ogg() {
        let kind = VoiceMessageKind::Audio {
            file_id: "x".to_string(),
            duration: 1,
            mime_type: "audio/something-weird".to_string(),
        };
        assert_eq!(kind.extension(), "ogg");
    }

    #[test]
    fn test_video_note_extension() {
        let kind = VoiceMessageKind::VideoNote {
            file_id: "x".to_string(),
            duration: 5,
        };
        assert_eq!(kind.extension(), "ogg");
        assert_eq!(kind.mime_type(), "audio/ogg");
    }

    // -- Handler tests -------------------------------------------------------

    #[test]
    fn test_handler_creation() {
        let handler = TelegramVoiceHandler::new(
            "123:ABC",
            SttProvider::groq("gsk_test"),
            Some(TtsProvider::openai("sk_test")),
            TelegramVoiceConfig::default(),
        );
        assert!(handler.can_reply_with_voice() || !handler.config.auto_voice_reply);
    }

    #[test]
    fn test_handler_should_process_enabled() {
        let handler = TelegramVoiceHandler::new(
            "123:ABC",
            SttProvider::groq("gsk_test"),
            None,
            TelegramVoiceConfig {
                enabled: true,
                max_duration_secs: 60,
                ..Default::default()
            },
        );

        let short = VoiceMessageKind::Voice {
            file_id: "f1".to_string(),
            duration: 30,
            mime_type: "audio/ogg".to_string(),
        };
        assert!(handler.should_process(&short));

        let long = VoiceMessageKind::Voice {
            file_id: "f2".to_string(),
            duration: 120,
            mime_type: "audio/ogg".to_string(),
        };
        assert!(!handler.should_process(&long));
    }

    #[test]
    fn test_handler_should_process_disabled() {
        let handler = TelegramVoiceHandler::new(
            "123:ABC",
            SttProvider::groq("gsk_test"),
            None,
            TelegramVoiceConfig {
                enabled: false,
                ..Default::default()
            },
        );

        let kind = VoiceMessageKind::Voice {
            file_id: "f1".to_string(),
            duration: 5,
            mime_type: "audio/ogg".to_string(),
        };
        assert!(!handler.should_process(&kind));
    }

    #[test]
    fn test_handler_should_process_exact_limit() {
        let handler = TelegramVoiceHandler::new(
            "123:ABC",
            SttProvider::groq("gsk_test"),
            None,
            TelegramVoiceConfig {
                enabled: true,
                max_duration_secs: 60,
                ..Default::default()
            },
        );

        let exactly_at_limit = VoiceMessageKind::Voice {
            file_id: "f".to_string(),
            duration: 60,
            mime_type: "audio/ogg".to_string(),
        };
        assert!(handler.should_process(&exactly_at_limit));

        let one_over = VoiceMessageKind::Voice {
            file_id: "f".to_string(),
            duration: 61,
            mime_type: "audio/ogg".to_string(),
        };
        assert!(!handler.should_process(&one_over));
    }

    #[test]
    fn test_handler_can_reply_with_voice_no_tts() {
        let handler = TelegramVoiceHandler::new(
            "123:ABC",
            SttProvider::groq("gsk_test"),
            None,
            TelegramVoiceConfig {
                auto_voice_reply: true,
                ..Default::default()
            },
        );
        // auto_voice_reply is true but no TTS provider
        assert!(!handler.can_reply_with_voice());
    }

    #[test]
    fn test_handler_can_reply_with_voice_no_auto() {
        let handler = TelegramVoiceHandler::new(
            "123:ABC",
            SttProvider::groq("gsk_test"),
            Some(TtsProvider::openai("sk_test")),
            TelegramVoiceConfig {
                auto_voice_reply: false,
                ..Default::default()
            },
        );
        // TTS available but auto_voice_reply is false
        assert!(!handler.can_reply_with_voice());
    }

    #[test]
    fn test_handler_can_reply_with_voice_both_enabled() {
        let handler = TelegramVoiceHandler::new(
            "123:ABC",
            SttProvider::groq("gsk_test"),
            Some(TtsProvider::openai("sk_test")),
            TelegramVoiceConfig {
                auto_voice_reply: true,
                ..Default::default()
            },
        );
        assert!(handler.can_reply_with_voice());
    }

    // -- URL construction tests ----------------------------------------------

    #[test]
    fn test_get_file_url() {
        let handler = TelegramVoiceHandler::new(
            "123:ABCDEF",
            SttProvider::groq("key"),
            None,
            TelegramVoiceConfig::default(),
        );

        let url = handler.get_file_url("file_12345");
        assert_eq!(
            url,
            "https://api.telegram.org/bot123:ABCDEF/getFile?file_id=file_12345"
        );
    }

    #[test]
    fn test_download_url() {
        let handler = TelegramVoiceHandler::new(
            "123:ABCDEF",
            SttProvider::groq("key"),
            None,
            TelegramVoiceConfig::default(),
        );

        let url = handler.download_url("voice/file_0.oga");
        assert_eq!(
            url,
            "https://api.telegram.org/file/bot123:ABCDEF/voice/file_0.oga"
        );
    }

    // -- mime_to_extension tests ---------------------------------------------

    #[test]
    fn test_mime_to_extension() {
        assert_eq!(mime_to_extension("audio/ogg"), "ogg");
        assert_eq!(mime_to_extension("audio/mpeg"), "mp3");
        assert_eq!(mime_to_extension("audio/mp4"), "m4a");
        assert_eq!(mime_to_extension("audio/m4a"), "m4a");
        assert_eq!(mime_to_extension("audio/wav"), "wav");
        assert_eq!(mime_to_extension("audio/x-wav"), "wav");
        assert_eq!(mime_to_extension("audio/webm"), "webm");
        assert_eq!(mime_to_extension("audio/flac"), "flac");
        assert_eq!(mime_to_extension("application/octet-stream"), "ogg");
    }

    // -- Telegram JSON structure tests ---------------------------------------

    #[test]
    fn test_telegram_voice_deserialization() {
        let json = json!({
            "file_id": "AwACAgIAAxkBAAI",
            "file_unique_id": "AgADdQAD",
            "duration": 5,
            "mime_type": "audio/ogg",
            "file_size": 12345
        });

        let voice: TelegramVoice = serde_json::from_value(json).expect("should deserialize");
        assert_eq!(voice.file_id, "AwACAgIAAxkBAAI");
        assert_eq!(voice.duration, 5);
        assert_eq!(voice.mime_type, Some("audio/ogg".to_string()));
        assert_eq!(voice.file_size, Some(12345));
    }

    #[test]
    fn test_telegram_voice_minimal_deserialization() {
        let json = json!({
            "file_id": "abc123"
        });

        let voice: TelegramVoice = serde_json::from_value(json).expect("should deserialize");
        assert_eq!(voice.file_id, "abc123");
        assert_eq!(voice.duration, 0);
        assert!(voice.mime_type.is_none());
        assert!(voice.file_size.is_none());
    }

    #[test]
    fn test_telegram_audio_deserialization() {
        let json = json!({
            "file_id": "CQACAgIAAxkBAAJ",
            "file_unique_id": "AgADeQAD",
            "duration": 180,
            "mime_type": "audio/mpeg",
            "file_size": 2_000_000
        });

        let audio: TelegramAudio = serde_json::from_value(json).expect("should deserialize");
        assert_eq!(audio.file_id, "CQACAgIAAxkBAAJ");
        assert_eq!(audio.duration, 180);
        assert_eq!(audio.mime_type, Some("audio/mpeg".to_string()));
    }

    #[test]
    fn test_telegram_video_note_deserialization() {
        let json = json!({
            "file_id": "DQACAgIAAxkBAAK",
            "file_unique_id": "AgADfQAD",
            "duration": 10,
            "length": 240,
            "file_size": 500_000
        });

        let vn: TelegramVideoNote = serde_json::from_value(json).expect("should deserialize");
        assert_eq!(vn.file_id, "DQACAgIAAxkBAAK");
        assert_eq!(vn.duration, 10);
        assert_eq!(vn.length, 240);
    }

    // -- Full message JSON detection (realistic Telegram Bot API payloads) ----

    #[test]
    fn test_detect_from_full_update_voice() {
        let update = json!({
            "update_id": 123456789,
            "message": {
                "message_id": 100,
                "from": {
                    "id": 12345,
                    "first_name": "Test",
                    "is_bot": false
                },
                "chat": {
                    "id": 12345,
                    "type": "private"
                },
                "date": 1700000000,
                "voice": {
                    "file_id": "AwACAgIAAxkBAAIBSmXY",
                    "file_unique_id": "AgADdQAD_abc",
                    "duration": 3,
                    "mime_type": "audio/ogg",
                    "file_size": 9876
                }
            }
        });

        let message = update.get("message").expect("key should exist");
        let kind = detect_voice_message(message);
        assert!(kind.is_some());
        let kind = kind.expect("operation should succeed");
        assert!(matches!(kind, VoiceMessageKind::Voice { .. }));
        assert_eq!(kind.duration(), 3);
    }

    #[test]
    fn test_detect_from_text_only_message() {
        let update = json!({
            "update_id": 123456790,
            "message": {
                "message_id": 101,
                "from": {
                    "id": 12345,
                    "first_name": "Test",
                    "is_bot": false
                },
                "chat": {
                    "id": 12345,
                    "type": "private"
                },
                "date": 1700000001,
                "text": "Just a regular text message"
            }
        });

        let message = update.get("message").expect("key should exist");
        assert!(detect_voice_message(message).is_none());
    }

    // -- Error case tests (duration limits) ----------------------------------

    #[tokio::test]
    async fn test_process_inbound_exceeds_duration() {
        let handler = TelegramVoiceHandler::new(
            "123:ABC",
            SttProvider::groq("gsk_test"),
            None,
            TelegramVoiceConfig {
                enabled: true,
                max_duration_secs: 10,
                ..Default::default()
            },
        );

        let kind = VoiceMessageKind::Voice {
            file_id: "file_too_long".to_string(),
            duration: 30,
            mime_type: "audio/ogg".to_string(),
        };

        let result = handler.process_inbound(&kind).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("exceeds max duration"));
    }

    #[tokio::test]
    async fn test_process_outbound_no_voice_reply() {
        let handler = TelegramVoiceHandler::new(
            "123:ABC",
            SttProvider::groq("gsk_test"),
            None,
            TelegramVoiceConfig {
                auto_voice_reply: false,
                ..Default::default()
            },
        );

        // Should return Ok(false) — no voice reply sent
        let result = handler.process_outbound("12345", "Hello").await;
        assert!(result.is_ok());
        assert!(!result.expect("operation should succeed"));
    }

    #[tokio::test]
    async fn test_process_outbound_auto_reply_no_tts() {
        let handler = TelegramVoiceHandler::new(
            "123:ABC",
            SttProvider::groq("gsk_test"),
            None, // no TTS provider
            TelegramVoiceConfig {
                auto_voice_reply: true,
                ..Default::default()
            },
        );

        // auto_voice_reply true but no TTS provider => Ok(false)
        let result = handler.process_outbound("12345", "Hello").await;
        assert!(result.is_ok());
        assert!(!result.expect("operation should succeed"));
    }
    // -- WhisperCpp provider tests -------------------------------------------

    #[test]
    fn test_stt_whisper_cpp_creation() {
        let stt = SttProvider::whisper_cpp("http://localhost:8080");
        match &stt {
            SttProvider::WhisperCpp { api_url } => {
                assert_eq!(api_url, "http://localhost:8080");
            }
            _ => panic!("Expected WhisperCpp variant"),
        }
    }

    #[test]
    fn test_stt_whisper_cpp_serialization() {
        let stt = SttProvider::whisper_cpp("https://wsp.example.com");
        let json = serde_json::to_string(&stt).expect("should serialize");
        let parsed: SttProvider = serde_json::from_str(&json).expect("should parse");
        assert_eq!(parsed, stt);
    }

    // -- Piper TTS provider tests --------------------------------------------

    #[test]
    fn test_tts_piper_creation() {
        let tts = TtsProvider::piper("http://localhost:8104");
        match &tts {
            TtsProvider::Piper { api_url, voice } => {
                assert_eq!(api_url, "http://localhost:8104");
                assert_eq!(voice, "default");
            }
            _ => panic!("Expected Piper variant"),
        }
    }

    #[test]
    fn test_tts_piper_custom() {
        let tts = TtsProvider::piper_custom("https://kk.example.com", "en_US-libritts-high");
        match &tts {
            TtsProvider::Piper { api_url, voice } => {
                assert_eq!(api_url, "https://kk.example.com");
                assert_eq!(voice, "en_US-libritts-high");
            }
            _ => panic!("Expected Piper variant"),
        }
    }

    #[test]
    fn test_tts_piper_serialization() {
        let tts = TtsProvider::piper("https://kk.example.com");
        let json = serde_json::to_string(&tts).expect("should serialize");
        let parsed: TtsProvider = serde_json::from_str(&json).expect("should parse");
        assert_eq!(parsed, tts);
    }

    // -- Handler with WhisperCpp + Piper tests --------------------------------

    #[test]
    fn test_handler_with_whisper_cpp() {
        let handler = TelegramVoiceHandler::new(
            "123:ABC",
            SttProvider::whisper_cpp("http://localhost:8080"),
            Some(TtsProvider::piper("http://localhost:8104")),
            TelegramVoiceConfig {
                auto_voice_reply: true,
                ..Default::default()
            },
        );
        assert!(handler.can_reply_with_voice());
        let kind = VoiceMessageKind::Voice {
            file_id: "f".to_string(),
            duration: 5,
            mime_type: "audio/ogg".to_string(),
        };
        assert!(handler.should_process(&kind));
    }

    #[test]
    fn test_handler_piper_is_piper_check() {
        let handler = TelegramVoiceHandler::new(
            "123:ABC",
            SttProvider::whisper_cpp("http://localhost:8080"),
            Some(TtsProvider::piper("http://localhost:8104")),
            TelegramVoiceConfig {
                auto_voice_reply: true,
                ..Default::default()
            },
        );
        assert!(matches!(
            &handler.tts_provider,
            Some(TtsProvider::Piper { .. })
        ));
    }
}
