//! Deepgram realtime streaming speech-to-text.
//!
//! Deepgram's Listen API accepts raw audio over WebSocket and emits
//! incremental/final transcript events. This module is intentionally additive to
//! the existing batch Whisper helper in `stt.rs`: Twilio/Meet-style realtime
//! callers can stream mu-law frames directly here, while the legacy
//! `transcribe_mulaw_bytes` path remains Groq/OpenAI multipart batch STT.

use std::collections::HashMap;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use tracing::{debug, trace};
use zeus_core::{Error, Result};

/// Credentials/env key used for Deepgram Listen streaming.
pub const DEEPGRAM_API_KEY_ENV: &str = "DEEPGRAM_API_KEY";

/// Deepgram Listen WebSocket endpoint.
pub const DEEPGRAM_LISTEN_URL: &str = "wss://api.deepgram.com/v1/listen";

/// Audio encoding sent to Deepgram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeepgramEncoding {
    /// 8-bit mu-law audio, the native format of Twilio media streams.
    Mulaw,
    /// Signed 16-bit linear PCM.
    Linear16,
}

impl DeepgramEncoding {
    fn as_query_value(self) -> &'static str {
        match self {
            Self::Mulaw => "mulaw",
            Self::Linear16 => "linear16",
        }
    }
}

/// Listen API configuration for a Deepgram streaming session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepgramListenConfig {
    /// Deepgram model name. `nova-3` is the current general realtime default.
    pub model: String,
    /// Optional BCP-47-ish language hint, e.g. `en-US`.
    pub language: Option<String>,
    /// Input audio encoding.
    pub encoding: DeepgramEncoding,
    /// Input sample rate in Hz.
    pub sample_rate: u32,
    /// Number of audio channels.
    pub channels: u16,
    /// Emit interim transcripts before a final result.
    pub interim_results: bool,
    /// Add punctuation.
    pub punctuate: bool,
    /// Enable Deepgram smart formatting.
    pub smart_format: bool,
    /// Deepgram endpointing value in milliseconds.
    pub endpointing_ms: Option<u16>,
    /// Emit utterance-end events after this silence duration.
    pub utterance_end_ms: Option<u16>,
}

impl Default for DeepgramListenConfig {
    fn default() -> Self {
        Self {
            model: "nova-3".to_string(),
            language: Some("en-US".to_string()),
            encoding: DeepgramEncoding::Mulaw,
            sample_rate: 8_000,
            channels: 1,
            interim_results: true,
            punctuate: true,
            smart_format: true,
            endpointing_ms: Some(300),
            utterance_end_ms: Some(1_000),
        }
    }
}

impl DeepgramListenConfig {
    /// Build the Deepgram Listen WebSocket URL with query parameters.
    pub fn listen_url(&self) -> String {
        let mut params = vec![
            ("model", self.model.as_str()),
            ("encoding", self.encoding.as_query_value()),
        ];

        let sample_rate = self.sample_rate.to_string();
        let channels = self.channels.to_string();
        let interim_results = self.interim_results.to_string();
        let punctuate = self.punctuate.to_string();
        let smart_format = self.smart_format.to_string();

        params.push(("sample_rate", sample_rate.as_str()));
        params.push(("channels", channels.as_str()));
        params.push(("interim_results", interim_results.as_str()));
        params.push(("punctuate", punctuate.as_str()));
        params.push(("smart_format", smart_format.as_str()));

        if let Some(language) = self.language.as_deref() {
            params.push(("language", language));
        }

        let endpointing = self.endpointing_ms.map(|value| value.to_string());
        if let Some(endpointing) = endpointing.as_deref() {
            params.push(("endpointing", endpointing));
        }

        let utterance_end_ms = self.utterance_end_ms.map(|value| value.to_string());
        if let Some(utterance_end_ms) = utterance_end_ms.as_deref() {
            params.push(("utterance_end_ms", utterance_end_ms));
        }

        let query = params
            .into_iter()
            .map(|(key, value)| {
                format!(
                    "{}={}",
                    urlencoding::encode(key),
                    urlencoding::encode(value)
                )
            })
            .collect::<Vec<_>>()
            .join("&");

        format!("{}?{}", DEEPGRAM_LISTEN_URL, query)
    }
}

/// Resolve `DEEPGRAM_API_KEY` from config credentials first, then env.
pub fn resolve_deepgram_api_key(credentials: Option<&HashMap<String, String>>) -> Option<String> {
    if let Some(creds) = credentials
        && let Some(value) = creds.get(DEEPGRAM_API_KEY_ENV)
        && !value.trim().is_empty()
    {
        return Some(value.clone());
    }

    std::env::var(DEEPGRAM_API_KEY_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Deepgram streaming STT client.
#[derive(Debug, Clone)]
pub struct DeepgramStreamingStt {
    api_key: String,
    config: DeepgramListenConfig,
}

impl DeepgramStreamingStt {
    /// Create a client from the `[credentials]` map or environment.
    pub fn from_credentials(credentials: Option<&HashMap<String, String>>) -> Result<Self> {
        let api_key = resolve_deepgram_api_key(credentials).ok_or_else(|| {
            Error::Internal(
                "No Deepgram STT API key found. Set DEEPGRAM_API_KEY in [credentials].".to_string(),
            )
        })?;

        Self::with_config(api_key, DeepgramListenConfig::default())
    }

    /// Create a client with the default Listen configuration.
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        Self::with_config(api_key, DeepgramListenConfig::default())
    }

    /// Create a client with an explicit Listen configuration.
    pub fn with_config(api_key: impl Into<String>, config: DeepgramListenConfig) -> Result<Self> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(Error::Internal("Deepgram API key is empty".to_string()));
        }

        Ok(Self { api_key, config })
    }

    /// The Listen URL this client will connect to.
    pub fn listen_url(&self) -> String {
        self.config.listen_url()
    }

    /// Connect to Deepgram Listen streaming.
    pub async fn connect(&self) -> Result<DeepgramStreamingSession> {
        let url = self.listen_url();
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| Error::Internal(format!("Deepgram request build failed: {}", e)))?;

        let auth_header = format!("Token {}", self.api_key)
            .parse()
            .map_err(|e| Error::Internal(format!("Deepgram auth header invalid: {}", e)))?;
        request.headers_mut().insert("Authorization", auth_header);

        debug!("Connecting to Deepgram Listen streaming: {}", url);
        let (ws_stream, _) = connect_async(request)
            .await
            .map_err(|e| Error::Internal(format!("Deepgram WebSocket connect failed: {}", e)))?;

        Ok(DeepgramStreamingSession { ws_stream })
    }
}

type DeepgramWebSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Active Deepgram streaming session.
pub struct DeepgramStreamingSession {
    ws_stream: DeepgramWebSocket,
}

impl DeepgramStreamingSession {
    /// Send raw audio bytes to Deepgram.
    pub async fn send_audio(&mut self, audio: &[u8]) -> Result<()> {
        if audio.is_empty() {
            return Ok(());
        }

        self.ws_stream
            .send(Message::Binary(audio.to_vec()))
            .await
            .map_err(|e| Error::Internal(format!("Deepgram audio send failed: {}", e)))
    }

    /// Ask Deepgram to finalize the current stream.
    pub async fn close_stream(&mut self) -> Result<()> {
        self.ws_stream
            .send(Message::Text(r#"{"type":"CloseStream"}"#.to_string()))
            .await
            .map_err(|e| Error::Internal(format!("Deepgram CloseStream send failed: {}", e)))
    }

    /// Close the WebSocket session.
    pub async fn close(&mut self) -> Result<()> {
        self.ws_stream
            .close(None)
            .await
            .map_err(|e| Error::Internal(format!("Deepgram WebSocket close failed: {}", e)))
    }

    /// Read the next non-empty transcript from Deepgram.
    pub async fn next_transcript(&mut self) -> Result<Option<DeepgramTranscript>> {
        while let Some(message) = self.ws_stream.next().await {
            match message.map_err(|e| Error::Internal(format!("Deepgram receive failed: {}", e)))? {
                Message::Text(text) => {
                    if let Some(transcript) = parse_deepgram_transcript(&text)? {
                        return Ok(Some(transcript));
                    }
                }
                Message::Binary(bytes) => {
                    let text = std::str::from_utf8(&bytes).map_err(|e| {
                        Error::Internal(format!("Deepgram binary message was not UTF-8: {}", e))
                    })?;
                    if let Some(transcript) = parse_deepgram_transcript(text)? {
                        return Ok(Some(transcript));
                    }
                }
                Message::Ping(payload) => {
                    self.ws_stream
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|e| {
                            Error::Internal(format!("Deepgram pong send failed: {}", e))
                        })?;
                }
                Message::Pong(_) => {}
                Message::Close(_) => return Ok(None),
                Message::Frame(_) => {}
            }
        }

        Ok(None)
    }
}

/// Transcript emitted by Deepgram Listen streaming.
#[derive(Debug, Clone, PartialEq)]
pub struct DeepgramTranscript {
    pub text: String,
    pub is_final: bool,
    pub speech_final: bool,
    pub confidence: Option<f64>,
    pub channel_index: Vec<u32>,
}

#[derive(Debug, Deserialize)]
struct DeepgramMessage {
    #[serde(rename = "type")]
    message_type: Option<String>,
    #[serde(default)]
    is_final: bool,
    #[serde(default)]
    speech_final: bool,
    #[serde(default)]
    channel_index: Vec<u32>,
    channel: Option<DeepgramChannel>,
}

#[derive(Debug, Deserialize)]
struct DeepgramChannel {
    #[serde(default)]
    alternatives: Vec<DeepgramAlternative>,
}

#[derive(Debug, Deserialize)]
struct DeepgramAlternative {
    #[serde(default)]
    transcript: String,
    confidence: Option<f64>,
}

/// Parse a Deepgram Listen message, returning only non-empty transcript events.
pub fn parse_deepgram_transcript(payload: &str) -> Result<Option<DeepgramTranscript>> {
    let message: DeepgramMessage = serde_json::from_str(payload)?;

    if !matches!(message.message_type.as_deref(), Some("Results") | None) {
        trace!(
            "Ignoring Deepgram control message: {:?}",
            message.message_type
        );
        return Ok(None);
    }

    let Some(channel) = message.channel else {
        return Ok(None);
    };

    let Some(alternative) = channel.alternatives.into_iter().next() else {
        return Ok(None);
    };

    let text = alternative.transcript.trim().to_string();
    if text.is_empty() {
        return Ok(None);
    }

    Ok(Some(DeepgramTranscript {
        text,
        is_final: message.is_final,
        speech_final: message.speech_final,
        confidence: alternative.confidence,
        channel_index: message.channel_index,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_listen_url_matches_twilio_mulaw_shape() {
        let url = DeepgramListenConfig::default().listen_url();

        assert!(url.starts_with("wss://api.deepgram.com/v1/listen?"));
        assert!(url.contains("model=nova-3"));
        assert!(url.contains("encoding=mulaw"));
        assert!(url.contains("sample_rate=8000"));
        assert!(url.contains("channels=1"));
        assert!(url.contains("interim_results=true"));
        assert!(url.contains("punctuate=true"));
        assert!(url.contains("smart_format=true"));
        assert!(url.contains("language=en-US"));
        assert!(url.contains("endpointing=300"));
        assert!(url.contains("utterance_end_ms=1000"));
    }

    #[test]
    fn custom_listen_url_escapes_query_values() {
        let config = DeepgramListenConfig {
            model: "nova custom".to_string(),
            language: Some("pt-BR".to_string()),
            encoding: DeepgramEncoding::Linear16,
            sample_rate: 16_000,
            channels: 2,
            interim_results: false,
            punctuate: false,
            smart_format: false,
            endpointing_ms: None,
            utterance_end_ms: None,
        };

        let url = config.listen_url();
        assert!(url.contains("model=nova%20custom"));
        assert!(url.contains("encoding=linear16"));
        assert!(url.contains("sample_rate=16000"));
        assert!(url.contains("channels=2"));
        assert!(url.contains("interim_results=false"));
        assert!(url.contains("language=pt-BR"));
        assert!(!url.contains("endpointing="));
        assert!(!url.contains("utterance_end_ms="));
    }

    #[test]
    fn credentials_take_precedence_over_env() {
        temp_env::with_var(DEEPGRAM_API_KEY_ENV, Some("env-key"), || {
            let mut credentials = HashMap::new();
            credentials.insert(
                DEEPGRAM_API_KEY_ENV.to_string(),
                "credential-key".to_string(),
            );

            let key = resolve_deepgram_api_key(Some(&credentials)).unwrap();
            assert_eq!(key, "credential-key");
        });
    }

    #[test]
    fn empty_credentials_fall_back_to_env() {
        temp_env::with_var(DEEPGRAM_API_KEY_ENV, Some("env-key"), || {
            let mut credentials = HashMap::new();
            credentials.insert(DEEPGRAM_API_KEY_ENV.to_string(), "".to_string());

            let key = resolve_deepgram_api_key(Some(&credentials)).unwrap();
            assert_eq!(key, "env-key");
        });
    }

    #[test]
    fn missing_api_key_errors() {
        temp_env::with_var(DEEPGRAM_API_KEY_ENV, None::<&str>, || {
            let credentials = HashMap::new();
            let err = DeepgramStreamingStt::from_credentials(Some(&credentials)).unwrap_err();
            assert!(err.to_string().contains("DEEPGRAM_API_KEY"));
        });
    }

    #[test]
    fn parses_final_transcript() {
        let payload = r#"{
            "type": "Results",
            "channel_index": [0, 1],
            "is_final": true,
            "speech_final": true,
            "channel": {
                "alternatives": [{
                    "transcript": " hello from deepgram ",
                    "confidence": 0.97
                }]
            }
        }"#;

        let transcript = parse_deepgram_transcript(payload).unwrap().unwrap();
        assert_eq!(transcript.text, "hello from deepgram");
        assert!(transcript.is_final);
        assert!(transcript.speech_final);
        assert_eq!(transcript.confidence, Some(0.97));
        assert_eq!(transcript.channel_index, vec![0, 1]);
    }

    #[test]
    fn ignores_control_and_empty_messages() {
        let metadata = r#"{"type":"Metadata","request_id":"abc"}"#;
        assert!(parse_deepgram_transcript(metadata).unwrap().is_none());

        let empty_result = r#"{
            "type":"Results",
            "channel":{"alternatives":[{"transcript":"   "}]}
        }"#;
        assert!(parse_deepgram_transcript(empty_result).unwrap().is_none());
    }
}
