//! Microsoft Edge TTS provider
//!
//! Uses the free Microsoft Edge TTS service. No API key required.
//!
//! The full WebSocket-based synthesis protocol is non-trivial; this
//! implementation currently delegates to the `edge-tts` CLI tool when
//! available. The [`synthesize`] method returns a clear error if the
//! CLI is not found.

use async_trait::async_trait;
use tracing::instrument;

use crate::{AudioFormat, TTSError, TTSProvider, TTSResponse, Voice};

/// Microsoft Edge TTS provider (free, no API key).
pub struct EdgeTTSProvider {
    _private: (),
}

impl EdgeTTSProvider {
    /// Create a new Edge TTS provider.
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for EdgeTTSProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Static list of commonly used Edge TTS voices.
const EDGE_VOICES: &[(&str, &str, &str, &str)] = &[
    ("en-US-GuyNeural", "Guy (US)", "male", "en-US"),
    ("en-US-JennyNeural", "Jenny (US)", "female", "en-US"),
    ("en-US-AriaNeural", "Aria (US)", "female", "en-US"),
    ("en-US-DavisNeural", "Davis (US)", "male", "en-US"),
    ("en-US-AmberNeural", "Amber (US)", "female", "en-US"),
    ("en-US-AnaNeural", "Ana (US)", "female", "en-US"),
    ("en-US-BrandonNeural", "Brandon (US)", "male", "en-US"),
    (
        "en-US-ChristopherNeural",
        "Christopher (US)",
        "male",
        "en-US",
    ),
    ("en-GB-SoniaNeural", "Sonia (UK)", "female", "en-GB"),
    ("en-GB-RyanNeural", "Ryan (UK)", "male", "en-GB"),
    ("en-AU-NatashaNeural", "Natasha (AU)", "female", "en-AU"),
    ("en-AU-WilliamNeural", "William (AU)", "male", "en-AU"),
];

#[async_trait]
impl TTSProvider for EdgeTTSProvider {
    fn id(&self) -> &str {
        "edge"
    }

    fn name(&self) -> &str {
        "Microsoft Edge TTS"
    }

    fn is_configured(&self) -> bool {
        // No credentials needed
        true
    }

    async fn voices(&self) -> Result<Vec<Voice>, TTSError> {
        Ok(EDGE_VOICES
            .iter()
            .map(|(id, name, gender, lang)| Voice {
                id: id.to_string(),
                name: name.to_string(),
                gender: Some(gender.to_string()),
                language: Some(lang.to_string()),
                preview_url: None,
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
        // Attempt to use the edge-tts CLI
        let format_arg = match format {
            AudioFormat::Mp3 => "audio-24khz-48kbitrate-mono-mp3",
            AudioFormat::Wav => "riff-24khz-16bit-mono-pcm",
            AudioFormat::Opus => "webm-24khz-16bit-mono-opus",
        };

        let output_file = format!("/tmp/zeus-edge-tts-{}.{}", uuid::Uuid::new_v4(), format);

        let result = tokio::process::Command::new("edge-tts")
            .arg("--voice")
            .arg(voice)
            .arg("--text")
            .arg(text)
            .arg("--codec")
            .arg(format_arg)
            .arg("--write-media")
            .arg(&output_file)
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() => {
                let audio = tokio::fs::read(&output_file).await.map_err(|e| {
                    TTSError::SynthesisFailed(format!("failed to read output: {e}"))
                })?;
                // Best-effort cleanup
                let _ = tokio::fs::remove_file(&output_file).await;
                Ok(TTSResponse {
                    audio,
                    format,
                    duration_ms: None,
                    provider: "edge".to_string(),
                    voice: voice.to_string(),
                })
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(TTSError::SynthesisFailed(format!(
                    "edge-tts failed: {stderr}"
                )))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(TTSError::SynthesisFailed(
                "Edge TTS synthesis requires the `edge-tts` CLI tool. \
                     Install it with: pip install edge-tts"
                    .to_string(),
            )),
            Err(e) => Err(TTSError::SynthesisFailed(format!(
                "failed to spawn edge-tts: {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_impl() {
        let p = EdgeTTSProvider::default();
        assert!(p.is_configured());
    }

    #[tokio::test]
    async fn test_voices_have_language() {
        let p = EdgeTTSProvider::new();
        let voices = p.voices().await.expect("async operation should succeed");
        for v in &voices {
            assert!(v.language.is_some(), "voice {} missing language", v.id);
        }
    }
}
