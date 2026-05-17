//! ElevenLabs TTS provider
//!
//! Uses the ElevenLabs REST API for high-quality neural text-to-speech.
//! Requires an API key from <https://elevenlabs.io>.

use async_trait::async_trait;
use serde::Deserialize;
use tracing::instrument;

use crate::{AudioFormat, TTSError, TTSProvider, TTSResponse, Voice};

/// ElevenLabs TTS provider.
pub struct ElevenLabsProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl ElevenLabsProvider {
    /// Create a new provider with the given API key.
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.elevenlabs.io/v1".to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Create a provider with a custom base URL (useful for testing/proxies).
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            api_key,
            base_url,
            client: reqwest::Client::new(),
        }
    }
}

// ElevenLabs /voices response structures
#[derive(Deserialize)]
struct VoicesResponse {
    voices: Vec<ElevenLabsVoice>,
}

#[derive(Deserialize)]
struct ElevenLabsVoice {
    voice_id: String,
    name: String,
    #[serde(default)]
    labels: VoiceLabels,
    #[serde(default)]
    preview_url: Option<String>,
}

#[derive(Default, Deserialize)]
struct VoiceLabels {
    #[serde(default)]
    gender: Option<String>,
    #[serde(default)]
    accent: Option<String>,
}

#[async_trait]
impl TTSProvider for ElevenLabsProvider {
    fn id(&self) -> &str {
        "elevenlabs"
    }

    fn name(&self) -> &str {
        "ElevenLabs"
    }

    fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    #[instrument(skip(self), level = "debug")]
    async fn voices(&self) -> Result<Vec<Voice>, TTSError> {
        if !self.is_configured() {
            return Err(TTSError::ProviderNotConfigured("elevenlabs".to_string()));
        }

        let url = format!("{}/voices", self.base_url);
        let resp = self
            .client
            .get(&url)
            .header("xi-api-key", &self.api_key)
            .send()
            .await
            .map_err(|e| TTSError::NetworkError(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(TTSError::SynthesisFailed(format!(
                "ElevenLabs voices API returned status {}",
                resp.status()
            )));
        }

        let body: VoicesResponse = resp
            .json()
            .await
            .map_err(|e| TTSError::SynthesisFailed(format!("failed to parse voices: {e}")))?;

        Ok(body
            .voices
            .into_iter()
            .map(|v| Voice {
                id: v.voice_id,
                name: v.name,
                gender: v.labels.gender,
                language: v.labels.accent,
                preview_url: v.preview_url,
            })
            .collect())
    }

    #[instrument(skip(self, text), fields(text_len = text.len()), level = "debug")]
    async fn synthesize(
        &self,
        text: &str,
        voice: &str,
        _speed: f32,
        format: AudioFormat,
    ) -> Result<TTSResponse, TTSError> {
        if !self.is_configured() {
            return Err(TTSError::ProviderNotConfigured("elevenlabs".to_string()));
        }

        let output_format = match format {
            AudioFormat::Mp3 => "mp3_44100_128",
            AudioFormat::Wav => "pcm_44100",
            AudioFormat::Opus => "mp3_44100_128", // Fallback to mp3
        };

        let url = format!(
            "{}/text-to-speech/{}?output_format={}",
            self.base_url, voice, output_format
        );

        let body = serde_json::json!({
            "text": text,
            "model_id": "eleven_monolingual_v1",
            "voice_settings": {
                "stability": 0.5,
                "similarity_boost": 0.75
            }
        });

        let resp = self
            .client
            .post(&url)
            .header("xi-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| TTSError::NetworkError(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(TTSError::SynthesisFailed(format!(
                "ElevenLabs returned {status}: {text}"
            )));
        }

        let audio = resp
            .bytes()
            .await
            .map_err(|e| TTSError::NetworkError(e.to_string()))?
            .to_vec();

        Ok(TTSResponse {
            audio,
            format,
            duration_ms: None,
            provider: "elevenlabs".to_string(),
            voice: voice.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_custom_base_url() {
        let p = ElevenLabsProvider::with_base_url(
            "key".to_string(),
            "http://localhost:8080".to_string(),
        );
        assert_eq!(p.base_url, "http://localhost:8080");
        assert!(p.is_configured());
    }

    #[test]
    fn test_default_base_url() {
        let p = ElevenLabsProvider::new("test-key".to_string());
        assert_eq!(p.base_url, "https://api.elevenlabs.io/v1");
    }

    #[test]
    fn test_provider_id_and_name() {
        let p = ElevenLabsProvider::new("key".to_string());
        assert_eq!(p.id(), "elevenlabs");
        assert_eq!(p.name(), "ElevenLabs");
    }

    #[test]
    fn test_not_configured_with_empty_key() {
        let p = ElevenLabsProvider::new(String::new());
        assert!(!p.is_configured());
    }

    #[test]
    fn test_configured_with_key() {
        let p = ElevenLabsProvider::new("sk-test-key".to_string());
        assert!(p.is_configured());
    }

    #[tokio::test]
    async fn test_voices_fails_when_not_configured() {
        let p = ElevenLabsProvider::new(String::new());
        let result = p.voices().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            TTSError::ProviderNotConfigured(name) => assert_eq!(name, "elevenlabs"),
            other => panic!("Expected ProviderNotConfigured, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_synthesize_fails_when_not_configured() {
        let p = ElevenLabsProvider::new(String::new());
        let result = p
            .synthesize("hello", "voice-id", 1.0, AudioFormat::Mp3)
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            TTSError::ProviderNotConfigured(name) => assert_eq!(name, "elevenlabs"),
            other => panic!("Expected ProviderNotConfigured, got: {:?}", other),
        }
    }
}
