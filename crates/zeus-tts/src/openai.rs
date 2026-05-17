//! OpenAI TTS provider
//!
//! Uses the OpenAI Audio API (`/v1/audio/speech`) with the `tts-1` model.
//! Supports 6 built-in voices: alloy, echo, fable, onyx, nova, shimmer.

use async_trait::async_trait;
use tracing::instrument;

use crate::{AudioFormat, TTSError, TTSProvider, TTSResponse, Voice};

/// OpenAI TTS provider.
pub struct OpenAIProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAIProvider {
    /// Create a new provider with the given API key.
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Create a provider with a custom base URL (useful for proxies/testing).
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            api_key,
            base_url,
            client: reqwest::Client::new(),
        }
    }
}

/// Static list of OpenAI TTS voices.
const OPENAI_VOICES: &[(&str, &str, &str)] = &[
    ("alloy", "Alloy", "neutral"),
    ("echo", "Echo", "male"),
    ("fable", "Fable", "male"),
    ("onyx", "Onyx", "male"),
    ("nova", "Nova", "female"),
    ("shimmer", "Shimmer", "female"),
];

#[async_trait]
impl TTSProvider for OpenAIProvider {
    fn id(&self) -> &str {
        "openai"
    }

    fn name(&self) -> &str {
        "OpenAI TTS"
    }

    fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    async fn voices(&self) -> Result<Vec<Voice>, TTSError> {
        Ok(OPENAI_VOICES
            .iter()
            .map(|(id, name, gender)| Voice {
                id: id.to_string(),
                name: name.to_string(),
                gender: Some(gender.to_string()),
                language: Some("en-US".to_string()),
                preview_url: None,
            })
            .collect())
    }

    #[instrument(skip(self, text), fields(text_len = text.len()), level = "debug")]
    async fn synthesize(
        &self,
        text: &str,
        voice: &str,
        speed: f32,
        format: AudioFormat,
    ) -> Result<TTSResponse, TTSError> {
        if !self.is_configured() {
            return Err(TTSError::ProviderNotConfigured("openai".to_string()));
        }

        let response_format = match format {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Wav => "wav",
            AudioFormat::Opus => "opus",
        };

        let url = format!("{}/audio/speech", self.base_url);

        let body = serde_json::json!({
            "model": "tts-1",
            "input": text,
            "voice": voice,
            "speed": speed,
            "response_format": response_format
        });

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| TTSError::NetworkError(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let detail = resp
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(TTSError::SynthesisFailed(format!(
                "OpenAI returned {status}: {detail}"
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
            provider: "openai".to_string(),
            voice: voice.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_custom_base_url() {
        let p =
            OpenAIProvider::with_base_url("sk-test".to_string(), "http://local:9000".to_string());
        assert_eq!(p.base_url, "http://local:9000");
        assert!(p.is_configured());
    }

    #[tokio::test]
    async fn test_voices_count_and_content() {
        let p = OpenAIProvider::new("sk-key".to_string());
        let voices = p.voices().await.expect("async operation should succeed");
        assert_eq!(voices.len(), 6);
        // All should have en-US language
        for v in &voices {
            assert_eq!(v.language.as_deref(), Some("en-US"));
            assert!(v.gender.is_some());
        }
    }
}
