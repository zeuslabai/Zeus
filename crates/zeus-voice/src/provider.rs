//! Voice call provider trait

use async_trait::async_trait;
use zeus_core::Result;

use crate::call::CallState;

/// Trait for voice call providers
#[async_trait]
pub trait VoiceCallProvider: Send + Sync {
    /// Initiate a call to a phone number
    /// Returns a call SID/ID
    async fn initiate_call(&self, to: &str, greeting_text: &str) -> Result<String>;

    /// Hang up an active call
    async fn hangup_call(&self, call_id: &str) -> Result<()>;

    /// Play TTS audio on an active call
    async fn play_tts(&self, call_id: &str, text: &str) -> Result<()>;

    /// Get the current state of a call
    async fn get_call_state(&self, call_id: &str) -> Result<CallState>;

    /// Send DTMF tones on an active call
    async fn send_dtmf(&self, call_id: &str, digits: &str) -> Result<()>;

    /// Get the provider name
    fn provider_name(&self) -> &'static str;
}
