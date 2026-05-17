//! Zeus TTS - Modular text-to-speech providers for Zeus
//!
//! Provides a unified interface for multiple TTS backends:
//! - **ElevenLabs** - High-quality neural TTS via ElevenLabs API
//! - **OpenAI** - OpenAI TTS (tts-1 / tts-1-hd) with 6 voices
//! - **Edge TTS** - Free Microsoft Edge TTS (requires `edge-tts` CLI)
//! - **Local (Piper)** - Offline TTS via Piper subprocess
//! - **Piper HTTP** - Remote Piper TTS via HTTP (configurable `piper_url`)
//!
//! The [`TTSManager`] routes synthesis requests to the appropriate provider,
//! handling default provider/voice resolution and provider registration.

pub mod edge;
pub mod elevenlabs;
pub mod local;
pub mod openai;
pub mod piper;

use std::collections::HashMap;
use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during TTS operations.
#[derive(Debug, thiserror::Error)]
pub enum TTSError {
    #[error("provider not found: {0}")]
    ProviderNotFound(String),

    #[error("provider not configured: {0}")]
    ProviderNotConfigured(String),

    #[error("synthesis failed: {0}")]
    SynthesisFailed(String),

    #[error("network error: {0}")]
    NetworkError(String),

    #[error("invalid voice: {0}")]
    InvalidVoice(String),

    #[error("audio format error: {0}")]
    AudioFormatError(String),
}

// ---------------------------------------------------------------------------
// Audio format
// ---------------------------------------------------------------------------

/// Supported audio output formats.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    Wav,
    #[default]
    Mp3,
    Opus,
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioFormat::Wav => write!(f, "wav"),
            AudioFormat::Mp3 => write!(f, "mp3"),
            AudioFormat::Opus => write!(f, "opus"),
        }
    }
}

// ---------------------------------------------------------------------------
// Voice
// ---------------------------------------------------------------------------

/// Metadata describing a single voice offered by a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Voice {
    /// Unique voice identifier (provider-specific).
    pub id: String,
    /// Human-friendly display name.
    pub name: String,
    /// Optional gender label (e.g. "male", "female", "neutral").
    pub gender: Option<String>,
    /// BCP-47 language code (e.g. "en-US").
    pub language: Option<String>,
    /// URL to a sample/preview clip, if available.
    pub preview_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Request / Response
// ---------------------------------------------------------------------------

/// A request to synthesize speech.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TTSRequest {
    /// The text to synthesize.
    pub text: String,
    /// Target provider id. `None` means use the manager default.
    pub provider: Option<String>,
    /// Voice id. `None` means use the manager or provider default.
    pub voice: Option<String>,
    /// Playback speed multiplier (1.0 = normal).
    #[serde(default = "default_speed")]
    pub speed: f32,
    /// Desired audio output format.
    #[serde(default)]
    pub format: AudioFormat,
}

fn default_speed() -> f32 {
    1.0
}

impl TTSRequest {
    /// Create a minimal request with just text, using all defaults.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            provider: None,
            voice: None,
            speed: 1.0,
            format: AudioFormat::default(),
        }
    }
}

/// The result of a successful synthesis.
#[derive(Debug, Clone)]
pub struct TTSResponse {
    /// Raw audio bytes.
    pub audio: Vec<u8>,
    /// Format of the audio data.
    pub format: AudioFormat,
    /// Estimated duration in milliseconds, if known.
    pub duration_ms: Option<u64>,
    /// Which provider produced this audio.
    pub provider: String,
    /// Which voice was used.
    pub voice: String,
}

// ---------------------------------------------------------------------------
// Provider status
// ---------------------------------------------------------------------------

/// Summary status for a registered provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub id: String,
    pub name: String,
    pub configured: bool,
    pub voices_count: usize,
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// TTS subsystem configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TTSConfig {
    pub default_provider: Option<String>,
    pub default_voice: Option<String>,
    pub elevenlabs_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    /// URL of a remote Piper TTS server (env: `ZEUS_PIPER_URL`).
    /// When set, a `piper` provider is registered that synthesizes via HTTP.
    pub piper_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

/// Trait implemented by each TTS backend.
#[async_trait]
pub trait TTSProvider: Send + Sync {
    /// Short machine-readable identifier (e.g. "elevenlabs", "openai").
    fn id(&self) -> &str;

    /// Human-friendly provider name.
    fn name(&self) -> &str;

    /// Whether the provider has the credentials / config it needs.
    fn is_configured(&self) -> bool;

    /// List the voices available from this provider.
    async fn voices(&self) -> Result<Vec<Voice>, TTSError>;

    /// Synthesize `text` with the given parameters and return audio bytes.
    async fn synthesize(
        &self,
        text: &str,
        voice: &str,
        speed: f32,
        format: AudioFormat,
    ) -> Result<TTSResponse, TTSError>;
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

/// Central router that holds registered providers and dispatches requests.
pub struct TTSManager {
    providers: HashMap<String, Box<dyn TTSProvider>>,
    default_provider: Option<String>,
    default_voice: Option<String>,
}

impl TTSManager {
    /// Create an empty manager with no providers registered.
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            default_provider: None,
            default_voice: None,
        }
    }

    /// Create a manager pre-configured from a [`TTSConfig`].
    pub fn from_config(config: &TTSConfig) -> Self {
        let mut mgr = Self::new();
        mgr.default_provider = config.default_provider.clone();
        mgr.default_voice = config.default_voice.clone();

        if let Some(ref key) = config.elevenlabs_api_key {
            mgr.register_provider(Box::new(elevenlabs::ElevenLabsProvider::new(key.clone())));
        }
        if let Some(ref key) = config.openai_api_key {
            mgr.register_provider(Box::new(openai::OpenAIProvider::new(key.clone())));
        }
        if config.piper_url.is_some()
            || std::env::var("ZEUS_PIPER_URL")
                .ok()
                .filter(|s| !s.is_empty())
                .is_some()
        {
            mgr.register_provider(Box::new(piper::PiperHttpProvider::new(
                config.piper_url.clone(),
            )));
        }

        mgr
    }

    /// Register a provider. Replaces any existing provider with the same id.
    pub fn register_provider(&mut self, provider: Box<dyn TTSProvider>) {
        let id = provider.id().to_string();
        self.providers.insert(id, provider);
    }

    /// Set the default provider id used when a request omits it.
    pub fn set_default_provider(&mut self, id: impl Into<String>) {
        self.default_provider = Some(id.into());
    }

    /// Set the default voice used when a request omits it.
    pub fn set_default_voice(&mut self, voice: impl Into<String>) {
        self.default_voice = Some(voice.into());
    }

    /// Return status info for every registered provider.
    pub fn providers(&self) -> Vec<ProviderStatus> {
        self.providers
            .values()
            .map(|p| ProviderStatus {
                id: p.id().to_string(),
                name: p.name().to_string(),
                configured: p.is_configured(),
                voices_count: 0, // Populated asynchronously via voices()
            })
            .collect()
    }

    /// Return the number of registered providers.
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    /// List voices for a specific provider.
    pub async fn voices(&self, provider_id: &str) -> Result<Vec<Voice>, TTSError> {
        let provider = self
            .providers
            .get(provider_id)
            .ok_or_else(|| TTSError::ProviderNotFound(provider_id.to_string()))?;
        provider.voices().await
    }

    /// Synthesize speech, routing to the appropriate provider.
    ///
    /// Resolution order for provider:
    /// 1. `request.provider` if set
    /// 2. `self.default_provider` if set
    /// 3. First registered provider
    ///
    /// Resolution order for voice:
    /// 1. `request.voice` if set
    /// 2. `self.default_voice` if set
    /// 3. Provider decides (typically its own default)
    pub async fn synthesize(&self, request: TTSRequest) -> Result<TTSResponse, TTSError> {
        // Resolve provider
        let provider_id = request
            .provider
            .as_deref()
            .or(self.default_provider.as_deref())
            .or_else(|| self.providers.keys().next().map(|s| s.as_str()));

        let provider_id = provider_id
            .ok_or_else(|| TTSError::ProviderNotFound("no providers registered".to_string()))?;

        let provider = self
            .providers
            .get(provider_id)
            .ok_or_else(|| TTSError::ProviderNotFound(provider_id.to_string()))?;

        if !provider.is_configured() {
            return Err(TTSError::ProviderNotConfigured(provider_id.to_string()));
        }

        // Resolve voice
        let voice = request
            .voice
            .as_deref()
            .or(self.default_voice.as_deref())
            .unwrap_or("default");

        provider
            .synthesize(&request.text, voice, request.speed, request.format)
            .await
    }
}

impl Default for TTSManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::EdgeTTSProvider;
    use crate::elevenlabs::ElevenLabsProvider;
    use crate::local::LocalProvider;
    use crate::openai::OpenAIProvider;
    use crate::piper::PiperHttpProvider;

    // -- Helper: a fake in-memory provider for testing routing ---------------

    struct FakeProvider {
        fake_id: String,
        fake_name: String,
        configured: bool,
    }

    impl FakeProvider {
        fn new(id: &str, name: &str, configured: bool) -> Self {
            Self {
                fake_id: id.to_string(),
                fake_name: name.to_string(),
                configured,
            }
        }
    }

    #[async_trait]
    impl TTSProvider for FakeProvider {
        fn id(&self) -> &str {
            &self.fake_id
        }
        fn name(&self) -> &str {
            &self.fake_name
        }
        fn is_configured(&self) -> bool {
            self.configured
        }
        async fn voices(&self) -> Result<Vec<Voice>, TTSError> {
            Ok(vec![Voice {
                id: "fake-voice".to_string(),
                name: "Fake Voice".to_string(),
                gender: Some("neutral".to_string()),
                language: Some("en-US".to_string()),
                preview_url: None,
            }])
        }
        async fn synthesize(
            &self,
            text: &str,
            voice: &str,
            _speed: f32,
            format: AudioFormat,
        ) -> Result<TTSResponse, TTSError> {
            Ok(TTSResponse {
                audio: text.as_bytes().to_vec(),
                format,
                duration_ms: Some(1000),
                provider: self.fake_id.clone(),
                voice: voice.to_string(),
            })
        }
    }

    // -- Manager tests -------------------------------------------------------

    #[test]
    fn test_manager_creation() {
        let mgr = TTSManager::new();
        assert_eq!(mgr.provider_count(), 0);
        assert!(mgr.providers().is_empty());
    }

    #[test]
    fn test_manager_default_impl() {
        let mgr = TTSManager::default();
        assert_eq!(mgr.provider_count(), 0);
    }

    #[test]
    fn test_register_provider() {
        let mut mgr = TTSManager::new();
        mgr.register_provider(Box::new(FakeProvider::new("fake", "Fake TTS", true)));
        assert_eq!(mgr.provider_count(), 1);
    }

    #[test]
    fn test_provider_listing() {
        let mut mgr = TTSManager::new();
        mgr.register_provider(Box::new(FakeProvider::new("alpha", "Alpha TTS", true)));
        mgr.register_provider(Box::new(FakeProvider::new("beta", "Beta TTS", false)));
        assert_eq!(mgr.provider_count(), 2);
        let statuses = mgr.providers();
        assert_eq!(statuses.len(), 2);
        let ids: Vec<&str> = statuses.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"alpha"));
        assert!(ids.contains(&"beta"));
    }

    #[test]
    fn test_register_duplicate_provider_replaces() {
        let mut mgr = TTSManager::new();
        mgr.register_provider(Box::new(FakeProvider::new("dup", "First", true)));
        mgr.register_provider(Box::new(FakeProvider::new("dup", "Second", false)));
        assert_eq!(mgr.provider_count(), 1);
        let statuses = mgr.providers();
        assert_eq!(statuses[0].name, "Second");
    }

    #[test]
    fn test_multiple_providers_registered() {
        let mut mgr = TTSManager::new();
        mgr.register_provider(Box::new(FakeProvider::new("a", "A", true)));
        mgr.register_provider(Box::new(FakeProvider::new("b", "B", true)));
        mgr.register_provider(Box::new(FakeProvider::new("c", "C", true)));
        assert_eq!(mgr.provider_count(), 3);
    }

    #[tokio::test]
    async fn test_synthesize_routes_to_explicit_provider() {
        let mut mgr = TTSManager::new();
        mgr.register_provider(Box::new(FakeProvider::new("alpha", "Alpha", true)));
        mgr.register_provider(Box::new(FakeProvider::new("beta", "Beta", true)));

        let req = TTSRequest {
            text: "hello".to_string(),
            provider: Some("beta".to_string()),
            voice: Some("v1".to_string()),
            speed: 1.0,
            format: AudioFormat::Mp3,
        };
        let resp = mgr
            .synthesize(req)
            .await
            .expect("async operation should succeed");
        assert_eq!(resp.provider, "beta");
        assert_eq!(resp.voice, "v1");
    }

    #[tokio::test]
    async fn test_synthesize_uses_default_provider() {
        let mut mgr = TTSManager::new();
        mgr.register_provider(Box::new(FakeProvider::new("alpha", "Alpha", true)));
        mgr.register_provider(Box::new(FakeProvider::new("beta", "Beta", true)));
        mgr.set_default_provider("alpha");

        let req = TTSRequest::new("hello");
        let resp = mgr
            .synthesize(req)
            .await
            .expect("async operation should succeed");
        assert_eq!(resp.provider, "alpha");
    }

    #[tokio::test]
    async fn test_synthesize_uses_default_voice() {
        let mut mgr = TTSManager::new();
        mgr.register_provider(Box::new(FakeProvider::new("prov", "Prov", true)));
        mgr.set_default_voice("my-default-voice");

        let req = TTSRequest::new("hello");
        let resp = mgr
            .synthesize(req)
            .await
            .expect("async operation should succeed");
        assert_eq!(resp.voice, "my-default-voice");
    }

    #[tokio::test]
    async fn test_synthesize_missing_provider_error() {
        let mut mgr = TTSManager::new();
        mgr.register_provider(Box::new(FakeProvider::new("alpha", "Alpha", true)));

        let req = TTSRequest {
            text: "hello".to_string(),
            provider: Some("nonexistent".to_string()),
            voice: None,
            speed: 1.0,
            format: AudioFormat::Mp3,
        };
        let err = mgr.synthesize(req).await.unwrap_err();
        assert!(matches!(err, TTSError::ProviderNotFound(_)));
    }

    #[tokio::test]
    async fn test_synthesize_no_providers_error() {
        let mgr = TTSManager::new();
        let req = TTSRequest::new("hello");
        let err = mgr.synthesize(req).await.unwrap_err();
        assert!(matches!(err, TTSError::ProviderNotFound(_)));
    }

    #[tokio::test]
    async fn test_synthesize_unconfigured_provider_error() {
        let mut mgr = TTSManager::new();
        mgr.register_provider(Box::new(FakeProvider::new("bad", "Bad", false)));

        let req = TTSRequest::new("hello");
        let err = mgr.synthesize(req).await.unwrap_err();
        assert!(matches!(err, TTSError::ProviderNotConfigured(_)));
    }

    #[tokio::test]
    async fn test_voices_for_provider() {
        let mut mgr = TTSManager::new();
        mgr.register_provider(Box::new(FakeProvider::new("fake", "Fake", true)));

        let voices = mgr
            .voices("fake")
            .await
            .expect("async operation should succeed");
        assert_eq!(voices.len(), 1);
        assert_eq!(voices[0].id, "fake-voice");
    }

    #[tokio::test]
    async fn test_voices_unknown_provider() {
        let mgr = TTSManager::new();
        let err = mgr.voices("nope").await.unwrap_err();
        assert!(matches!(err, TTSError::ProviderNotFound(_)));
    }

    // -- TTSRequest tests ----------------------------------------------------

    #[test]
    fn test_request_new_defaults() {
        let req = TTSRequest::new("test text");
        assert_eq!(req.text, "test text");
        assert!(req.provider.is_none());
        assert!(req.voice.is_none());
        assert_eq!(req.speed, 1.0);
        assert_eq!(req.format, AudioFormat::Mp3);
    }

    #[test]
    fn test_request_with_all_fields() {
        let req = TTSRequest {
            text: "full request".to_string(),
            provider: Some("openai".to_string()),
            voice: Some("alloy".to_string()),
            speed: 1.5,
            format: AudioFormat::Opus,
        };
        assert_eq!(req.text, "full request");
        assert_eq!(req.provider.as_deref(), Some("openai"));
        assert_eq!(req.voice.as_deref(), Some("alloy"));
        assert_eq!(req.speed, 1.5);
        assert_eq!(req.format, AudioFormat::Opus);
    }

    #[test]
    fn test_request_serialization() {
        let req = TTSRequest::new("ser test");
        let json = serde_json::to_string(&req).expect("should serialize to JSON");
        let de: TTSRequest = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.text, "ser test");
        assert_eq!(de.speed, 1.0);
    }

    // -- AudioFormat tests ---------------------------------------------------

    #[test]
    fn test_audio_format_display() {
        assert_eq!(AudioFormat::Wav.to_string(), "wav");
        assert_eq!(AudioFormat::Mp3.to_string(), "mp3");
        assert_eq!(AudioFormat::Opus.to_string(), "opus");
    }

    #[test]
    fn test_audio_format_serialization() {
        let json = serde_json::to_string(&AudioFormat::Opus).expect("should serialize to JSON");
        assert_eq!(json, "\"opus\"");
        let de: AudioFormat = serde_json::from_str("\"wav\"").expect("should parse successfully");
        assert_eq!(de, AudioFormat::Wav);
    }

    #[test]
    fn test_audio_format_default() {
        assert_eq!(AudioFormat::default(), AudioFormat::Mp3);
    }

    // -- Voice tests ---------------------------------------------------------

    #[test]
    fn test_voice_serialization() {
        let voice = Voice {
            id: "v1".to_string(),
            name: "Voice One".to_string(),
            gender: Some("female".to_string()),
            language: Some("en-US".to_string()),
            preview_url: Some("https://example.com/preview.mp3".to_string()),
        };
        let json = serde_json::to_string(&voice).expect("should serialize to JSON");
        let de: Voice = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.id, "v1");
        assert_eq!(de.name, "Voice One");
        assert_eq!(de.gender.as_deref(), Some("female"));
        assert_eq!(de.language.as_deref(), Some("en-US"));
        assert!(de.preview_url.is_some());
    }

    #[test]
    fn test_voice_minimal() {
        let voice = Voice {
            id: "min".to_string(),
            name: "Minimal".to_string(),
            gender: None,
            language: None,
            preview_url: None,
        };
        let json = serde_json::to_string(&voice).expect("should serialize to JSON");
        assert!(json.contains("\"gender\":null"));
    }

    // -- ProviderStatus tests ------------------------------------------------

    #[test]
    fn test_provider_status_construction() {
        let ps = ProviderStatus {
            id: "el".to_string(),
            name: "ElevenLabs".to_string(),
            configured: true,
            voices_count: 42,
        };
        assert_eq!(ps.id, "el");
        assert!(ps.configured);
        assert_eq!(ps.voices_count, 42);
    }

    #[test]
    fn test_provider_status_serialization() {
        let ps = ProviderStatus {
            id: "openai".to_string(),
            name: "OpenAI".to_string(),
            configured: false,
            voices_count: 6,
        };
        let json = serde_json::to_string(&ps).expect("should serialize to JSON");
        let de: ProviderStatus = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.id, "openai");
        assert!(!de.configured);
        assert_eq!(de.voices_count, 6);
    }

    // -- TTSConfig tests -----------------------------------------------------

    #[test]
    fn test_config_default() {
        let cfg = TTSConfig::default();
        assert!(cfg.default_provider.is_none());
        assert!(cfg.default_voice.is_none());
        assert!(cfg.elevenlabs_api_key.is_none());
        assert!(cfg.openai_api_key.is_none());
        assert!(cfg.piper_url.is_none());
    }

    #[test]
    fn test_config_serialization() {
        let cfg = TTSConfig {
            default_provider: Some("openai".to_string()),
            default_voice: Some("nova".to_string()),
            elevenlabs_api_key: Some("el-key".to_string()),
            openai_api_key: Some("sk-key".to_string()),
            piper_url: Some("http://localhost:8104".to_string()),
        };
        let json = serde_json::to_string(&cfg).expect("should serialize to JSON");
        let de: TTSConfig = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.default_provider.as_deref(), Some("openai"));
        assert_eq!(de.default_voice.as_deref(), Some("nova"));
        assert_eq!(de.elevenlabs_api_key.as_deref(), Some("el-key"));
        assert_eq!(de.openai_api_key.as_deref(), Some("sk-key"));
        assert_eq!(de.piper_url.as_deref(), Some("http://localhost:8104"));
    }

    #[test]
    fn test_config_from_partial_json() {
        let json = r#"{"default_provider":"edge"}"#;
        let cfg: TTSConfig = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(cfg.default_provider.as_deref(), Some("edge"));
        assert!(cfg.default_voice.is_none());
    }

    // -- TTSError tests ------------------------------------------------------

    #[test]
    fn test_error_display_provider_not_found() {
        let e = TTSError::ProviderNotFound("foo".to_string());
        assert_eq!(e.to_string(), "provider not found: foo");
    }

    #[test]
    fn test_error_display_provider_not_configured() {
        let e = TTSError::ProviderNotConfigured("bar".to_string());
        assert_eq!(e.to_string(), "provider not configured: bar");
    }

    #[test]
    fn test_error_display_synthesis_failed() {
        let e = TTSError::SynthesisFailed("timeout".to_string());
        assert_eq!(e.to_string(), "synthesis failed: timeout");
    }

    #[test]
    fn test_error_display_network_error() {
        let e = TTSError::NetworkError("dns".to_string());
        assert_eq!(e.to_string(), "network error: dns");
    }

    #[test]
    fn test_error_display_invalid_voice() {
        let e = TTSError::InvalidVoice("bad-id".to_string());
        assert_eq!(e.to_string(), "invalid voice: bad-id");
    }

    #[test]
    fn test_error_display_audio_format_error() {
        let e = TTSError::AudioFormatError("unsupported".to_string());
        assert_eq!(e.to_string(), "audio format error: unsupported");
    }

    // -- Provider-specific unit tests ----------------------------------------

    #[test]
    fn test_elevenlabs_provider_new() {
        let p = ElevenLabsProvider::new("el-api-key-123".to_string());
        assert_eq!(p.id(), "elevenlabs");
        assert_eq!(p.name(), "ElevenLabs");
        assert!(p.is_configured());
    }

    #[test]
    fn test_elevenlabs_provider_empty_key_not_configured() {
        let p = ElevenLabsProvider::new(String::new());
        assert!(!p.is_configured());
    }

    #[test]
    fn test_openai_provider_new() {
        let p = OpenAIProvider::new("sk-test-key".to_string());
        assert_eq!(p.id(), "openai");
        assert_eq!(p.name(), "OpenAI TTS");
        assert!(p.is_configured());
    }

    #[test]
    fn test_openai_provider_empty_key_not_configured() {
        let p = OpenAIProvider::new(String::new());
        assert!(!p.is_configured());
    }

    #[tokio::test]
    async fn test_openai_static_voices() {
        let p = OpenAIProvider::new("sk-test".to_string());
        let voices = p.voices().await.expect("async operation should succeed");
        assert_eq!(voices.len(), 6);
        let ids: Vec<&str> = voices.iter().map(|v| v.id.as_str()).collect();
        assert!(ids.contains(&"alloy"));
        assert!(ids.contains(&"echo"));
        assert!(ids.contains(&"fable"));
        assert!(ids.contains(&"onyx"));
        assert!(ids.contains(&"nova"));
        assert!(ids.contains(&"shimmer"));
    }

    #[test]
    fn test_edge_provider_new() {
        let p = EdgeTTSProvider::new();
        assert_eq!(p.id(), "edge");
        assert_eq!(p.name(), "Microsoft Edge TTS");
        assert!(p.is_configured()); // No key needed
    }

    #[tokio::test]
    async fn test_edge_static_voices() {
        let p = EdgeTTSProvider::new();
        let voices = p.voices().await.expect("async operation should succeed");
        assert!(voices.len() >= 10);
        let ids: Vec<&str> = voices.iter().map(|v| v.id.as_str()).collect();
        assert!(ids.contains(&"en-US-GuyNeural"));
        assert!(ids.contains(&"en-US-JennyNeural"));
        assert!(ids.contains(&"en-GB-SoniaNeural"));
    }

    #[test]
    fn test_local_provider_new_default() {
        let p = LocalProvider::new(None);
        assert_eq!(p.id(), "local");
        assert_eq!(p.name(), "Local (Piper)");
    }

    #[test]
    fn test_local_provider_new_custom_path() {
        let p = LocalProvider::new(Some("/usr/local/bin/piper".to_string()));
        assert_eq!(p.id(), "local");
    }

    #[test]
    fn test_piper_http_provider_new() {
        let p = PiperHttpProvider::new(Some("http://localhost:8104".to_string()));
        assert_eq!(p.id(), "piper");
        assert_eq!(p.name(), "Piper (HTTP)");
        assert!(p.is_configured());
    }

    #[test]
    fn test_piper_http_provider_default_url() {
        // Clear ZEUS_PIPER_URL so we test the compiled-in default, not live config.
        temp_env::with_vars([("ZEUS_PIPER_URL", None::<&str>)], || {
            let p = PiperHttpProvider::new(None);
            assert_eq!(p.base_url(), "http://localhost:8104");
        });
    }

    #[test]
    fn test_local_provider_not_configured_by_default() {
        // Piper is unlikely to be installed in CI, so is_configured may be false.
        // We just verify the method does not panic.
        let p = LocalProvider::new(None);
        let _ = p.is_configured();
    }

    // -- Manager from_config tests -------------------------------------------

    #[test]
    fn test_manager_from_config_empty() {
        // Clear ZEUS_PIPER_URL so env var doesn't auto-register piper.
        temp_env::with_vars([("ZEUS_PIPER_URL", None::<&str>)], || {
            let cfg = TTSConfig::default();
            let mgr = TTSManager::from_config(&cfg);
            assert_eq!(mgr.provider_count(), 0);
        });
    }

    #[test]
    fn test_manager_from_config_with_openai_key() {
        // Clear ZEUS_PIPER_URL so env var doesn't auto-register piper alongside openai.
        temp_env::with_vars([("ZEUS_PIPER_URL", None::<&str>)], || {
            let cfg = TTSConfig {
                openai_api_key: Some("sk-test".to_string()),
                ..Default::default()
            };
            let mgr = TTSManager::from_config(&cfg);
            assert_eq!(mgr.provider_count(), 1);
            let statuses = mgr.providers();
            assert_eq!(statuses[0].id, "openai");
        });
    }

    #[test]
    fn test_manager_from_config_with_both_keys() {
        // Clear ZEUS_PIPER_URL so env var doesn't auto-register piper alongside elevenlabs+openai.
        temp_env::with_vars([("ZEUS_PIPER_URL", None::<&str>)], || {
            let cfg = TTSConfig {
                elevenlabs_api_key: Some("el-key".to_string()),
                openai_api_key: Some("sk-key".to_string()),
                default_provider: Some("openai".to_string()),
                default_voice: Some("nova".to_string()),
                piper_url: None,
            };
            let mgr = TTSManager::from_config(&cfg);
            assert_eq!(mgr.provider_count(), 2);
        });
    }

    #[test]
    fn test_manager_from_config_with_piper_url() {
        let cfg = TTSConfig {
            piper_url: Some("http://localhost:8104".to_string()),
            ..Default::default()
        };
        let mgr = TTSManager::from_config(&cfg);
        assert_eq!(mgr.provider_count(), 1);
        let statuses = mgr.providers();
        assert_eq!(statuses[0].id, "piper");
    }

    #[test]
    fn test_manager_from_config_all_providers() {
        let cfg = TTSConfig {
            elevenlabs_api_key: Some("el-key".to_string()),
            openai_api_key: Some("sk-key".to_string()),
            piper_url: Some("http://localhost:8104".to_string()),
            default_provider: Some("piper".to_string()),
            default_voice: None,
        };
        let mgr = TTSManager::from_config(&cfg);
        assert_eq!(mgr.provider_count(), 3);
    }

    // -- TTSResponse tests ---------------------------------------------------

    #[test]
    fn test_tts_response_construction() {
        let resp = TTSResponse {
            audio: vec![0u8; 100],
            format: AudioFormat::Wav,
            duration_ms: Some(2500),
            provider: "test".to_string(),
            voice: "v1".to_string(),
        };
        assert_eq!(resp.audio.len(), 100);
        assert_eq!(resp.format, AudioFormat::Wav);
        assert_eq!(resp.duration_ms, Some(2500));
        assert_eq!(resp.provider, "test");
        assert_eq!(resp.voice, "v1");
    }

    #[test]
    fn test_tts_response_no_duration() {
        let resp = TTSResponse {
            audio: vec![],
            format: AudioFormat::Opus,
            duration_ms: None,
            provider: "x".to_string(),
            voice: "y".to_string(),
        };
        assert!(resp.duration_ms.is_none());
    }
}
