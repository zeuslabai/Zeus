//! Voice/Audio tools - Speech-to-Text and Text-to-Speech
//!
//! Provides two tools:
//! - `transcribe_audio` (STT): Transcribes audio files using Groq or OpenAI Whisper API
//! - `text_to_speech` (TTS): Converts text to speech using Kokoro (local), OpenAI TTS, or macOS `say`
//!
//! Provider priority for STT: Groq (GROQ_API_KEY) > OpenAI (OPENAI_API_KEY)
//! Provider priority for TTS: kokoro (KOKORO_API_URL) > openai > macos

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use std::env;
use zeus_core::{Error, Result, ToolSchema};

// ============================================================================
// TranscribeAudioTool (Speech-to-Text)
// ============================================================================

/// Speech-to-text tool using Groq or OpenAI Whisper API
pub struct TranscribeAudioTool;

/// Which STT provider to use
#[derive(Debug, Clone, PartialEq)]
pub enum SttProvider {
    /// Self-hosted whisper.cpp (ZEUS_WHISPER_URL, no auth)
    SelfHosted(String),
    Groq,
    OpenAI,
}

impl SttProvider {
    /// Select the best available provider based on environment variables.
    /// Priority: ZEUS_WHISPER_URL (self-hosted) > GROQ_API_KEY > OPENAI_API_KEY
    pub fn select() -> Result<(Self, String)> {
        if let Ok(url) = env::var("ZEUS_WHISPER_URL")
            && !url.is_empty()
        {
            return Ok((SttProvider::SelfHosted(url), String::new()));
        }
        if let Ok(key) = env::var("GROQ_API_KEY") {
            return Ok((SttProvider::Groq, key));
        }
        if let Ok(key) = env::var("OPENAI_API_KEY") {
            return Ok((SttProvider::OpenAI, key));
        }
        Err(Error::Tool(
            "No STT provider found. Set ZEUS_WHISPER_URL, GROQ_API_KEY, or OPENAI_API_KEY."
                .to_string(),
        ))
    }

    /// Get the API endpoint URL for this provider
    fn endpoint(&self) -> String {
        match self {
            SttProvider::SelfHosted(url) => url.clone(),
            SttProvider::Groq => "https://api.groq.com/openai/v1/audio/transcriptions".to_string(),
            SttProvider::OpenAI => "https://api.openai.com/v1/audio/transcriptions".to_string(),
        }
    }

    /// Get the model name for this provider
    fn model(&self) -> &'static str {
        match self {
            SttProvider::SelfHosted(_) => "whisper-large-v3",
            SttProvider::Groq => "whisper-large-v3",
            SttProvider::OpenAI => "whisper-1",
        }
    }

    /// Whether this provider needs an Authorization header
    fn needs_auth(&self) -> bool {
        !matches!(self, SttProvider::SelfHosted(_))
    }

    /// Get display name
    fn name(&self) -> &'static str {
        match self {
            SttProvider::SelfHosted(_) => "self-hosted",
            SttProvider::Groq => "groq",
            SttProvider::OpenAI => "openai",
        }
    }
}

#[async_trait]
impl TalosTool for TranscribeAudioTool {
    fn name(&self) -> &'static str {
        "transcribe_audio"
    }

    fn description(&self) -> &'static str {
        "Transcribe an audio file to text using Groq or OpenAI Whisper API"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "file_path",
                "string",
                "Path to the audio file to transcribe",
                true,
            )
            .with_param(
                "language",
                "string",
                "Language code (e.g., 'en', 'es', 'fr'). Default: 'en'",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing file_path parameter".to_string()))?;

        let language = args
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("en");

        // Select provider
        let (provider, api_key) = SttProvider::select()?;

        // Read the audio file
        let raw_data = tokio::fs::read(file_path).await.map_err(|e| {
            Error::Tool(format!("Failed to read audio file '{}': {}", file_path, e))
        })?;

        // Auto-convert ogg/opus to wav for self-hosted Whisper (can't read ogg)
        let (file_data, filename) = if (file_path.ends_with(".ogg") || file_path.ends_with(".opus"))
            && matches!(provider, SttProvider::SelfHosted(_))
        {
            let wav_path = format!("{}.wav", file_path);
            let output = tokio::process::Command::new("ffmpeg")
                .args(["-y", "-i", file_path, "-ar", "16000", "-ac", "1", &wav_path])
                .output()
                .await
                .map_err(|e| Error::Tool(format!("ffmpeg not found for ogg→wav conversion: {}", e)))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(Error::Tool(format!("ffmpeg ogg→wav conversion failed: {}", stderr)));
            }
            let wav_data = tokio::fs::read(&wav_path).await.map_err(|e| {
                Error::Tool(format!("Failed to read converted wav '{}': {}", wav_path, e))
            })?;
            let _ = tokio::fs::remove_file(&wav_path).await; // cleanup temp file
            (wav_data, "audio.wav".to_string())
        } else {
            let fname = std::path::Path::new(file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("audio.wav")
                .to_string();
            (raw_data, fname)
        };

        // Detect MIME type from content
        let mime_type = detect_audio_mime(&file_data, &filename);

        // Build multipart form
        let file_part = reqwest::multipart::Part::bytes(file_data)
            .file_name(filename)
            .mime_str(&mime_type)
            .map_err(|e| Error::Tool(format!("Failed to set MIME type: {}", e)))?;

        let form = reqwest::multipart::Form::new()
            .part("file", file_part)
            .text("model", provider.model().to_string())
            .text("language", language.to_string())
            .text("response_format", "json".to_string());

        // Send request
        let client = reqwest::Client::new();
        let mut req = client.post(provider.endpoint()).multipart(form);

        if provider.needs_auth() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }

        let response = req
            .send()
            .await
            .map_err(|e| Error::Tool(format!("STT API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "no response body".to_string());
            return Err(Error::Tool(format!(
                "STT API returned {}: {}",
                status, body
            )));
        }

        let resp_json: Value = response
            .json()
            .await
            .map_err(|e| Error::Tool(format!("Failed to parse STT response: {}", e)))?;

        let text = resp_json
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let duration = resp_json
            .get("duration")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let result = serde_json::json!({
            "text": text,
            "duration": duration,
            "provider": provider.name()
        });

        Ok(serde_json::to_string_pretty(&result)?)
    }
}

/// Detect audio MIME type from content and filename
fn detect_audio_mime(data: &[u8], filename: &str) -> String {
    // Try magic bytes first
    if data.len() >= 12 {
        if &data[..4] == b"fLaC" {
            return "audio/flac".to_string();
        }
        if &data[..3] == b"ID3" || (data[0] == 0xFF && (data[1] & 0xE0) == 0xE0) {
            return "audio/mpeg".to_string();
        }
        if &data[..4] == b"OggS" {
            return "audio/ogg".to_string();
        }
        if &data[..4] == b"RIFF" && &data[8..12] == b"WAVE" {
            return "audio/wav".to_string();
        }
    }

    // Fallback to extension
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "mp3" => "audio/mpeg".to_string(),
        "wav" => "audio/wav".to_string(),
        "flac" => "audio/flac".to_string(),
        "ogg" | "oga" => "audio/ogg".to_string(),
        "m4a" | "mp4" => "audio/mp4".to_string(),
        "webm" => "audio/webm".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

// ============================================================================
// TextToSpeechTool (Text-to-Speech)
// ============================================================================

/// Text-to-speech tool using Kokoro (local), OpenAI TTS, or macOS `say` command.
///
/// Provider priority: kokoro (local via KOKORO_API_URL) > openai > macos
pub struct TextToSpeechTool;

#[async_trait]
impl TalosTool for TextToSpeechTool {
    fn name(&self) -> &'static str {
        "text_to_speech"
    }

    fn description(&self) -> &'static str {
        "Convert text to speech audio. Providers: 'kokoro' (local), 'openai' (API), 'macos' (say command)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("text", "string", "Text to convert to speech", true)
            .with_param(
                "voice",
                "string",
                "Voice name. Kokoro: af_heart/af_bella/am_adam/bf_emma etc. OpenAI: alloy/coral/nova/echo/sage. macOS: system voices.",
                false,
            )
            .with_param(
                "output_path",
                "string",
                "Path to save the audio file. Default: ~/.zeus/media/tts_<timestamp>.wav",
                false,
            )
            .with_param(
                "provider",
                "string",
                "TTS provider: 'kokoro', 'openai', 'macos'. Default: auto-select (kokoro if running, else openai if API key, else macos)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing text parameter".to_string()))?;

        let voice = args.get("voice").and_then(|v| v.as_str());

        let output_path = args.get("output_path").and_then(|v| v.as_str());

        let provider = args
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");

        // Determine output path
        let out_path = match output_path {
            Some(p) => std::path::PathBuf::from(p),
            None => {
                let media_dir = dirs::home_dir()
                    .unwrap_or_default()
                    .join(".zeus")
                    .join("media");
                tokio::fs::create_dir_all(&media_dir)
                    .await
                    .map_err(|e| Error::Tool(format!("Failed to create media dir: {}", e)))?;
                let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
                media_dir.join(format!("tts_{}.wav", timestamp))
            }
        };

        // Select provider
        let selected_provider = match provider {
            "openai" => "openai",
            "macos" => "macos",
            "kokoro" => "kokoro",
            _ => {
                // Auto-select: kokoro > openai > macos
                if env::var("KOKORO_API_URL").is_ok() {
                    "kokoro"
                } else if env::var("OPENAI_API_KEY").is_ok() {
                    "openai"
                } else {
                    #[cfg(target_os = "macos")]
                    {
                        "macos"
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        return Err(Error::Tool(
                            "No TTS provider available. Set KOKORO_API_URL for local Kokoro, or OPENAI_API_KEY for OpenAI TTS."
                                .to_string(),
                        ));
                    }
                }
            }
        };

        match selected_provider {
            "kokoro" => tts_kokoro(text, voice, &out_path).await,
            "openai" => tts_openai(text, voice, &out_path).await,
            "macos" => tts_macos(text, voice, &out_path).await,
            _ => Err(Error::Tool(format!(
                "Unknown TTS provider: {}",
                selected_provider
            ))),
        }
    }
}

/// Kokoro local TTS implementation via Kokoro-FastAPI (OpenAI-compatible endpoint).
///
/// Requires a running Kokoro-FastAPI server. Set KOKORO_API_URL to the base URL
/// (default: http://localhost:8880). The server exposes an OpenAI-compatible
/// /v1/audio/speech endpoint.
async fn tts_kokoro(
    text: &str,
    voice: Option<&str>,
    output_path: &std::path::Path,
) -> Result<String> {
    let base_url = env::var("ZEUS_KOKORO_URL")
        .or_else(|_| env::var("KOKORO_API_URL"))
        .unwrap_or_else(|_| "http://localhost:8880".to_string());
    let url = format!("{}/v1/audio/speech", base_url.trim_end_matches('/'));

    let voice = voice.unwrap_or("af_heart");

    let body = serde_json::json!({
        "model": "kokoro",
        "input": text,
        "voice": voice,
        "response_format": "wav"
    });

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            Error::Tool(format!(
                "Kokoro TTS request failed (is Kokoro-FastAPI running at {}?): {}",
                base_url, e
            ))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "no response body".to_string());
        return Err(Error::Tool(format!(
            "Kokoro TTS API returned {}: {}",
            status, body
        )));
    }

    let audio_bytes = response
        .bytes()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read Kokoro TTS response: {}", e)))?;

    // Ensure parent directory exists
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| Error::Tool(format!("Failed to create output directory: {}", e)))?;
    }

    tokio::fs::write(output_path, &audio_bytes)
        .await
        .map_err(|e| Error::Tool(format!("Failed to write audio file: {}", e)))?;

    let result = serde_json::json!({
        "file_path": output_path.to_string_lossy(),
        "provider": "kokoro",
        "voice": voice,
        "size_bytes": audio_bytes.len()
    });

    Ok(serde_json::to_string_pretty(&result)?)
}

/// OpenAI TTS implementation
async fn tts_openai(
    text: &str,
    voice: Option<&str>,
    output_path: &std::path::Path,
) -> Result<String> {
    let api_key = env::var("OPENAI_API_KEY")
        .map_err(|_| Error::Tool("OPENAI_API_KEY not set".to_string()))?;

    let voice = voice.unwrap_or("nova");

    // Validate voice
    let valid_voices = [
        "alloy", "coral", "nova", "echo", "sage", "ash", "ballad", "shimmer", "verse",
    ];
    if !valid_voices.contains(&voice) {
        return Err(Error::Tool(format!(
            "Invalid OpenAI voice '{}'. Valid voices: {}",
            voice,
            valid_voices.join(", ")
        )));
    }

    let body = serde_json::json!({
        "model": "gpt-4o-mini-tts",
        "input": text,
        "voice": voice,
        "response_format": "wav"
    });

    let client = reqwest::Client::new();
    let response = client
        .post("https://api.openai.com/v1/audio/speech")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Tool(format!("OpenAI TTS request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "no response body".to_string());
        return Err(Error::Tool(format!(
            "OpenAI TTS API returned {}: {}",
            status, body
        )));
    }

    let audio_bytes = response
        .bytes()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read TTS response: {}", e)))?;

    // Ensure parent directory exists
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| Error::Tool(format!("Failed to create output directory: {}", e)))?;
    }

    tokio::fs::write(output_path, &audio_bytes)
        .await
        .map_err(|e| Error::Tool(format!("Failed to write audio file: {}", e)))?;

    let result = serde_json::json!({
        "file_path": output_path.to_string_lossy(),
        "provider": "openai",
        "voice": voice,
        "size_bytes": audio_bytes.len()
    });

    Ok(serde_json::to_string_pretty(&result)?)
}

/// macOS TTS implementation using the `say` command
async fn tts_macos(
    text: &str,
    voice: Option<&str>,
    output_path: &std::path::Path,
) -> Result<String> {
    // The `say` command outputs AIFF by default; we'll use it with -o flag
    // then convert to WAV using afconvert if the output path ends in .wav
    let wants_wav = output_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("wav"))
        .unwrap_or(false);

    let aiff_path = if wants_wav {
        output_path.with_extension("aiff")
    } else {
        output_path.to_path_buf()
    };

    // Ensure parent directory exists
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| Error::Tool(format!("Failed to create output directory: {}", e)))?;
    }

    // Build the say command
    let mut cmd = tokio::process::Command::new("say");
    cmd.arg("-o").arg(&aiff_path);

    if let Some(v) = voice {
        cmd.arg("-v").arg(v);
    }

    // Pass text via stdin to avoid shell escaping issues
    cmd.stdin(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Tool(format!("Failed to start 'say' command: {}", e)))?;

    if let Some(ref mut stdin) = child.stdin {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(text.as_bytes())
            .await
            .map_err(|e| Error::Tool(format!("Failed to write to say stdin: {}", e)))?;
    }

    let status = child
        .wait()
        .await
        .map_err(|e| Error::Tool(format!("Failed to wait for 'say' command: {}", e)))?;

    if !status.success() {
        return Err(Error::Tool(format!(
            "'say' command failed with exit code: {:?}",
            status.code()
        )));
    }

    // Convert AIFF to WAV if needed
    if wants_wav {
        let convert_status = tokio::process::Command::new("afconvert")
            .arg("-f")
            .arg("WAVE")
            .arg("-d")
            .arg("LEI16")
            .arg(&aiff_path)
            .arg(output_path)
            .status()
            .await
            .map_err(|e| Error::Tool(format!("Failed to run afconvert: {}", e)))?;

        // Clean up the intermediate AIFF file
        let _ = tokio::fs::remove_file(&aiff_path).await;

        if !convert_status.success() {
            return Err(Error::Tool(format!(
                "afconvert failed with exit code: {:?}",
                convert_status.code()
            )));
        }
    }

    let metadata = tokio::fs::metadata(output_path)
        .await
        .map_err(|e| Error::Tool(format!("Failed to read output file metadata: {}", e)))?;

    let result = serde_json::json!({
        "file_path": output_path.to_string_lossy(),
        "provider": "macos",
        "voice": voice.unwrap_or("default"),
        "size_bytes": metadata.len()
    });

    Ok(serde_json::to_string_pretty(&result)?)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    /// Serialize env-mutating tests to prevent parallel races.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_transcribe_audio_schema() {
        let tool = TranscribeAudioTool;
        let schema = tool.schema();

        assert_eq!(schema.name, "transcribe_audio");
        assert!(schema.description.contains("Transcribe"));

        let params = schema.parameters.as_object().expect("should be an object");
        let props = params
            .get("properties")
            .expect("key should exist")
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("file_path"));
        assert!(props.contains_key("language"));

        let required = params
            .get("required")
            .expect("key should exist")
            .as_array()
            .expect("should be an array");
        assert!(required.contains(&json!("file_path")));
        assert!(!required.contains(&json!("language")));
    }

    #[test]
    fn test_text_to_speech_schema() {
        let tool = TextToSpeechTool;
        let schema = tool.schema();

        assert_eq!(schema.name, "text_to_speech");
        assert!(schema.description.contains("speech"));

        let params = schema.parameters.as_object().expect("should be an object");
        let props = params
            .get("properties")
            .expect("key should exist")
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("text"));
        assert!(props.contains_key("voice"));
        assert!(props.contains_key("output_path"));
        assert!(props.contains_key("provider"));

        let required = params
            .get("required")
            .expect("key should exist")
            .as_array()
            .expect("should be an array");
        assert!(required.contains(&json!("text")));
        assert!(!required.contains(&json!("voice")));
        assert!(!required.contains(&json!("provider")));
    }

    /// Helper to set an env var in tests (unsafe in Rust 2024 edition)
    unsafe fn set_env(key: &str, val: &str) {
        unsafe { env::set_var(key, val) };
    }

    /// Helper to remove an env var in tests (unsafe in Rust 2024 edition)
    unsafe fn remove_env(key: &str) {
        unsafe { env::remove_var(key) };
    }

    /// Helper to save and restore env vars around a test
    unsafe fn restore_env(key: &str, orig: Option<String>) {
        match orig {
            Some(v) => unsafe { env::set_var(key, v) },
            None => unsafe { env::remove_var(key) },
        }
    }

    #[test]
    fn test_stt_provider_selection_self_hosted() {
        let _guard = ENV_LOCK.lock().unwrap();
        let orig_whisper = env::var("ZEUS_WHISPER_URL").ok();
        let orig_groq = env::var("GROQ_API_KEY").ok();
        let orig_openai = env::var("OPENAI_API_KEY").ok();

        unsafe {
            set_env("ZEUS_WHISPER_URL", "http://localhost:8080/inference");
            set_env("GROQ_API_KEY", "test-groq-key");
        }

        let result = SttProvider::select();
        assert!(result.is_ok());
        let (provider, _key) = result.expect("operation should succeed");
        assert_eq!(provider.name(), "self-hosted");
        assert_eq!(provider.model(), "whisper-large-v3");
        assert!(provider.endpoint().contains("localhost"));
        assert!(!provider.needs_auth());

        unsafe {
            restore_env("ZEUS_WHISPER_URL", orig_whisper);
            restore_env("GROQ_API_KEY", orig_groq);
            restore_env("OPENAI_API_KEY", orig_openai);
        }
    }

    #[test]
    fn test_stt_provider_selection_groq() {
        let _guard = ENV_LOCK.lock().unwrap();
        let orig_whisper = env::var("ZEUS_WHISPER_URL").ok();
        let orig_groq = env::var("GROQ_API_KEY").ok();
        let orig_openai = env::var("OPENAI_API_KEY").ok();

        unsafe {
            remove_env("ZEUS_WHISPER_URL");
            set_env("GROQ_API_KEY", "test-groq-key");
            remove_env("OPENAI_API_KEY");
        }

        let result = SttProvider::select();
        assert!(result.is_ok());
        let (provider, key) = result.expect("operation should succeed");
        assert_eq!(provider, SttProvider::Groq);
        assert_eq!(key, "test-groq-key");
        assert_eq!(provider.name(), "groq");
        assert_eq!(provider.model(), "whisper-large-v3");
        assert!(provider.endpoint().contains("groq.com"));
        assert!(provider.needs_auth());

        unsafe {
            restore_env("ZEUS_WHISPER_URL", orig_whisper);
            restore_env("GROQ_API_KEY", orig_groq);
            restore_env("OPENAI_API_KEY", orig_openai);
        }
    }

    #[test]
    fn test_stt_provider_selection_openai_fallback() {
        let _guard = ENV_LOCK.lock().unwrap();
        let orig_whisper = env::var("ZEUS_WHISPER_URL").ok();
        let orig_groq = env::var("GROQ_API_KEY").ok();
        let orig_openai = env::var("OPENAI_API_KEY").ok();

        unsafe {
            remove_env("ZEUS_WHISPER_URL");
            remove_env("GROQ_API_KEY");
            set_env("OPENAI_API_KEY", "test-openai-key");
        }

        let result = SttProvider::select();
        assert!(result.is_ok());
        let (provider, key) = result.expect("operation should succeed");
        assert_eq!(provider, SttProvider::OpenAI);
        assert_eq!(key, "test-openai-key");
        assert_eq!(provider.name(), "openai");
        assert_eq!(provider.model(), "whisper-1");
        assert!(provider.endpoint().contains("openai.com"));
        assert!(provider.needs_auth());

        unsafe {
            restore_env("ZEUS_WHISPER_URL", orig_whisper);
            restore_env("GROQ_API_KEY", orig_groq);
            restore_env("OPENAI_API_KEY", orig_openai);
        }
    }

    #[test]
    fn test_stt_provider_selection_no_keys() {
        let _guard = ENV_LOCK.lock().unwrap();
        let orig_whisper = env::var("ZEUS_WHISPER_URL").ok();
        let orig_groq = env::var("GROQ_API_KEY").ok();
        let orig_openai = env::var("OPENAI_API_KEY").ok();

        unsafe {
            remove_env("ZEUS_WHISPER_URL");
            remove_env("GROQ_API_KEY");
            remove_env("OPENAI_API_KEY");
        }

        let result = SttProvider::select();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No STT provider"));

        unsafe {
            restore_env("ZEUS_WHISPER_URL", orig_whisper);
            restore_env("GROQ_API_KEY", orig_groq);
            restore_env("OPENAI_API_KEY", orig_openai);
        }
    }

    #[test]
    fn test_detect_audio_mime_magic_bytes() {
        // WAV
        let wav = [
            0x52u8, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00, 0x57, 0x41, 0x56, 0x45,
        ];
        assert_eq!(detect_audio_mime(&wav, "audio.wav"), "audio/wav");

        // FLAC
        let flac = [
            0x66u8, 0x4C, 0x61, 0x43, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(detect_audio_mime(&flac, "audio.flac"), "audio/flac");

        // MP3 with ID3 tag
        let mp3 = [
            0x49u8, 0x44, 0x33, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(detect_audio_mime(&mp3, "audio.mp3"), "audio/mpeg");

        // OGG
        let ogg = [
            0x4Fu8, 0x67, 0x67, 0x53, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(detect_audio_mime(&ogg, "audio.ogg"), "audio/ogg");
    }

    #[test]
    fn test_detect_audio_mime_by_extension() {
        let unknown = [0x00u8; 4];
        assert_eq!(detect_audio_mime(&unknown, "song.mp3"), "audio/mpeg");
        assert_eq!(detect_audio_mime(&unknown, "clip.wav"), "audio/wav");
        assert_eq!(detect_audio_mime(&unknown, "track.flac"), "audio/flac");
        assert_eq!(detect_audio_mime(&unknown, "voice.m4a"), "audio/mp4");
        assert_eq!(detect_audio_mime(&unknown, "file.webm"), "audio/webm");
        assert_eq!(
            detect_audio_mime(&unknown, "data.xyz"),
            "application/octet-stream"
        );
    }

    #[tokio::test]
    async fn test_transcribe_missing_file_path() {
        let tool = TranscribeAudioTool;
        let args = json!({"language": "en"});
        let result = tool.execute(args).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Missing file_path")
        );
    }

    #[tokio::test]
    async fn test_tts_missing_text() {
        let tool = TextToSpeechTool;
        let args = json!({"provider": "macos"});
        let result = tool.execute(args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Missing text"));
    }

    #[test]
    fn test_stt_provider_groq_priority_over_openai() {
        let _guard = ENV_LOCK.lock().unwrap();
        let orig_whisper = env::var("ZEUS_WHISPER_URL").ok();
        let orig_groq = env::var("GROQ_API_KEY").ok();
        let orig_openai = env::var("OPENAI_API_KEY").ok();

        // Both cloud keys set, no self-hosted — Groq should win
        unsafe {
            remove_env("ZEUS_WHISPER_URL");
            set_env("GROQ_API_KEY", "groq-key");
            set_env("OPENAI_API_KEY", "openai-key");
        }

        let result = SttProvider::select();
        assert!(result.is_ok());
        let (provider, _) = result.expect("operation should succeed");
        assert_eq!(provider, SttProvider::Groq);

        // Restore
        unsafe {
            restore_env("ZEUS_WHISPER_URL", orig_whisper);
            restore_env("GROQ_API_KEY", orig_groq);
            restore_env("OPENAI_API_KEY", orig_openai);
        }
    }
}
