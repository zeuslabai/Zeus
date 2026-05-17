//! Inbound Voice Call Handling + Recording/Transcription Pipeline
//!
//! Provides:
//! - TwilioVoiceConfig for inbound call configuration
//! - Inbound call webhook handler (POST /v1/voice/inbound)
//! - Call recording download from Twilio Recordings API
//! - Whisper transcription pipeline for recorded calls

use serde::{Deserialize, Serialize};
use tracing::{debug, info};
use zeus_core::{Error, Result};

/// Twilio REST API base URL
const TWILIO_API_BASE: &str = "https://api.twilio.com/2010-04-01";

/// Maximum recording size to download (50 MB)
const MAX_RECORDING_SIZE: usize = 50 * 1024 * 1024;

// ============================================================================
// TwilioVoiceConfig
// ============================================================================

/// Extended Twilio voice configuration for inbound calls and recordings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwilioVoiceConfig {
    /// Twilio Account SID
    pub account_sid: String,
    /// Twilio Auth Token
    pub auth_token: String,
    /// Phone number receiving inbound calls (E.164)
    #[serde(default)]
    pub inbound_number: String,
    /// Webhook path for inbound calls (default: /v1/voice/inbound)
    #[serde(default = "default_inbound_path")]
    pub inbound_webhook_path: String,
    /// Local directory or bucket path for storing recordings
    #[serde(default = "default_recording_bucket")]
    pub recording_bucket: String,
    /// Whether to auto-record inbound calls
    #[serde(default = "default_auto_record")]
    pub auto_record: bool,
    /// TTS voice for answering inbound calls
    #[serde(default = "default_tts_voice")]
    pub tts_voice: String,
    /// Greeting message for inbound callers
    #[serde(default = "default_greeting")]
    pub greeting: String,
    /// Maximum recording duration in seconds (default: 3600 = 1 hour)
    #[serde(default = "default_max_recording_duration")]
    pub max_recording_duration: u32,
    /// Webhook base URL for callbacks
    #[serde(default)]
    pub webhook_base_url: String,
}

fn default_inbound_path() -> String {
    "/v1/voice/inbound".to_string()
}

fn default_recording_bucket() -> String {
    let dir = zeus_core::default_config_dir().join("recordings");
    dir.to_string_lossy().to_string()
}

fn default_auto_record() -> bool {
    true
}

fn default_tts_voice() -> String {
    "Polly.Amy".to_string()
}

fn default_greeting() -> String {
    "Hello, you have reached Zeus. How can I help you today?".to_string()
}

fn default_max_recording_duration() -> u32 {
    3600
}

impl Default for TwilioVoiceConfig {
    fn default() -> Self {
        Self {
            account_sid: String::new(),
            auth_token: String::new(),
            inbound_number: String::new(),
            inbound_webhook_path: default_inbound_path(),
            recording_bucket: default_recording_bucket(),
            auto_record: default_auto_record(),
            tts_voice: default_tts_voice(),
            greeting: default_greeting(),
            max_recording_duration: default_max_recording_duration(),
            webhook_base_url: String::new(),
        }
    }
}

impl TwilioVoiceConfig {
    /// Create config from environment variables
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(sid) = std::env::var("TWILIO_ACCOUNT_SID") {
            config.account_sid = sid;
        }
        if let Ok(token) = std::env::var("TWILIO_AUTH_TOKEN") {
            config.auth_token = token;
        }
        if let Ok(number) = std::env::var("TWILIO_INBOUND_NUMBER") {
            config.inbound_number = number;
        }
        if let Ok(path) = std::env::var("ZEUS_VOICE_INBOUND_PATH") {
            config.inbound_webhook_path = path;
        }
        if let Ok(bucket) = std::env::var("ZEUS_RECORDING_BUCKET") {
            config.recording_bucket = bucket;
        }
        if let Ok(url) = std::env::var("ZEUS_VOICE_WEBHOOK_URL") {
            config.webhook_base_url = url;
        }
        config
    }

    /// Validate the config has required fields
    pub fn validate(&self) -> Result<()> {
        if self.account_sid.is_empty() {
            return Err(Error::Config(
                "TwilioVoiceConfig: account_sid is required".to_string(),
            ));
        }
        if self.auth_token.is_empty() {
            return Err(Error::Config(
                "TwilioVoiceConfig: auth_token is required".to_string(),
            ));
        }
        Ok(())
    }
}

// ============================================================================
// Inbound Call Webhook Payload
// ============================================================================

/// Twilio inbound call webhook payload (form-urlencoded)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundCallWebhook {
    /// Unique call identifier
    #[serde(rename = "CallSid")]
    pub call_sid: String,
    /// Caller's phone number (E.164)
    #[serde(rename = "From")]
    pub from: String,
    /// Number that was called (E.164)
    #[serde(rename = "To")]
    pub to: String,
    /// Call status
    #[serde(rename = "CallStatus", default)]
    pub call_status: String,
    /// Call direction ("inbound" or "outbound")
    #[serde(rename = "Direction", default)]
    pub direction: String,
    /// Caller's city (if available)
    #[serde(rename = "CallerCity", default)]
    pub caller_city: Option<String>,
    /// Caller's state (if available)
    #[serde(rename = "CallerState", default)]
    pub caller_state: Option<String>,
    /// Caller's country
    #[serde(rename = "CallerCountry", default)]
    pub caller_country: Option<String>,
}

/// Twilio recording status callback payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStatusWebhook {
    /// Call SID the recording belongs to
    #[serde(rename = "CallSid")]
    pub call_sid: String,
    /// Recording SID
    #[serde(rename = "RecordingSid")]
    pub recording_sid: String,
    /// Recording URL (without extension)
    #[serde(rename = "RecordingUrl")]
    pub recording_url: String,
    /// Recording duration in seconds
    #[serde(rename = "RecordingDuration")]
    pub recording_duration: String,
    /// Recording status
    #[serde(rename = "RecordingStatus")]
    pub recording_status: String,
}

// ============================================================================
// TwiML Generation
// ============================================================================

/// Generate TwiML for answering an inbound call.
///
/// Says the greeting, optionally starts recording, then connects
/// to the WebSocket media stream for bidirectional audio.
pub fn inbound_call_twiml(config: &TwilioVoiceConfig) -> String {
    let webhook_host = config
        .webhook_base_url
        .trim_start_matches("https://")
        .trim_start_matches("http://");

    let recording_attr = if config.auto_record {
        format!(
            r#" record="record-from-answer-dual" recordingStatusCallback="{}/v1/voice/recording-status" maxLength="{}" trim="trim-silence""#,
            config.webhook_base_url, config.max_recording_duration,
        )
    } else {
        String::new()
    };

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Response>
    <Say voice="{}">{}</Say>
    <Connect{}>
        <Stream url="wss://{}/voice/media-stream" />
    </Connect>
</Response>"#,
        config.tts_voice,
        xml_escape(&config.greeting),
        recording_attr,
        webhook_host,
    )
}

/// Generate TwiML for a simple voicemail-style greeting + record
pub fn voicemail_twiml(config: &TwilioVoiceConfig) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Response>
    <Say voice="{}">Please leave a message after the beep.</Say>
    <Record maxLength="{}" recordingStatusCallback="{}/v1/voice/recording-status" trim="trim-silence" playBeep="true" />
    <Say voice="{}">Thank you. Goodbye.</Say>
</Response>"#,
        config.tts_voice, config.max_recording_duration, config.webhook_base_url, config.tts_voice,
    )
}

/// Escape special XML characters
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ============================================================================
// Recording Download
// ============================================================================

/// Download a call recording from Twilio.
///
/// Returns the raw audio bytes (WAV format by appending .wav to the URL).
pub async fn download_recording(
    account_sid: &str,
    auth_token: &str,
    recording_sid: &str,
) -> Result<Vec<u8>> {
    let url = format!(
        "{}/Accounts/{}/Recordings/{}.wav",
        TWILIO_API_BASE, account_sid, recording_sid,
    );

    info!("Downloading recording {} from Twilio", recording_sid);

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .basic_auth(account_sid, Some(auth_token))
        .send()
        .await
        .map_err(|e| Error::Tool(format!("Failed to download recording: {}", e)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Tool(format!(
            "Recording download failed ({}): {}",
            status, body
        )));
    }

    // Check content-length if available
    if let Some(len) = resp.content_length()
        && len as usize > MAX_RECORDING_SIZE
    {
        return Err(Error::Tool(format!(
            "Recording too large ({} bytes, max {})",
            len, MAX_RECORDING_SIZE
        )));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read recording bytes: {}", e)))?;

    if bytes.len() > MAX_RECORDING_SIZE {
        return Err(Error::Tool(format!(
            "Recording too large ({} bytes, max {})",
            bytes.len(),
            MAX_RECORDING_SIZE
        )));
    }

    info!(
        "Downloaded recording {}: {} bytes",
        recording_sid,
        bytes.len()
    );

    Ok(bytes.to_vec())
}

/// Save recording bytes to the recording bucket directory
pub async fn save_recording(
    recording_bucket: &str,
    recording_sid: &str,
    data: &[u8],
) -> Result<String> {
    let dir = std::path::Path::new(recording_bucket);
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| Error::Tool(format!("Failed to create recording directory: {}", e)))?;

    let file_path = dir.join(format!("{}.wav", recording_sid));
    tokio::fs::write(&file_path, data)
        .await
        .map_err(|e| Error::Tool(format!("Failed to write recording: {}", e)))?;

    let path_str = file_path.to_string_lossy().to_string();
    info!("Saved recording to {}", path_str);
    Ok(path_str)
}

// ============================================================================
// Whisper Transcription Pipeline
// ============================================================================

/// Transcribe a WAV file using Whisper API (Groq or OpenAI).
///
/// Accepts raw WAV bytes and sends to the best available Whisper API.
pub async fn transcribe_recording(wav_data: &[u8]) -> Result<String> {
    if wav_data.is_empty() {
        return Ok(String::new());
    }

    // Select provider: prefer Groq, fallback to OpenAI
    let (endpoint, model, api_key) = select_whisper_provider()?;

    info!(
        "Transcribing {} bytes via {} ({})",
        wav_data.len(),
        model,
        endpoint
    );

    let file_part = reqwest::multipart::Part::bytes(wav_data.to_vec())
        .file_name("recording.wav")
        .mime_str("audio/wav")
        .map_err(|e| Error::Internal(format!("Failed to set MIME type: {}", e)))?;

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model", model.to_string())
        .text("language", "en".to_string())
        .text("response_format", "verbose_json".to_string());

    let client = reqwest::Client::new();
    let response = client
        .post(endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| Error::Internal(format!("Whisper API request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Internal(format!(
            "Whisper API returned {}: {}",
            status, body
        )));
    }

    let resp_json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| Error::Internal(format!("Failed to parse Whisper response: {}", e)))?;

    let text = resp_json
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    debug!("Transcription result: {} chars", text.len());
    Ok(text)
}

/// Transcription result with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionResult {
    /// The transcribed text
    pub text: String,
    /// Recording SID
    pub recording_sid: String,
    /// Call SID
    pub call_sid: String,
    /// Duration in seconds (from Twilio)
    pub duration_secs: Option<u32>,
    /// File path where recording was saved (if any)
    pub recording_path: Option<String>,
}

/// Full pipeline: download recording → save → transcribe
///
/// Returns a TranscriptionResult with text and metadata.
pub async fn recording_transcription_pipeline(
    config: &TwilioVoiceConfig,
    call_sid: &str,
    recording_sid: &str,
    recording_duration: Option<&str>,
) -> Result<TranscriptionResult> {
    info!(
        "Starting transcription pipeline for recording {} (call {})",
        recording_sid, call_sid
    );

    // 1. Download recording
    let wav_data =
        download_recording(&config.account_sid, &config.auth_token, recording_sid).await?;

    // 2. Save to recording bucket
    let recording_path = save_recording(&config.recording_bucket, recording_sid, &wav_data).await?;

    // 3. Transcribe
    let text = transcribe_recording(&wav_data).await?;

    let duration_secs = recording_duration.and_then(|d| d.parse::<u32>().ok());

    info!(
        "Transcription pipeline complete for {}: {} chars, {}s",
        recording_sid,
        text.len(),
        duration_secs.unwrap_or(0)
    );

    Ok(TranscriptionResult {
        text,
        recording_sid: recording_sid.to_string(),
        call_sid: call_sid.to_string(),
        duration_secs,
        recording_path: Some(recording_path),
    })
}

/// Select the best available Whisper provider
fn select_whisper_provider() -> Result<(&'static str, &'static str, String)> {
    if let Ok(key) = std::env::var("GROQ_API_KEY") {
        return Ok((
            "https://api.groq.com/openai/v1/audio/transcriptions",
            "whisper-large-v3",
            key,
        ));
    }
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        return Ok((
            "https://api.openai.com/v1/audio/transcriptions",
            "whisper-1",
            key,
        ));
    }
    Err(Error::Internal(
        "No Whisper API key found. Set GROQ_API_KEY or OPENAI_API_KEY.".to_string(),
    ))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_twilio_voice_config_default() {
        let config = TwilioVoiceConfig::default();
        assert_eq!(config.inbound_webhook_path, "/v1/voice/inbound");
        assert!(config.auto_record);
        assert_eq!(config.tts_voice, "Polly.Amy");
        assert_eq!(config.max_recording_duration, 3600);
        assert!(config.account_sid.is_empty());
        assert!(config.auth_token.is_empty());
    }

    #[test]
    fn test_twilio_voice_config_serialization() {
        let config = TwilioVoiceConfig {
            account_sid: "AC123".to_string(),
            auth_token: "secret".to_string(),
            inbound_number: "+15551234567".to_string(),
            webhook_base_url: "https://example.ngrok.io".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&config).expect("should serialize");
        let parsed: TwilioVoiceConfig = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(parsed.account_sid, "AC123");
        assert_eq!(parsed.inbound_number, "+15551234567");
        assert_eq!(parsed.inbound_webhook_path, "/v1/voice/inbound");
        assert!(parsed.auto_record);
    }

    #[test]
    fn test_twilio_voice_config_validate_ok() {
        let config = TwilioVoiceConfig {
            account_sid: "AC123".to_string(),
            auth_token: "secret".to_string(),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_twilio_voice_config_validate_missing_sid() {
        let config = TwilioVoiceConfig {
            auth_token: "secret".to_string(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_twilio_voice_config_validate_missing_token() {
        let config = TwilioVoiceConfig {
            account_sid: "AC123".to_string(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_inbound_call_twiml_basic() {
        let config = TwilioVoiceConfig {
            tts_voice: "Polly.Amy".to_string(),
            greeting: "Hello, this is Zeus.".to_string(),
            webhook_base_url: "https://example.ngrok.io".to_string(),
            auto_record: false,
            ..Default::default()
        };
        let twiml = inbound_call_twiml(&config);

        assert!(twiml.contains(r#"<?xml version="1.0" encoding="UTF-8"?>"#));
        assert!(twiml.contains("<Response>"));
        assert!(twiml.contains("</Response>"));
        assert!(twiml.contains(r#"voice="Polly.Amy""#));
        assert!(twiml.contains("Hello, this is Zeus."));
        assert!(twiml.contains("wss://example.ngrok.io/voice/media-stream"));
        assert!(!twiml.contains("record="));
    }

    #[test]
    fn test_inbound_call_twiml_with_recording() {
        let config = TwilioVoiceConfig {
            tts_voice: "Polly.Joanna".to_string(),
            greeting: "Welcome to Zeus.".to_string(),
            webhook_base_url: "https://my-server.com".to_string(),
            auto_record: true,
            max_recording_duration: 600,
            ..Default::default()
        };
        let twiml = inbound_call_twiml(&config);

        assert!(twiml.contains(r#"record="record-from-answer-dual""#));
        assert!(twiml.contains("recordingStatusCallback"));
        assert!(twiml.contains("maxLength=\"600\""));
        assert!(twiml.contains("wss://my-server.com/voice/media-stream"));
    }

    #[test]
    fn test_inbound_call_twiml_escapes_greeting() {
        let config = TwilioVoiceConfig {
            greeting: "Hello & welcome <user>".to_string(),
            webhook_base_url: "https://example.com".to_string(),
            auto_record: false,
            ..Default::default()
        };
        let twiml = inbound_call_twiml(&config);
        assert!(twiml.contains("Hello &amp; welcome &lt;user&gt;"));
    }

    #[test]
    fn test_voicemail_twiml() {
        let config = TwilioVoiceConfig {
            tts_voice: "Polly.Amy".to_string(),
            webhook_base_url: "https://example.com".to_string(),
            max_recording_duration: 120,
            ..Default::default()
        };
        let twiml = voicemail_twiml(&config);

        assert!(twiml.contains("leave a message"));
        assert!(twiml.contains(r#"maxLength="120""#));
        assert!(twiml.contains("playBeep=\"true\""));
        assert!(twiml.contains("recordingStatusCallback"));
    }

    #[test]
    fn test_xml_escape_special_chars() {
        assert_eq!(xml_escape("a & b"), "a &amp; b");
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(xml_escape(r#"he said "hi""#), "he said &quot;hi&quot;");
        assert_eq!(xml_escape("it's"), "it&apos;s");
    }

    #[test]
    fn test_inbound_call_webhook_deserialization() {
        let data = "CallSid=CA123&From=%2B15551234567&To=%2B15559876543&CallStatus=ringing&Direction=inbound";
        let webhook: InboundCallWebhook =
            serde_urlencoded::from_str(data).expect("should deserialize");
        assert_eq!(webhook.call_sid, "CA123");
        assert_eq!(webhook.from, "+15551234567");
        assert_eq!(webhook.to, "+15559876543");
        assert_eq!(webhook.call_status, "ringing");
        assert_eq!(webhook.direction, "inbound");
    }

    #[test]
    fn test_recording_status_webhook_deserialization() {
        let data = "CallSid=CA123&RecordingSid=RE456&RecordingUrl=https%3A%2F%2Fapi.twilio.com%2F2010-04-01%2FAccounts%2FAC123%2FRecordings%2FRE456&RecordingDuration=30&RecordingStatus=completed";
        let webhook: RecordingStatusWebhook =
            serde_urlencoded::from_str(data).expect("should deserialize");
        assert_eq!(webhook.call_sid, "CA123");
        assert_eq!(webhook.recording_sid, "RE456");
        assert_eq!(webhook.recording_duration, "30");
        assert_eq!(webhook.recording_status, "completed");
    }

    #[test]
    fn test_transcription_result_serialization() {
        let result = TranscriptionResult {
            text: "Hello world".to_string(),
            recording_sid: "RE123".to_string(),
            call_sid: "CA456".to_string(),
            duration_secs: Some(30),
            recording_path: Some("/recordings/RE123.wav".to_string()),
        };
        let json = serde_json::to_string(&result).expect("should serialize");
        let parsed: TranscriptionResult = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(parsed.text, "Hello world");
        assert_eq!(parsed.recording_sid, "RE123");
        assert_eq!(parsed.duration_secs, Some(30));
    }

    #[tokio::test]
    async fn test_save_recording() {
        let tmp = std::env::temp_dir().join("zeus_test_save_recording");
        let _ = std::fs::remove_dir_all(&tmp);

        let data = vec![0u8; 100];
        let path = save_recording(tmp.to_str().unwrap(), "RE_TEST_123", &data)
            .await
            .expect("should save");

        assert!(std::path::Path::new(&path).exists());
        let saved = std::fs::read(&path).expect("should read");
        assert_eq!(saved.len(), 100);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_transcribe_recording_empty() {
        let result = transcribe_recording(&[]).await.expect("should succeed");
        assert!(result.is_empty());
    }

    #[test]
    fn test_select_whisper_no_keys() {
        // This test only works when no Whisper keys are set
        if std::env::var("GROQ_API_KEY").is_err() && std::env::var("OPENAI_API_KEY").is_err() {
            let result = select_whisper_provider();
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_inbound_call_twiml_strips_protocol() {
        let config = TwilioVoiceConfig {
            webhook_base_url: "http://localhost:8090".to_string(),
            auto_record: false,
            ..Default::default()
        };
        let twiml = inbound_call_twiml(&config);
        assert!(twiml.contains("wss://localhost:8090/voice/media-stream"));
        assert!(!twiml.contains("wss://http://"));
    }
}
