//! Speech-to-text for real-time voice streams
//!
//! Transcribes mu-law 8kHz audio bytes from Twilio media streams
//! using Groq or OpenAI Whisper API.
//!
//! API keys are resolved from the credentials map (config.toml `[credentials]`)
//! with a fallback to environment variables for backward compatibility.

use std::collections::HashMap;
use tracing::debug;
use zeus_core::{Error, Result};

use crate::deepgram::{DEEPGRAM_API_KEY_ENV, DeepgramStreamingStt, resolve_deepgram_api_key};

/// Which STT provider to use
#[derive(Debug, Clone, Copy)]
enum SttProvider {
    Groq,
    OpenAI,
}

/// Which realtime streaming STT provider to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingSttProvider {
    Deepgram,
}

impl StreamingSttProvider {
    /// Select the best available realtime streaming STT provider.
    ///
    /// Resolution order per key: credentials map → environment variable.
    /// Deepgram is currently the only realtime provider.
    pub fn select(credentials: Option<&HashMap<String, String>>) -> Result<(Self, String)> {
        if let Some(key) = resolve_deepgram_api_key(credentials) {
            return Ok((StreamingSttProvider::Deepgram, key));
        }

        Err(Error::Internal(format!(
            "No realtime STT API key found. Set {} in [credentials].",
            DEEPGRAM_API_KEY_ENV
        )))
    }
}

/// Build the configured realtime streaming STT client.
pub fn streaming_stt_client(
    credentials: Option<&HashMap<String, String>>,
) -> Result<DeepgramStreamingStt> {
    let (provider, api_key) = StreamingSttProvider::select(credentials)?;

    match provider {
        StreamingSttProvider::Deepgram => DeepgramStreamingStt::new(api_key),
    }
}

impl SttProvider {
    /// Select the best available provider.
    ///
    /// Resolution order per key: credentials map → environment variable.
    /// Priority: Groq > OpenAI.
    fn select(credentials: Option<&HashMap<String, String>>) -> Result<(Self, String)> {
        if let Some(key) = Self::resolve_key("GROQ_API_KEY", credentials) {
            return Ok((SttProvider::Groq, key));
        }
        if let Some(key) = Self::resolve_key("OPENAI_API_KEY", credentials) {
            return Ok((SttProvider::OpenAI, key));
        }
        Err(Error::Internal(
            "No STT API key found. Set GROQ_API_KEY or OPENAI_API_KEY in [credentials]."
                .to_string(),
        ))
    }

    /// Look up a key from credentials first, then fall back to env var.
    fn resolve_key(name: &str, credentials: Option<&HashMap<String, String>>) -> Option<String> {
        if let Some(creds) = credentials
            && let Some(val) = creds.get(name)
            && !val.is_empty()
        {
            return Some(val.clone());
        }
        std::env::var(name).ok()
    }

    fn endpoint(&self) -> &'static str {
        match self {
            SttProvider::Groq => "https://api.groq.com/openai/v1/audio/transcriptions",
            SttProvider::OpenAI => "https://api.openai.com/v1/audio/transcriptions",
        }
    }

    fn model(&self) -> &'static str {
        match self {
            SttProvider::Groq => "whisper-large-v3",
            SttProvider::OpenAI => "whisper-1",
        }
    }
}

/// Build a minimal WAV header for mu-law 8kHz mono audio.
///
/// Format tag 7 = mu-law. Whisper API accepts WAV files.
fn wrap_mulaw_wav(mulaw_bytes: &[u8]) -> Vec<u8> {
    let data_size = mulaw_bytes.len() as u32;
    let file_size = 36 + data_size; // header(44) - 8 + data

    let mut wav = Vec::with_capacity(44 + mulaw_bytes.len());

    // RIFF header
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");

    // fmt chunk
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    wav.extend_from_slice(&7u16.to_le_bytes()); // format tag: 7 = mu-law
    wav.extend_from_slice(&1u16.to_le_bytes()); // channels: 1 (mono)
    wav.extend_from_slice(&8000u32.to_le_bytes()); // sample rate: 8000 Hz
    wav.extend_from_slice(&8000u32.to_le_bytes()); // byte rate: 8000 (1 byte per sample)
    wav.extend_from_slice(&1u16.to_le_bytes()); // block align: 1
    wav.extend_from_slice(&8u16.to_le_bytes()); // bits per sample: 8

    // data chunk
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.extend_from_slice(mulaw_bytes);

    wav
}

/// Transcribe raw mu-law 8kHz audio bytes to text.
///
/// Wraps the bytes in a WAV container and sends to Groq or OpenAI Whisper API.
///
/// `credentials` — optional reference to the `[credentials]` map from config.toml.
/// When `None`, falls back to environment variables (backward-compat).
pub async fn transcribe_mulaw_bytes(
    audio: &[u8],
    credentials: Option<&HashMap<String, String>>,
) -> Result<String> {
    if audio.is_empty() {
        return Ok(String::new());
    }

    let (provider, api_key) = SttProvider::select(credentials)?;

    debug!(
        "Transcribing {} bytes via {} Whisper",
        audio.len(),
        provider.model()
    );

    let wav_data = wrap_mulaw_wav(audio);

    let file_part = reqwest::multipart::Part::bytes(wav_data)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| Error::Internal(format!("Failed to set MIME type: {}", e)))?;

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model", provider.model().to_string())
        .text("language", "en".to_string())
        .text("response_format", "json".to_string());

    let client = reqwest::Client::new();
    let response = client
        .post(provider.endpoint())
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await
        .map_err(|e| Error::Internal(format!("STT API request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "no response body".to_string());
        return Err(Error::Internal(format!(
            "STT API returned {}: {}",
            status, body
        )));
    }

    let resp_json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| Error::Internal(format!("Failed to parse STT response: {}", e)))?;

    let text = resp_json
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_mulaw_wav_header() {
        let audio = vec![0x80u8; 100];
        let wav = wrap_mulaw_wav(&audio);

        // RIFF header
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");

        // fmt chunk
        assert_eq!(&wav[12..16], b"fmt ");
        let fmt_size = u32::from_le_bytes([wav[16], wav[17], wav[18], wav[19]]);
        assert_eq!(fmt_size, 16);
        let format_tag = u16::from_le_bytes([wav[20], wav[21]]);
        assert_eq!(format_tag, 7); // mu-law
        let channels = u16::from_le_bytes([wav[22], wav[23]]);
        assert_eq!(channels, 1);
        let sample_rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
        assert_eq!(sample_rate, 8000);

        // data chunk
        assert_eq!(&wav[36..40], b"data");
        let data_size = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_size, 100);
        assert_eq!(&wav[44..], &audio[..]);
    }

    #[test]
    fn test_wrap_mulaw_wav_total_size() {
        let audio = vec![0u8; 16000]; // 2 seconds at 8kHz
        let wav = wrap_mulaw_wav(&audio);
        assert_eq!(wav.len(), 44 + 16000);

        let file_size = u32::from_le_bytes([wav[4], wav[5], wav[6], wav[7]]);
        assert_eq!(file_size, 36 + 16000);
    }

    #[test]
    fn test_wrap_mulaw_wav_empty() {
        let wav = wrap_mulaw_wav(&[]);
        assert_eq!(wav.len(), 44);
        let data_size = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_size, 0);
    }

    #[tokio::test]
    async fn test_transcribe_empty_returns_empty() {
        let result = transcribe_mulaw_bytes(&[], None)
            .await
            .expect("async operation should succeed");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_transcribe_no_api_key_returns_error() {
        // With no credentials and no env vars, should return error
        let empty_creds = std::collections::HashMap::new();
        if std::env::var("GROQ_API_KEY").is_err() && std::env::var("OPENAI_API_KEY").is_err() {
            let result = transcribe_mulaw_bytes(&[0x80; 100], Some(&empty_creds)).await;
            assert!(result.is_err());
        }
    }

    #[tokio::test]
    async fn test_select_prefers_credentials_over_env() {
        let mut creds = std::collections::HashMap::new();
        creds.insert("GROQ_API_KEY".to_string(), "cred-key-123".to_string());
        let (provider, key) = SttProvider::select(Some(&creds)).unwrap();
        assert!(matches!(provider, SttProvider::Groq));
        assert_eq!(key, "cred-key-123");
    }
}
