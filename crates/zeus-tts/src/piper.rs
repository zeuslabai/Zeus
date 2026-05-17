//! Piper HTTP TTS provider
//!
//! Connects to a remote Piper TTS server via HTTP, sending text and receiving
//! synthesized audio. The server URL is configurable via `ZEUS_PIPER_URL`
//! env var or `[tts].piper_url` in config.toml (defaults to `http://localhost:8104`).
//!
//! Supports both buffered (`synthesize`) and streaming (`synthesize_stream`)
//! modes. Streaming yields audio chunks as they arrive from the server, which
//! is useful for low-latency playback and HTTP chunked transfer to clients.

use async_trait::async_trait;
use futures_util::stream::{Stream, StreamExt};
use std::pin::Pin;
use tracing::instrument;

use crate::{AudioFormat, TTSError, TTSProvider, TTSResponse, Voice};

/// Streaming audio chunk result type.
pub type AudioChunkStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, TTSError>> + Send>>;

/// Default Piper server endpoint (local dev; override with ZEUS_PIPER_URL env var).
const DEFAULT_PIPER_URL: &str = "http://localhost:8104";

/// HTTP-based Piper TTS provider.
///
/// Sends synthesis requests to a remote Piper server and streams back the
/// resulting audio bytes.
pub struct PiperHttpProvider {
    base_url: String,
    client: reqwest::Client,
}

impl PiperHttpProvider {
    /// Create a new provider pointing at the given server URL.
    ///
    /// If `url` is `None`, defaults to [`DEFAULT_PIPER_URL`].
    pub fn new(url: Option<String>) -> Self {
        let base_url = url
            .or_else(|| std::env::var("ZEUS_PIPER_URL").ok().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| DEFAULT_PIPER_URL.to_string())
            .trim_end_matches('/')
            .to_string();

        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    /// Return the configured server URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Synthesize speech and return a stream of audio byte chunks.
    ///
    /// Unlike [`TTSProvider::synthesize`] which buffers the entire response,
    /// this yields `bytes::Bytes` chunks as they arrive from the Piper server,
    /// enabling low-latency playback and chunked HTTP transfer.
    #[instrument(skip(self, text), fields(text_len = text.len(), url = %self.base_url), level = "debug")]
    pub async fn synthesize_stream(
        &self,
        text: &str,
        voice: &str,
        speed: f32,
        format: AudioFormat,
    ) -> Result<AudioChunkStream, TTSError> {
        let url = format!("{}/api/tts", self.base_url);

        let mut query: Vec<(&str, String)> = vec![("text", text.to_string())];

        if voice != "default" && !voice.is_empty() {
            query.push(("voice", voice.to_string()));
        }

        if (speed - 1.0).abs() > f32::EPSILON {
            query.push(("speed", speed.to_string()));
        }

        let output_format = match format {
            AudioFormat::Wav => "wav",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Opus => "opus",
        };
        query.push(("format", output_format.to_string()));

        let resp = self
            .client
            .get(&url)
            .query(&query)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| TTSError::NetworkError(format!("piper HTTP request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
            return Err(TTSError::SynthesisFailed(format!(
                "piper server returned {status}: {body}"
            )));
        }

        // Stream response bytes as they arrive from the server
        let byte_stream = resp.bytes_stream();
        let mapped = byte_stream.map(|chunk| match chunk {
            Ok(bytes) => Ok(bytes.to_vec()),
            Err(e) => Err(TTSError::NetworkError(format!("stream read error: {e}"))),
        });

        Ok(Box::pin(mapped))
    }
}

#[async_trait]
impl TTSProvider for PiperHttpProvider {
    fn id(&self) -> &str {
        "piper"
    }

    fn name(&self) -> &str {
        "Piper (HTTP)"
    }

    fn is_configured(&self) -> bool {
        !self.base_url.is_empty()
    }

    async fn voices(&self) -> Result<Vec<Voice>, TTSError> {
        // Try GET /api/voices — many Piper HTTP servers expose this
        let url = format!("{}/api/voices", self.base_url);
        let resp = self.client.get(&url).send().await;

        match resp {
            Ok(r) if r.status().is_success() => {
                // Parse JSON array of voice objects
                let body = r
                    .text()
                    .await
                    .map_err(|e| TTSError::NetworkError(e.to_string()))?;

                // Piper servers typically return a map of voice_id -> metadata
                if let Ok(map) = serde_json::from_str::<serde_json::Value>(&body) {
                    let voices = if let Some(obj) = map.as_object() {
                        obj.iter()
                            .map(|(id, meta)| Voice {
                                id: id.clone(),
                                name: meta
                                    .get("name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or(id.as_str())
                                    .to_string(),
                                gender: meta
                                    .get("gender")
                                    .and_then(|g| g.as_str())
                                    .map(|g| g.to_string()),
                                language: meta
                                    .get("language")
                                    .and_then(|l| l.as_str())
                                    .map(|l| l.to_string()),
                                preview_url: None,
                            })
                            .collect()
                    } else if let Some(arr) = map.as_array() {
                        arr.iter()
                            .filter_map(|v| {
                                let id = v.get("id").or(v.get("key")).and_then(|i| i.as_str())?;
                                Some(Voice {
                                    id: id.to_string(),
                                    name: v
                                        .get("name")
                                        .and_then(|n| n.as_str())
                                        .unwrap_or(id)
                                        .to_string(),
                                    gender: v
                                        .get("gender")
                                        .and_then(|g| g.as_str())
                                        .map(|g| g.to_string()),
                                    language: v
                                        .get("language")
                                        .and_then(|l| l.as_str())
                                        .map(|l| l.to_string()),
                                    preview_url: None,
                                })
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    Ok(voices)
                } else {
                    Ok(Vec::new())
                }
            }
            _ => {
                // Server may not support /api/voices — return a default voice
                Ok(vec![Voice {
                    id: "default".to_string(),
                    name: "Default".to_string(),
                    gender: None,
                    language: Some("en-US".to_string()),
                    preview_url: None,
                }])
            }
        }
    }

    #[instrument(skip(self, text), fields(text_len = text.len(), url = %self.base_url), level = "debug")]
    async fn synthesize(
        &self,
        text: &str,
        voice: &str,
        speed: f32,
        format: AudioFormat,
    ) -> Result<TTSResponse, TTSError> {
        let url = format!("{}/api/tts", self.base_url);

        let mut query: Vec<(&str, String)> = vec![("text", text.to_string())];

        if voice != "default" && !voice.is_empty() {
            query.push(("voice", voice.to_string()));
        }

        if (speed - 1.0).abs() > f32::EPSILON {
            query.push(("speed", speed.to_string()));
        }

        let output_format = match format {
            AudioFormat::Wav => "wav",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Opus => "opus",
        };
        query.push(("format", output_format.to_string()));

        let resp = self
            .client
            .get(&url)
            .query(&query)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| TTSError::NetworkError(format!("piper HTTP request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
            return Err(TTSError::SynthesisFailed(format!(
                "piper server returned {status}: {body}"
            )));
        }

        // Stream the full response body
        let audio = resp
            .bytes()
            .await
            .map_err(|e| TTSError::NetworkError(format!("failed to read audio response: {e}")))?
            .to_vec();

        if audio.is_empty() {
            return Err(TTSError::SynthesisFailed(
                "piper server returned empty audio".to_string(),
            ));
        }

        // Detect actual format from response — Piper usually returns WAV
        let actual_format = if audio.len() >= 4 && &audio[..4] == b"RIFF" {
            AudioFormat::Wav
        } else {
            format
        };

        Ok(TTSResponse {
            audio,
            format: actual_format,
            duration_ms: None,
            provider: "piper".to_string(),
            voice: voice.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_piper_http_provider_new_default() {
        let p = PiperHttpProvider::new(None);
        assert_eq!(p.id(), "piper");
        assert_eq!(p.name(), "Piper (HTTP)");
        // base_url may be overridden by ZEUS_PIPER_URL env var — assert non-empty only
        assert!(!p.base_url().is_empty());
        assert!(p.is_configured());
    }

    #[test]
    fn test_piper_http_provider_new_custom_url() {
        let p = PiperHttpProvider::new(Some("https://my-piper.example.com".to_string()));
        assert_eq!(p.base_url(), "https://my-piper.example.com");
        assert!(p.is_configured());
    }

    #[test]
    fn test_piper_http_provider_strips_trailing_slash() {
        let p = PiperHttpProvider::new(Some("https://example.com/".to_string()));
        assert_eq!(p.base_url(), "https://example.com");
    }

    #[test]
    fn test_piper_http_provider_id_and_name() {
        let p = PiperHttpProvider::new(None);
        assert_eq!(p.id(), "piper");
        assert_eq!(p.name(), "Piper (HTTP)");
    }

    #[test]
    fn test_piper_http_provider_empty_url_not_configured() {
        let p = PiperHttpProvider::new(Some(String::new()));
        assert!(!p.is_configured());
    }

    #[tokio::test]
    async fn test_piper_voices_fallback() {
        // Against an unreachable server, voices() should return a default voice
        let p = PiperHttpProvider::new(Some("http://127.0.0.1:1".to_string()));
        let voices = p.voices().await.expect("async operation should succeed");
        assert_eq!(voices.len(), 1);
        assert_eq!(voices[0].id, "default");
    }

    #[tokio::test]
    async fn test_piper_synthesize_unreachable_server() {
        let p = PiperHttpProvider::new(Some("http://127.0.0.1:1".to_string()));
        let err = p
            .synthesize("hello", "default", 1.0, AudioFormat::Wav)
            .await
            .unwrap_err();
        assert!(matches!(err, TTSError::NetworkError(_)));
    }

    #[tokio::test]
    async fn test_piper_synthesize_stream_unreachable_server() {
        let p = PiperHttpProvider::new(Some("http://127.0.0.1:1".to_string()));
        let result = p
            .synthesize_stream("hello", "default", 1.0, AudioFormat::Wav)
            .await;
        assert!(result.is_err());
        let err = result.err().expect("err should succeed");
        assert!(matches!(err, TTSError::NetworkError(_)));
    }

    #[test]
    fn test_default_piper_url_constant() {
        assert_eq!(DEFAULT_PIPER_URL, "http://localhost:8104");
    }
}
