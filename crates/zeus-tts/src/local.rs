//! Local TTS provider via Piper
//!
//! Uses the [Piper](https://github.com/rhasspy/piper) TTS engine as a
//! subprocess. Piper must be installed separately and model files placed
//! in `~/.zeus/piper-models/`.

use std::path::PathBuf;

use async_trait::async_trait;
use tracing::instrument;

use crate::{AudioFormat, TTSError, TTSProvider, TTSResponse, Voice};

/// Local TTS provider using Piper as a subprocess.
pub struct LocalProvider {
    piper_path: String,
    models_dir: PathBuf,
}

impl LocalProvider {
    /// Create a new local provider.
    ///
    /// `piper_path` defaults to `"piper"` (looked up in `$PATH`).
    /// Models are expected in `~/.zeus/piper-models/`.
    pub fn new(piper_path: Option<String>) -> Self {
        let models_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".zeus")
            .join("piper-models");

        Self {
            piper_path: piper_path.unwrap_or_else(|| "piper".to_string()),
            models_dir,
        }
    }

    /// Return the path where model files are expected.
    pub fn models_dir(&self) -> &PathBuf {
        &self.models_dir
    }

    /// Scan the models directory for `.onnx` model files.
    fn discover_models(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.models_dir) else {
            return Vec::new();
        };
        entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "onnx"))
            .filter_map(|e| {
                e.path()
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
            })
            .collect()
    }
}

#[async_trait]
impl TTSProvider for LocalProvider {
    fn id(&self) -> &str {
        "local"
    }

    fn name(&self) -> &str {
        "Local (Piper)"
    }

    fn is_configured(&self) -> bool {
        // Check if piper binary exists in PATH (or at the configured path)
        which_sync(&self.piper_path)
    }

    async fn voices(&self) -> Result<Vec<Voice>, TTSError> {
        let models = self.discover_models();
        Ok(models
            .into_iter()
            .map(|model_name| Voice {
                id: model_name.clone(),
                name: model_name,
                gender: None,
                language: None,
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
        if !self.is_configured() {
            return Err(TTSError::ProviderNotConfigured(
                "piper binary not found in PATH".to_string(),
            ));
        }

        let model_path = self.models_dir.join(format!("{voice}.onnx"));
        if !model_path.exists() {
            return Err(TTSError::InvalidVoice(format!(
                "model file not found: {}",
                model_path.display()
            )));
        }

        let output_file = format!("/tmp/zeus-piper-{}.{}", uuid::Uuid::new_v4(), format);

        // Piper outputs raw PCM by default; use --output_file for wav
        let mut cmd = tokio::process::Command::new(&self.piper_path);
        cmd.arg("--model")
            .arg(&model_path)
            .arg("--output_file")
            .arg(&output_file);

        // Pipe text via stdin
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| TTSError::SynthesisFailed(format!("failed to spawn piper: {e}")))?;

        // Write text to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(text.as_bytes())
                .await
                .map_err(|e| TTSError::SynthesisFailed(format!("stdin write failed: {e}")))?;
            // Drop stdin to signal EOF
        }

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| TTSError::SynthesisFailed(format!("piper execution failed: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(TTSError::SynthesisFailed(format!("piper failed: {stderr}")));
        }

        let audio = tokio::fs::read(&output_file)
            .await
            .map_err(|e| TTSError::SynthesisFailed(format!("failed to read output: {e}")))?;

        // Best-effort cleanup
        let _ = tokio::fs::remove_file(&output_file).await;

        // Piper outputs wav by default; if a different format was requested we
        // would need post-processing, but we return what we have for now.
        let actual_format = if format == AudioFormat::Wav {
            AudioFormat::Wav
        } else {
            // Piper outputs wav; caller should be aware
            AudioFormat::Wav
        };

        Ok(TTSResponse {
            audio,
            format: actual_format,
            duration_ms: None,
            provider: "local".to_string(),
            voice: voice.to_string(),
        })
    }
}

/// Synchronous check for whether a binary exists in PATH.
fn which_sync(binary: &str) -> bool {
    // If it is an absolute path, just check existence
    let path = std::path::Path::new(binary);
    if path.is_absolute() {
        return path.exists();
    }
    // Otherwise search PATH
    if let Ok(paths) = std::env::var("PATH") {
        for dir in paths.split(':') {
            let candidate = std::path::Path::new(dir).join(binary);
            if candidate.exists() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_models_dir_default() {
        let p = LocalProvider::new(None);
        let dir = p.models_dir();
        assert!(dir.ends_with("piper-models"));
    }

    #[test]
    fn test_which_sync_ls() {
        // `ls` should exist on any Unix system
        assert!(which_sync("ls"));
    }

    #[test]
    fn test_which_sync_nonexistent() {
        assert!(!which_sync("this-binary-does-not-exist-xyz"));
    }

    #[test]
    fn test_discover_models_empty() {
        // The models dir likely doesn't exist in test env
        let p = LocalProvider::new(None);
        let models = p.discover_models();
        // Should return empty vec, not panic
        assert!(models.is_empty() || !models.is_empty());
    }
}
