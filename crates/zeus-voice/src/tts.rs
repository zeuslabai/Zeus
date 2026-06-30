//! Piper TTS provider for the voice agent loop.
//!
//! Implements [`VoiceTTSProvider`] by calling a Piper TTS HTTP server.
//! This is a standalone HTTP client (no dependency on zeus-tts) that speaks
//! the Piper HTTP API: `GET /api/tts?text=...&voice=...&format=wav`.
//!
//! Default server: `http://localhost:8104` (configurable via `[deployment]` in config.toml)

use async_trait::async_trait;
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::debug;
use zeus_core::Result;

use crate::agent_loop::VoiceTTSProvider;

/// Default Piper TTS server URL (configurable via `[deployment].piper_tts_url`).
const DEFAULT_PIPER_URL: &str = "http://localhost:8104";

/// Available Piper voices.
///
/// These map to voice models installed on the Piper server.
pub const PIPER_VOICES: &[PiperVoice] = &[
    PiperVoice {
        id: "en_US-amy-medium",
        name: "Amy (US)",
        language: "en-US",
        gender: "female",
    },
    PiperVoice {
        id: "en_US-lessac-medium",
        name: "Lessac (US)",
        language: "en-US",
        gender: "male",
    },
    PiperVoice {
        id: "en_US-libritts-high",
        name: "LibriTTS (US)",
        language: "en-US",
        gender: "neutral",
    },
    PiperVoice {
        id: "en_US-ryan-medium",
        name: "Ryan (US)",
        language: "en-US",
        gender: "male",
    },
    PiperVoice {
        id: "en_GB-alan-medium",
        name: "Alan (GB)",
        language: "en-GB",
        gender: "male",
    },
    PiperVoice {
        id: "en_GB-jenny_dioco-medium",
        name: "Jenny (GB)",
        language: "en-GB",
        gender: "female",
    },
    PiperVoice {
        id: "es_ES-davefx-medium",
        name: "Dave (ES)",
        language: "es-ES",
        gender: "male",
    },
    PiperVoice {
        id: "de_DE-thorsten-medium",
        name: "Thorsten (DE)",
        language: "de-DE",
        gender: "male",
    },
];

/// A Piper voice definition.
#[derive(Debug, Clone)]
pub struct PiperVoice {
    pub id: &'static str,
    pub name: &'static str,
    pub language: &'static str,
    pub gender: &'static str,
}

/// HTTP-based Piper TTS provider implementing [`VoiceTTSProvider`].
///
/// Sends text to a Piper HTTP server and returns WAV audio bytes.
/// Supports both buffered and streaming synthesis modes.
pub struct PiperTtsProvider {
    base_url: String,
    client: Client,
}

impl PiperTtsProvider {
    /// Create a new Piper TTS provider.
    ///
    /// If `url` is `None`, reads `ZEUS_PIPER_URL` env var; falls back to [`DEFAULT_PIPER_URL`].
    pub fn new(url: Option<String>) -> Self {
        let base_url = url
            .or_else(|| std::env::var("ZEUS_PIPER_URL").ok())
            .unwrap_or_else(|| DEFAULT_PIPER_URL.to_string())
            .trim_end_matches('/')
            .to_string();

        Self {
            base_url,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Create from the `ZEUS_PIPER_URL` env var, falling back to the default.
    pub fn from_env() -> Self {
        let url = std::env::var("ZEUS_PIPER_URL").ok();
        Self::new(url)
    }

    /// Return the configured base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// List available voices from the server, falling back to built-in list.
    pub async fn list_voices(&self) -> Vec<PiperVoice> {
        // Try the server's /api/voices endpoint
        let url = format!("{}/api/voices", self.base_url);
        match self.client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.text().await
                    && let Ok(map) = serde_json::from_str::<serde_json::Value>(&body)
                    && let Some(obj) = map.as_object()
                {
                    return obj
                        .keys()
                        .take(20)
                        .map(|id| PiperVoice {
                            // Leak into 'static for the struct — these are short-lived
                            // We could use String fields but the built-in list uses &'static str
                            // For dynamic voices, just return the built-in list annotated
                            id: Box::leak(id.clone().into_boxed_str()),
                            name: Box::leak(id.clone().into_boxed_str()),
                            language: "en-US",
                            gender: "unknown",
                        })
                        .collect();
                }
            }
            _ => {}
        }
        // Fallback to built-in voice list
        PIPER_VOICES.to_vec()
    }

    /// Build the TTS request URL with query parameters.
    fn build_url(&self, text: &str, voice: &str) -> String {
        let encoded_text = urlencoding::encode(text);
        let voice_param = if voice.is_empty() || voice == "default" {
            PIPER_VOICES[0].id
        } else {
            voice
        };
        format!(
            "{}/api/tts?text={}&voice={}&format=wav",
            self.base_url, encoded_text, voice_param
        )
    }
}

#[async_trait]
impl VoiceTTSProvider for PiperTtsProvider {
    /// Synthesize text to WAV audio bytes (buffered).
    async fn synthesize_wav(&self, text: &str, voice: &str) -> Result<Vec<u8>> {
        let url = self.build_url(text, voice);

        debug!(
            "Piper TTS: synthesizing {} chars, voice={}, url={}",
            text.len(),
            voice,
            &self.base_url
        );

        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .map_err(|e| zeus_core::Error::Internal(format!("Piper TTS request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_else(|_| "unknown".into());
            return Err(zeus_core::Error::Internal(format!(
                "Piper TTS server returned {status}: {body}"
            )));
        }

        let audio = resp
            .bytes()
            .await
            .map_err(|e| zeus_core::Error::Internal(format!("Failed to read TTS audio: {e}")))?
            .to_vec();

        if audio.is_empty() {
            return Err(zeus_core::Error::Internal(
                "Piper TTS returned empty audio".into(),
            ));
        }

        debug!("Piper TTS: received {} bytes of WAV audio", audio.len());
        Ok(audio)
    }

    /// Synthesize text with streaming — returns chunks via mpsc channel.
    async fn synthesize_stream(
        &self,
        text: &str,
        voice: &str,
    ) -> Result<mpsc::Receiver<Result<Vec<u8>>>> {
        let url = self.build_url(text, voice);

        debug!(
            "Piper TTS stream: synthesizing {} chars, voice={}",
            text.len(),
            voice
        );

        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| {
                zeus_core::Error::Internal(format!("Piper TTS stream request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_else(|_| "unknown".into());
            return Err(zeus_core::Error::Internal(format!(
                "Piper TTS server returned {status}: {body}"
            )));
        }

        let (tx, rx) = mpsc::channel(32);

        // Spawn a task to stream bytes from the HTTP response into the channel
        tokio::spawn(async move {
            use futures_util::StreamExt;
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        if tx.send(Ok(bytes.to_vec())).await.is_err() {
                            break; // receiver dropped
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(zeus_core::Error::Internal(format!(
                                "Piper TTS stream error: {e}"
                            ))))
                            .await;
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }
}

// ============================================================================
// TTS Prefetcher
// ============================================================================

/// Minimum character length for a sentence to be sent to TTS individually.
/// Shorter fragments are merged with the next sentence.
const MIN_SENTENCE_CHARS: usize = 20;

/// Split text into sentence-sized chunks suitable for TTS synthesis.
///
/// Splits on `.`, `!`, or `?` followed by whitespace or end-of-string.
/// Fragments shorter than [`MIN_SENTENCE_CHARS`] are merged with the next
/// sentence to avoid very short TTS requests.
pub fn split_sentences(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return vec![];
    }

    // Split on sentence-ending punctuation + boundary
    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if matches!(ch, '.' | '!' | '?') {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                sentences.push(trimmed);
                current.clear();
            }
        }
    }

    // Flush any remaining text (no trailing punctuation)
    let remainder = current.trim().to_string();
    if !remainder.is_empty() {
        sentences.push(remainder);
    }

    // Merge short fragments forward
    let mut merged: Vec<String> = Vec::new();
    let mut pending = String::new();
    for sentence in sentences {
        if pending.is_empty() {
            pending = sentence;
        } else {
            pending.push(' ');
            pending.push_str(&sentence);
        }
        if pending.len() >= MIN_SENTENCE_CHARS {
            merged.push(pending.clone());
            pending.clear();
        }
    }
    if !pending.is_empty() {
        if let Some(last) = merged.last_mut() {
            last.push(' ');
            last.push_str(&pending);
        } else {
            merged.push(pending);
        }
    }

    merged
}

/// Pre-synthesizes TTS audio for a sequence of text sentences in the background,
/// buffering up to `buffer_size` segments ahead of playback.
///
/// The background task synthesizes sentences in order and sends each result
/// over a bounded channel.  Because the channel has fixed capacity, synthesis
/// naturally pauses when the buffer is full and resumes as the caller consumes
/// segments — implementing backpressure without extra coordination.
///
/// # Usage
/// ```no_run
/// # async fn example() {
/// # use std::sync::Arc;
/// # use zeus_voice::tts::{TtsPrefetcher, split_sentences, PiperTtsProvider};
/// let provider = Arc::new(PiperTtsProvider::from_env());
/// let sentences = split_sentences("Hello world. How are you today?");
/// let mut prefetcher = TtsPrefetcher::new(sentences, provider, "en_US-amy-medium".into(), 3);
/// while let Some(result) = prefetcher.next_segment().await {
///     let audio_bytes = result.unwrap();
///     // play audio_bytes …
/// }
/// # }
/// ```
pub struct TtsPrefetcher {
    segment_rx: mpsc::Receiver<Result<Vec<u8>>>,
}

impl TtsPrefetcher {
    /// Spawn a background synthesizer for `sentences`.
    ///
    /// - `provider`: the TTS backend to call.
    /// - `voice`: voice ID passed to the provider.
    /// - `buffer_size`: channel capacity — limits how many segments are
    ///   pre-synthesized ahead of playback (maps to `prefetch_segments` config).
    pub fn new(
        sentences: Vec<String>,
        provider: Arc<dyn crate::agent_loop::VoiceTTSProvider>,
        voice: String,
        buffer_size: usize,
    ) -> Self {
        let capacity = buffer_size.max(1);
        let (tx, rx) = mpsc::channel(capacity);

        tokio::spawn(async move {
            for sentence in sentences {
                if sentence.trim().is_empty() {
                    continue;
                }
                let audio = provider.synthesize_wav(&sentence, &voice).await;
                if tx.send(audio).await.is_err() {
                    // Receiver dropped — caller no longer interested
                    break;
                }
            }
        });

        Self { segment_rx: rx }
    }

    /// Return the next synthesized audio segment.
    ///
    /// Returns `Some(Ok(bytes))` on success, `Some(Err(...))` if synthesis
    /// failed for that segment, or `None` when all segments are exhausted.
    pub async fn next_segment(&mut self) -> Option<Result<Vec<u8>>> {
        self.segment_rx.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_piper_tts_provider_new_default() {
        // Clear env var so we test the compiled-in default, not any live config.
        temp_env::with_vars([("ZEUS_PIPER_URL", None::<&str>)], || {
            let p = PiperTtsProvider::new(None);
            assert_eq!(p.base_url(), DEFAULT_PIPER_URL);
        });
    }

    #[test]
    fn test_piper_tts_provider_custom_url() {
        let p = PiperTtsProvider::new(Some("http://10.0.0.5:9000".to_string()));
        assert_eq!(p.base_url(), "http://10.0.0.5:9000");
    }

    #[test]
    fn test_piper_tts_provider_strips_trailing_slash() {
        let p = PiperTtsProvider::new(Some("http://example.com/".to_string()));
        assert_eq!(p.base_url(), "http://example.com");
    }

    #[test]
    fn test_piper_tts_provider_from_env_default() {
        temp_env::with_vars([("ZEUS_PIPER_URL", None::<&str>)], || {
            let p = PiperTtsProvider::from_env();
            assert_eq!(p.base_url(), DEFAULT_PIPER_URL);
        });
    }

    #[test]
    fn test_build_url_default_voice() {
        let p = PiperTtsProvider::new(Some("http://localhost:8104".to_string()));
        let url = p.build_url("hello world", "default");
        assert!(url.starts_with("http://localhost:8104/api/tts?"));
        assert!(url.contains("text=hello%20world"));
        assert!(url.contains("voice=en_US-amy-medium"));
        assert!(url.contains("format=wav"));
    }

    #[test]
    fn test_build_url_specific_voice() {
        let p = PiperTtsProvider::new(Some("http://localhost:8104".to_string()));
        let url = p.build_url("test", "en_GB-alan-medium");
        assert!(url.contains("voice=en_GB-alan-medium"));
    }

    #[test]
    fn test_build_url_empty_voice_uses_default() {
        let p = PiperTtsProvider::new(Some("http://localhost:8104".to_string()));
        let url = p.build_url("test", "");
        assert!(url.contains("voice=en_US-amy-medium"));
    }

    #[test]
    fn test_piper_voices_list() {
        assert_eq!(PIPER_VOICES.len(), 8);
        assert_eq!(PIPER_VOICES[0].id, "en_US-amy-medium");
        assert_eq!(PIPER_VOICES[0].language, "en-US");
        assert_eq!(PIPER_VOICES[0].gender, "female");
    }

    #[test]
    fn test_piper_voice_struct() {
        let voice = &PIPER_VOICES[4];
        assert_eq!(voice.id, "en_GB-alan-medium");
        assert_eq!(voice.name, "Alan (GB)");
        assert_eq!(voice.language, "en-GB");
        assert_eq!(voice.gender, "male");
    }

    #[test]
    fn test_default_piper_url() {
        assert_eq!(DEFAULT_PIPER_URL, "http://localhost:8104");
    }

    #[tokio::test]
    async fn test_synthesize_wav_unreachable() {
        let p = PiperTtsProvider::new(Some("http://127.0.0.1:1".to_string()));
        let result = p.synthesize_wav("hello", "default").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_synthesize_stream_unreachable() {
        let p = PiperTtsProvider::new(Some("http://127.0.0.1:1".to_string()));
        let result = p.synthesize_stream("hello", "default").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_voices_fallback() {
        let p = PiperTtsProvider::new(Some("http://127.0.0.1:1".to_string()));
        let voices = p.list_voices().await;
        assert_eq!(voices.len(), 8);
        assert_eq!(voices[0].id, "en_US-amy-medium");
    }

    // ── split_sentences ────────────────────────────────────────────────────

    #[test]
    fn test_split_sentences_empty() {
        assert!(split_sentences("").is_empty());
        assert!(split_sentences("   ").is_empty());
    }

    #[test]
    fn test_split_sentences_single_sentence() {
        let parts = split_sentences("Hello, how are you today?");
        assert_eq!(parts.len(), 1);
        assert!(parts[0].contains("Hello"));
    }

    #[test]
    fn test_split_sentences_multiple() {
        let text = "The agent is ready. It can answer questions! What would you like to know?";
        let parts = split_sentences(text);
        // All three sentences are long enough; expect at most 3 segments
        assert!(!parts.is_empty());
        // Reassembled text should contain all words
        let joined = parts.join(" ");
        assert!(joined.contains("agent"));
        assert!(joined.contains("questions"));
        assert!(joined.contains("know"));
    }

    #[test]
    fn test_split_sentences_merges_short_fragments() {
        // "Hi." is 3 chars — below MIN_SENTENCE_CHARS, should be merged forward
        let text = "Hi. This is a much longer sentence that should stand on its own.";
        let parts = split_sentences(text);
        // The short "Hi." fragment must be merged with the next sentence
        assert!(!parts.is_empty());
        assert!(parts[0].contains("Hi"));
        assert!(parts[0].contains("longer sentence"));
    }

    #[test]
    fn test_split_sentences_no_trailing_punctuation() {
        let text = "This sentence has no ending punctuation";
        let parts = split_sentences(text);
        assert_eq!(parts.len(), 1);
        assert!(parts[0].contains("punctuation"));
    }

    // ── TtsPrefetcher ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_prefetcher_returns_all_segments() {
        use crate::agent_loop::VoiceTTSProvider;
        use std::sync::Arc;
        use tokio::sync::mpsc;

        struct FakeTts;
        #[async_trait::async_trait]
        impl VoiceTTSProvider for FakeTts {
            async fn synthesize_wav(&self, text: &str, _voice: &str) -> zeus_core::Result<Vec<u8>> {
                Ok(text.as_bytes().to_vec())
            }
            async fn synthesize_stream(
                &self,
                _text: &str,
                _voice: &str,
            ) -> zeus_core::Result<mpsc::Receiver<zeus_core::Result<Vec<u8>>>> {
                let (tx, rx) = mpsc::channel(1);
                drop(tx);
                Ok(rx)
            }
        }

        let sentences = vec!["Hello world.".to_string(), "How are you?".to_string()];
        let provider = Arc::new(FakeTts);
        let mut prefetcher = TtsPrefetcher::new(sentences, provider, "default".into(), 3);

        let seg1 = prefetcher.next_segment().await.unwrap().unwrap();
        let seg2 = prefetcher.next_segment().await.unwrap().unwrap();
        assert_eq!(String::from_utf8(seg1).unwrap(), "Hello world.");
        assert_eq!(String::from_utf8(seg2).unwrap(), "How are you?");
        assert!(
            prefetcher.next_segment().await.is_none(),
            "channel should be closed"
        );
    }

    #[tokio::test]
    async fn test_prefetcher_empty_sentences() {
        use crate::agent_loop::VoiceTTSProvider;
        use std::sync::Arc;
        use tokio::sync::mpsc;

        struct FakeTts;
        #[async_trait::async_trait]
        impl VoiceTTSProvider for FakeTts {
            async fn synthesize_wav(&self, _: &str, _: &str) -> zeus_core::Result<Vec<u8>> {
                Ok(vec![])
            }
            async fn synthesize_stream(
                &self,
                _: &str,
                _: &str,
            ) -> zeus_core::Result<mpsc::Receiver<zeus_core::Result<Vec<u8>>>> {
                let (_tx, rx) = mpsc::channel(1);
                Ok(rx)
            }
        }

        let mut prefetcher = TtsPrefetcher::new(vec![], Arc::new(FakeTts), "v".into(), 3);
        assert!(prefetcher.next_segment().await.is_none());
    }

    #[test]
    fn test_voice_loop_config_prefetch_default() {
        use crate::agent_loop::{VoiceLoopConfig, default_prefetch_segments};
        let config = VoiceLoopConfig::default();
        assert_eq!(config.prefetch_segments, default_prefetch_segments());
        assert_eq!(config.prefetch_segments, 3);
    }
}
