//! No-audio fallback for builds compiled without local cpal audio support.
//!
//! The real cpal implementation is behind the default-on `audio` feature and is
//! unavailable on targets where the cpal/ALSA chain is intentionally omitted.
//! These stubs preserve the public API and fail gracefully at runtime.

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::talk_mode::{AudioInputProvider, AudioOutputProvider};

const AUDIO_NOT_BUILT_IN: &str = "audio not built in";

/// Placeholder local microphone input provider for no-audio builds.
#[derive(Debug, Default, Clone, Copy)]
pub struct CpalAudioInput;

impl CpalAudioInput {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl AudioInputProvider for CpalAudioInput {
    async fn start_capture(&self) -> zeus_core::Result<mpsc::Receiver<Vec<i16>>> {
        Err(zeus_core::Error::Internal(AUDIO_NOT_BUILT_IN.to_string()))
    }

    async fn stop_capture(&self) {}

    fn is_available(&self) -> bool {
        false
    }
}

/// Placeholder local speaker output provider for no-audio builds.
#[derive(Debug, Default, Clone, Copy)]
pub struct CpalAudioOutput;

impl CpalAudioOutput {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl AudioOutputProvider for CpalAudioOutput {
    async fn play_wav(&self, _wav_data: &[u8]) -> zeus_core::Result<()> {
        Err(zeus_core::Error::Internal(AUDIO_NOT_BUILT_IN.to_string()))
    }

    async fn stop_playback(&self) {}

    fn is_playing(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn input_reports_audio_not_built_in() {
        let input = CpalAudioInput::new();
        assert!(!input.is_available());
        let err = input.start_capture().await.unwrap_err().to_string();
        assert!(err.contains(AUDIO_NOT_BUILT_IN));
    }

    #[tokio::test]
    async fn output_reports_audio_not_built_in() {
        let output = CpalAudioOutput::new();
        assert!(!output.is_playing());
        let err = output.play_wav(&[]).await.unwrap_err().to_string();
        assert!(err.contains(AUDIO_NOT_BUILT_IN));
    }
}
