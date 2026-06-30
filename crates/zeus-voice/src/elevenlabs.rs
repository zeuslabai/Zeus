//! ElevenLabs TTS Provider
//!
//! Implements the `VoiceTTSProvider` trait using the ElevenLabs v1 text-to-speech API.
//!
//! Features:
//! - Buffered synthesis (`/v1/text-to-speech/{voice_id}`)
//! - Streaming synthesis (`/v1/text-to-speech/{voice_id}/stream`)
//! - Configurable voice, model, stability, and similarity boost
//! - Reads `ELEVENLABS_API_KEY` from environment

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use zeus_core::Result;

use crate::agent_loop::VoiceTTSProvider;

/// Default ElevenLabs voice ID — "Rachel"
const DEFAULT_VOICE_ID: &str = "21m00Tcm4TlvDq8ikWAM";

/// Default ElevenLabs model
const DEFAULT_MODEL_ID: &str = "eleven_multilingual_v2";

/// ElevenLabs API base URL
const API_BASE: &str = "https://api.elevenlabs.io";

/// Configuration for the ElevenLabs TTS provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElevenLabsConfig {
    /// API key (can also be set via `ELEVENLABS_API_KEY` env var)
    #[serde(default)]
    pub api_key: Option<String>,

    /// Voice ID to use (default: Rachel — `21m00Tcm4TlvDq8ikWAM`)
    #[serde(default = "default_voice_id")]
    pub voice_id: String,

    /// Model ID (default: `eleven_multilingual_v2`)
    #[serde(default = "default_model_id")]
    pub model_id: String,

    /// Voice stability (0.0 - 1.0). Higher = more consistent, lower = more expressive.
    #[serde(default = "default_stability")]
    pub stability: f32,

    /// Similarity boost (0.0 - 1.0). Higher = closer to original voice.
    #[serde(default = "default_similarity_boost")]
    pub similarity_boost: f32,

    /// Style exaggeration (0.0 - 1.0). Experimental.
    #[serde(default)]
    pub style: f32,

    /// Use speaker boost. Enhances voice clarity.
    #[serde(default = "default_speaker_boost")]
    pub use_speaker_boost: bool,

    /// Output format (default: mp3_44100_128)
    #[serde(default = "default_output_format")]
    pub output_format: String,
}

fn default_voice_id() -> String {
    DEFAULT_VOICE_ID.to_string()
}

fn default_model_id() -> String {
    DEFAULT_MODEL_ID.to_string()
}

fn default_stability() -> f32 {
    0.5
}

fn default_similarity_boost() -> f32 {
    0.75
}

fn default_speaker_boost() -> bool {
    true
}

fn default_output_format() -> String {
    "pcm_16000".to_string()
}

impl Default for ElevenLabsConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            voice_id: default_voice_id(),
            model_id: default_model_id(),
            stability: default_stability(),
            similarity_boost: default_similarity_boost(),
            style: 0.0,
            use_speaker_boost: default_speaker_boost(),
            output_format: default_output_format(),
        }
    }
}

impl ElevenLabsConfig {
    /// Resolve the API key from config or environment
    pub fn resolve_api_key(&self) -> Option<String> {
        self.api_key
            .clone()
            .or_else(|| std::env::var("ELEVENLABS_API_KEY").ok())
    }
}

/// ElevenLabs TTS provider
pub struct ElevenLabsProvider {
    config: ElevenLabsConfig,
    client: reqwest::Client,
}

impl ElevenLabsProvider {
    /// Create a new ElevenLabs provider with the given config
    pub fn new(config: ElevenLabsConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Create from environment variables with defaults
    pub fn from_env() -> Self {
        Self::new(ElevenLabsConfig::default())
    }

    /// Get the resolved API key or return an error
    fn api_key(&self) -> Result<String> {
        self.config.resolve_api_key().ok_or_else(|| {
            zeus_core::Error::Internal(
                "ElevenLabs API key not found. Set ELEVENLABS_API_KEY or configure elevenlabs.api_key"
                    .to_string(),
            )
        })
    }

    /// Build the request body for TTS
    fn build_request_body(&self, text: &str) -> serde_json::Value {
        serde_json::json!({
            "text": text,
            "model_id": self.config.model_id,
            "voice_settings": {
                "stability": self.config.stability,
                "similarity_boost": self.config.similarity_boost,
                "style": self.config.style,
                "use_speaker_boost": self.config.use_speaker_boost,
            }
        })
    }

    /// Resolve voice_id: use the config voice_id, or if a name is passed that
    /// doesn't look like an ElevenLabs ID, fall back to config default.
    fn resolve_voice_id(&self, voice: &str) -> String {
        if voice.is_empty() || voice == "default" {
            self.config.voice_id.clone()
        } else {
            voice.to_string()
        }
    }

    /// Convert raw PCM 16-bit 16kHz mono bytes to WAV
    fn pcm_bytes_to_wav(pcm_data: &[u8], sample_rate: u32) -> Vec<u8> {
        let data_size = pcm_data.len() as u32;
        let file_size = 36 + data_size;

        let mut wav = Vec::with_capacity(44 + pcm_data.len());

        // RIFF header
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&file_size.to_le_bytes());
        wav.extend_from_slice(b"WAVE");

        // fmt chunk
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM format
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
        wav.extend_from_slice(&2u16.to_le_bytes()); // block align
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

        // data chunk
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        wav.extend_from_slice(pcm_data);

        wav
    }
}

#[async_trait]
impl VoiceTTSProvider for ElevenLabsProvider {
    async fn synthesize_wav(&self, text: &str, voice: &str) -> Result<Vec<u8>> {
        let api_key = self.api_key()?;
        let voice_id = self.resolve_voice_id(voice);

        let url = format!(
            "{}/v1/text-to-speech/{}?output_format={}",
            API_BASE, voice_id, self.config.output_format
        );

        let body = self.build_request_body(text);

        debug!(voice_id = %voice_id, model = %self.config.model_id, "ElevenLabs TTS request");

        let response = self
            .client
            .post(&url)
            .header("xi-api-key", &api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| zeus_core::Error::Internal(format!("ElevenLabs request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            return Err(zeus_core::Error::Internal(format!(
                "ElevenLabs API returned {}: {}",
                status, error_body
            )));
        }

        let audio_bytes = response
            .bytes()
            .await
            .map_err(|e| zeus_core::Error::Internal(format!("ElevenLabs read error: {}", e)))?;

        // If output format is PCM, wrap in WAV header
        if self.config.output_format.starts_with("pcm_") {
            let sample_rate = self
                .config
                .output_format
                .strip_prefix("pcm_")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(16000);
            Ok(Self::pcm_bytes_to_wav(&audio_bytes, sample_rate))
        } else {
            // Return raw audio (mp3/ogg) — caller needs to handle conversion
            Ok(audio_bytes.to_vec())
        }
    }

    async fn synthesize_stream(
        &self,
        text: &str,
        voice: &str,
    ) -> Result<mpsc::Receiver<Result<Vec<u8>>>> {
        let api_key = self.api_key()?;
        let voice_id = self.resolve_voice_id(voice);

        let url = format!(
            "{}/v1/text-to-speech/{}/stream?output_format={}",
            API_BASE, voice_id, self.config.output_format
        );

        let body = self.build_request_body(text);

        debug!(voice_id = %voice_id, "ElevenLabs streaming TTS request");

        let response = self
            .client
            .post(&url)
            .header("xi-api-key", &api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                zeus_core::Error::Internal(format!("ElevenLabs stream request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            return Err(zeus_core::Error::Internal(format!(
                "ElevenLabs stream API returned {}: {}",
                status, error_body
            )));
        }

        let (tx, rx) = mpsc::channel(32);
        let output_format = self.config.output_format.clone();

        tokio::spawn(async move {
            use futures_util::StreamExt;

            let mut stream = response.bytes_stream();
            let mut pcm_accumulator: Vec<u8> = Vec::new();
            let chunk_threshold = 3200; // ~100ms at 16kHz 16-bit mono

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        if output_format.starts_with("pcm_") {
                            pcm_accumulator.extend_from_slice(&chunk);

                            // Emit WAV chunks when we have enough PCM data
                            while pcm_accumulator.len() >= chunk_threshold {
                                let pcm_chunk: Vec<u8> =
                                    pcm_accumulator.drain(..chunk_threshold).collect();
                                let sample_rate = output_format
                                    .strip_prefix("pcm_")
                                    .and_then(|s| s.parse::<u32>().ok())
                                    .unwrap_or(16000);
                                let wav =
                                    ElevenLabsProvider::pcm_bytes_to_wav(&pcm_chunk, sample_rate);
                                if tx.send(Ok(wav)).await.is_err() {
                                    return;
                                }
                            }
                        } else {
                            // Non-PCM: pass chunks through directly
                            if tx.send(Ok(chunk.to_vec())).await.is_err() {
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        warn!("ElevenLabs stream chunk error: {}", e);
                        let _ = tx
                            .send(Err(zeus_core::Error::Internal(format!(
                                "Stream error: {}",
                                e
                            ))))
                            .await;
                        return;
                    }
                }
            }

            // Flush remaining PCM data
            if !pcm_accumulator.is_empty() && output_format.starts_with("pcm_") {
                let sample_rate = output_format
                    .strip_prefix("pcm_")
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(16000);
                let wav = ElevenLabsProvider::pcm_bytes_to_wav(&pcm_accumulator, sample_rate);
                let _ = tx.send(Ok(wav)).await;
            }
        });

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_elevenlabs_config_defaults() {
        let config = ElevenLabsConfig::default();
        assert_eq!(config.voice_id, DEFAULT_VOICE_ID);
        assert_eq!(config.model_id, DEFAULT_MODEL_ID);
        assert_eq!(config.stability, 0.5);
        assert_eq!(config.similarity_boost, 0.75);
        assert_eq!(config.style, 0.0);
        assert!(config.use_speaker_boost);
        assert_eq!(config.output_format, "pcm_16000");
        assert!(config.api_key.is_none());
    }

    #[test]
    fn test_elevenlabs_config_serialization() {
        let config = ElevenLabsConfig {
            api_key: Some("sk-test".to_string()),
            voice_id: "custom_voice_123".to_string(),
            model_id: "eleven_monolingual_v1".to_string(),
            stability: 0.8,
            similarity_boost: 0.9,
            style: 0.3,
            use_speaker_boost: false,
            output_format: "mp3_44100_128".to_string(),
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: ElevenLabsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.voice_id, "custom_voice_123");
        assert_eq!(parsed.model_id, "eleven_monolingual_v1");
        assert_eq!(parsed.stability, 0.8);
        assert_eq!(parsed.similarity_boost, 0.9);
        assert_eq!(parsed.style, 0.3);
        assert!(!parsed.use_speaker_boost);
        assert_eq!(parsed.output_format, "mp3_44100_128");
    }

    #[test]
    fn test_elevenlabs_config_deserialize_minimal() {
        let json = r#"{}"#;
        let config: ElevenLabsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.voice_id, DEFAULT_VOICE_ID);
        assert_eq!(config.model_id, DEFAULT_MODEL_ID);
        assert_eq!(config.stability, 0.5);
    }

    #[test]
    fn test_resolve_voice_id_default() {
        let provider = ElevenLabsProvider::new(ElevenLabsConfig::default());
        assert_eq!(provider.resolve_voice_id("default"), DEFAULT_VOICE_ID);
        assert_eq!(provider.resolve_voice_id(""), DEFAULT_VOICE_ID);
    }

    #[test]
    fn test_resolve_voice_id_custom() {
        let provider = ElevenLabsProvider::new(ElevenLabsConfig::default());
        assert_eq!(provider.resolve_voice_id("custom_id_123"), "custom_id_123");
    }

    #[test]
    fn test_build_request_body() {
        let config = ElevenLabsConfig {
            stability: 0.7,
            similarity_boost: 0.8,
            style: 0.1,
            use_speaker_boost: true,
            ..Default::default()
        };
        let provider = ElevenLabsProvider::new(config);
        let body = provider.build_request_body("Hello world");

        assert_eq!(body["text"], "Hello world");
        assert_eq!(body["model_id"], DEFAULT_MODEL_ID);
        let stability = body["voice_settings"]["stability"].as_f64().unwrap();
        assert!((stability - 0.7).abs() < 0.001, "stability: {}", stability);
        let sim_boost = body["voice_settings"]["similarity_boost"].as_f64().unwrap();
        assert!(
            (sim_boost - 0.8).abs() < 0.001,
            "similarity_boost: {}",
            sim_boost
        );
        let style = body["voice_settings"]["style"].as_f64().unwrap();
        assert!((style - 0.1).abs() < 0.001, "style: {}", style);
        assert_eq!(body["voice_settings"]["use_speaker_boost"], true);
    }

    #[test]
    fn test_pcm_bytes_to_wav() {
        let pcm = vec![0u8; 100];
        let wav = ElevenLabsProvider::pcm_bytes_to_wav(&pcm, 16000);

        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");

        let format_tag = u16::from_le_bytes([wav[20], wav[21]]);
        assert_eq!(format_tag, 1); // PCM

        let channels = u16::from_le_bytes([wav[22], wav[23]]);
        assert_eq!(channels, 1); // mono

        let sample_rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
        assert_eq!(sample_rate, 16000);

        assert_eq!(&wav[36..40], b"data");
        let data_size = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_size, 100);

        // PCM data should follow
        assert_eq!(&wav[44..], &pcm[..]);
    }

    #[test]
    fn test_pcm_bytes_to_wav_empty() {
        let wav = ElevenLabsProvider::pcm_bytes_to_wav(&[], 44100);
        assert_eq!(wav.len(), 44); // Header only
        let data_size = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_size, 0);
    }

    #[test]
    fn test_api_key_missing() {
        let provider = ElevenLabsProvider::new(ElevenLabsConfig::default());
        // Without env var or config, api_key() should error
        // (we can't reliably test env vars in unit tests without mutex)
        if std::env::var("ELEVENLABS_API_KEY").is_err() {
            let result = provider.api_key();
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("API key not found")
            );
        }
    }

    #[test]
    fn test_api_key_from_config() {
        let config = ElevenLabsConfig {
            api_key: Some("sk-test-key".to_string()),
            ..Default::default()
        };
        let provider = ElevenLabsProvider::new(config);
        let key = provider.api_key().unwrap();
        assert_eq!(key, "sk-test-key");
    }

    #[test]
    fn test_resolve_api_key_config_priority() {
        let config = ElevenLabsConfig {
            api_key: Some("from-config".to_string()),
            ..Default::default()
        };
        // Config key should take priority over env
        let resolved = config.resolve_api_key();
        assert_eq!(resolved, Some("from-config".to_string()));
    }

    #[test]
    fn test_from_env_constructor() {
        let provider = ElevenLabsProvider::from_env();
        assert_eq!(provider.config.voice_id, DEFAULT_VOICE_ID);
        assert_eq!(provider.config.model_id, DEFAULT_MODEL_ID);
    }
}
