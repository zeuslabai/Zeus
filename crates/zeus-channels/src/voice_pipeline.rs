//! Voice Pipeline - Whisper STT + Piper TTS Integration
//!
//! Provides speech-to-text and text-to-speech capabilities using
//! Whisper (STT) and Piper (TTS) HTTP services.

use serde::{Deserialize, Serialize};
use zeus_core::Result;

/// Voice pipeline configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoicePipelineConfig {
    /// Whisper STT service URL
    pub whisper_url: String,
    /// Piper TTS service URL
    pub piper_url: String,
    /// Default voice for TTS (e.g., "en_US-lessac-medium")
    #[serde(default = "default_voice")]
    pub default_voice: String,
    /// Audio format for TTS output
    #[serde(default = "default_format")]
    pub audio_format: String,
}

fn default_voice() -> String {
    "en_US-lessac-medium".to_string()
}

fn default_format() -> String {
    "wav".to_string()
}

impl Default for VoicePipelineConfig {
    fn default() -> Self {
        Self {
            whisper_url: std::env::var("ZEUS_WHISPER_URL")
                .unwrap_or_else(|_| "http://localhost:8000".to_string()),
            piper_url: std::env::var("ZEUS_PIPER_URL")
                .unwrap_or_else(|_| "http://localhost:8001".to_string()),
            default_voice: default_voice(),
            audio_format: default_format(),
        }
    }
}

/// Voice pipeline for STT and TTS
pub struct VoicePipeline {
    config: VoicePipelineConfig,
    client: reqwest::Client,
}

impl VoicePipeline {
    /// Create a new voice pipeline
    pub fn new(config: VoicePipelineConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    /// Transcribe audio bytes to text using Whisper
    pub async fn transcribe(&self, audio_bytes: &[u8]) -> Result<String> {
        let url = format!("{}/transcribe", self.config.whisper_url);

        let form = reqwest::multipart::Form::new().part(
            "file",
            reqwest::multipart::Part::bytes(audio_bytes.to_vec())
                .file_name("audio.wav")
                .mime_str("audio/wav")
                .map_err(|e| zeus_core::Error::channel(format!("Invalid MIME type: {}", e)))?,
        );

        let response = self
            .client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| zeus_core::Error::channel(format!("Whisper request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(zeus_core::Error::channel(format!(
                "Whisper returned error: {}",
                response.status()
            )));
        }

        let result: TranscriptionResult = response
            .json()
            .await
            .map_err(|e| zeus_core::Error::channel(format!("Failed to parse response: {}", e)))?;

        Ok(result.text)
    }

    /// Synthesize text to audio bytes using Piper
    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        self.synthesize_with_voice(text, &self.config.default_voice)
            .await
    }

    /// Synthesize text with a specific voice
    pub async fn synthesize_with_voice(&self, text: &str, voice: &str) -> Result<Vec<u8>> {
        let url = format!("{}/synthesize", self.config.piper_url);

        let request = SynthesisRequest {
            text: text.to_string(),
            voice: voice.to_string(),
            format: self.config.audio_format.clone(),
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| zeus_core::Error::channel(format!("Piper request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(zeus_core::Error::channel(format!(
                "Piper returned error: {}",
                response.status()
            )));
        }

        let audio_bytes = response
            .bytes()
            .await
            .map_err(|e| zeus_core::Error::channel(format!("Failed to read audio: {}", e)))?;

        Ok(audio_bytes.to_vec())
    }

    /// Check if Whisper service is available
    pub async fn check_whisper(&self) -> bool {
        let url = format!("{}/health", self.config.whisper_url);
        self.client.get(&url).send().await.is_ok()
    }

    /// Check if Piper service is available
    pub async fn check_piper(&self) -> bool {
        let url = format!("{}/health", self.config.piper_url);
        self.client.get(&url).send().await.is_ok()
    }
}

#[derive(Debug, Deserialize)]
struct TranscriptionResult {
    text: String,
}

#[derive(Debug, Serialize)]
struct SynthesisRequest {
    text: String,
    voice: String,
    format: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        // Default reads ZEUS_WHISPER_URL / ZEUS_PIPER_URL env vars with localhost fallback.
        // Only assert env-independent fields here; URL defaults depend on env state.
        let config = VoicePipelineConfig::default();
        assert!(!config.whisper_url.is_empty());
        assert!(!config.piper_url.is_empty());
        assert_eq!(config.default_voice, "en_US-lessac-medium");
        assert_eq!(config.audio_format, "wav");
    }

    #[test]
    fn test_config_custom() {
        let config = VoicePipelineConfig {
            whisper_url: "http://whisper:9000".to_string(),
            piper_url: "http://piper:9001".to_string(),
            default_voice: "en_GB-alan-medium".to_string(),
            audio_format: "mp3".to_string(),
        };
        assert_eq!(config.whisper_url, "http://whisper:9000");
        assert_eq!(config.default_voice, "en_GB-alan-medium");
    }

    #[test]
    fn test_pipeline_creation() {
        let config = VoicePipelineConfig::default();
        let pipeline = VoicePipeline::new(config.clone());
        assert_eq!(pipeline.config.whisper_url, config.whisper_url);
    }

    #[tokio::test]
    async fn test_transcribe_empty_audio() {
        let pipeline = VoicePipeline::new(VoicePipelineConfig::default());
        let result = pipeline.transcribe(&[]).await;
        // Should fail with connection error since no real service is running
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_synthesize_empty_text() {
        let pipeline = VoicePipeline::new(VoicePipelineConfig::default());
        let result = pipeline.synthesize("").await;
        // Should fail with connection error since no real service is running
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_health_check_whisper() {
        // Use an unreachable URL so the health check always fails regardless of env vars
        let config = VoicePipelineConfig {
            whisper_url: "http://127.0.0.1:1".to_string(),
            ..VoicePipelineConfig::default()
        };
        let pipeline = VoicePipeline::new(config);
        let is_healthy = pipeline.check_whisper().await;
        assert!(!is_healthy);
    }

    #[tokio::test]
    async fn test_health_check_piper() {
        // Use an unreachable URL so the health check always fails regardless of env vars
        let config = VoicePipelineConfig {
            piper_url: "http://127.0.0.1:1".to_string(),
            ..VoicePipelineConfig::default()
        };
        let pipeline = VoicePipeline::new(config);
        // Should return false since no service is running
        let is_healthy = pipeline.check_piper().await;
        assert!(!is_healthy);
    }
}
