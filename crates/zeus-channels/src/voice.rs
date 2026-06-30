//! Voice channel adapter
//!
//! Bridges zeus-voice (Twilio telephony) into the channel system.
//! Voice calls appear as a channel where:
//! - Incoming call transcripts become `ChannelMessage`s
//! - Outbound `send()` calls either play TTS on an active call or initiate a new one
//! - The adapter runs a webhook server to receive Twilio status callbacks and media streams

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use zeus_core::{Error, Result};

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};

use zeus_voice::VoiceConfig;
use zeus_voice::call::{CallManager, CallState, TranscriptEntry};
use zeus_voice::provider::VoiceCallProvider;
use zeus_voice::twilio::TwilioProvider;
use zeus_voice::webhook::WebhookServer;

// ============================================================================
// Configuration
// ============================================================================

/// Voice channel configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceChannelConfig {
    /// Twilio Account SID
    #[serde(default)]
    pub account_sid: String,

    /// Twilio Auth Token
    #[serde(default)]
    pub auth_token: String,

    /// Phone number to call from (E.164 format)
    #[serde(default)]
    pub from_number: String,

    /// Base URL for webhooks (must be publicly accessible, e.g. ngrok URL)
    #[serde(default = "default_webhook_base_url")]
    pub webhook_base_url: String,

    /// Port for the webhook server
    #[serde(default = "default_webhook_port")]
    pub webhook_port: u16,

    /// TTS voice for calls (Twilio voice name)
    #[serde(default = "default_tts_voice")]
    pub tts_voice: String,

    /// Greeting message for incoming calls
    #[serde(default = "default_greeting")]
    pub incoming_greeting: String,

    /// Poll interval for checking active call transcripts (ms)
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

fn default_webhook_base_url() -> String {
    std::env::var("ZEUS_WEBHOOK_URL").unwrap_or_else(|_| "http://localhost:8090".to_string())
}

fn default_webhook_port() -> u16 {
    8090
}

fn default_tts_voice() -> String {
    "Polly.Amy".to_string()
}

fn default_greeting() -> String {
    "Hello, you've reached Zeus. How can I help you?".to_string()
}

fn default_poll_interval_ms() -> u64 {
    2000
}

impl Default for VoiceChannelConfig {
    fn default() -> Self {
        Self {
            account_sid: String::new(),
            auth_token: String::new(),
            from_number: String::new(),
            webhook_base_url: default_webhook_base_url(),
            webhook_port: default_webhook_port(),
            tts_voice: default_tts_voice(),
            incoming_greeting: default_greeting(),
            poll_interval_ms: default_poll_interval_ms(),
        }
    }
}

impl VoiceChannelConfig {
    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        if self.account_sid.is_empty() {
            return Err(Error::Channel(
                "Voice channel: account_sid required".to_string(),
            ));
        }
        if self.auth_token.is_empty() {
            return Err(Error::Channel(
                "Voice channel: auth_token required".to_string(),
            ));
        }
        if self.from_number.is_empty() {
            return Err(Error::Channel(
                "Voice channel: from_number required".to_string(),
            ));
        }
        Ok(())
    }

    /// Load from environment variables (falls back to config values)
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(sid) = std::env::var("TWILIO_ACCOUNT_SID") {
            self.account_sid = sid;
        }
        if let Ok(token) = std::env::var("TWILIO_AUTH_TOKEN") {
            self.auth_token = token;
        }
        if let Ok(phone) = std::env::var("TWILIO_PHONE_NUMBER") {
            self.from_number = phone;
        }
        if let Ok(url) = std::env::var("TWILIO_WEBHOOK_URL") {
            self.webhook_base_url = url;
        }
        self
    }

    /// Convert to zeus-voice VoiceConfig
    fn to_voice_config(&self) -> VoiceConfig {
        VoiceConfig {
            provider: "twilio".to_string(),
            account_sid: self.account_sid.clone(),
            auth_token: self.auth_token.clone(),
            from_number: self.from_number.clone(),
            webhook_base_url: self.webhook_base_url.clone(),
            webhook_port: self.webhook_port,
            tts_voice: self.tts_voice.clone(),
            ..Default::default()
        }
    }
}

// ============================================================================
// Adapter
// ============================================================================

/// Voice channel adapter
///
/// Wraps zeus-voice to provide voice calls as a messaging channel.
/// Incoming call audio is transcribed and delivered as ChannelMessages.
/// Outbound messages are spoken via TTS on active calls or trigger new calls.
pub struct VoiceAdapter {
    config: VoiceChannelConfig,
    call_manager: Arc<CallManager>,
    provider: Arc<TwilioProvider>,
    connected: Arc<AtomicBool>,
}

impl VoiceAdapter {
    /// Create a new voice adapter
    pub fn new(config: VoiceChannelConfig) -> Self {
        let voice_config = config.to_voice_config();
        let provider = Arc::new(TwilioProvider::new(voice_config));
        let call_manager = Arc::new(CallManager::new());

        Self {
            config,
            call_manager,
            provider,
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Get the call manager for external access
    pub fn call_manager(&self) -> &Arc<CallManager> {
        &self.call_manager
    }

    /// Get the provider for external access
    pub fn provider(&self) -> &Arc<TwilioProvider> {
        &self.provider
    }

    /// Initiate an outbound call
    pub async fn call(&self, to: &str, greeting: &str) -> Result<String> {
        let call_sid = self.provider.initiate_call(to, greeting).await?;
        self.call_manager
            .register_call(call_sid.clone(), to.to_string())
            .await;
        Ok(call_sid)
    }

    /// Hang up a call
    pub async fn hangup(&self, call_id: &str) -> Result<()> {
        self.provider.hangup_call(call_id).await?;
        self.call_manager
            .update_state(call_id, CallState::Completed)
            .await;
        Ok(())
    }

    /// Get active calls
    pub async fn active_calls(&self) -> Vec<zeus_voice::call::CallRecord> {
        self.call_manager.active_calls().await
    }
}

#[async_trait]
impl ChannelAdapter for VoiceAdapter {
    fn channel_type(&self) -> &'static str {
        "voice"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Webhook {
            path: "/voice".to_string(),
        }
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        self.config.validate()?;

        // Start the webhook server for Twilio callbacks
        let webhook = WebhookServer::new(
            self.config.webhook_port,
            self.call_manager.clone(),
            &self.config.webhook_base_url,
            &self.config.tts_voice,
            &self.config.incoming_greeting,
        );

        webhook
            .start()
            .await
            .map_err(|e| Error::Channel(format!("Failed to start voice webhook server: {}", e)))?;

        info!(
            port = self.config.webhook_port,
            "Voice channel webhook server started"
        );

        self.connected.store(true, Ordering::SeqCst);

        // Spawn a background task that monitors calls for new transcript entries
        // and converts them into ChannelMessages
        let call_manager = self.call_manager.clone();
        let poll_interval = self.config.poll_interval_ms;
        let connected = self.connected.clone();

        tokio::spawn(async move {
            let mut last_transcript_counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();

            while connected.load(Ordering::SeqCst) {
                let active = call_manager.active_calls().await;

                for call in &active {
                    let prev_count = last_transcript_counts
                        .get(&call.call_id)
                        .copied()
                        .unwrap_or(0);

                    if call.transcript.len() > prev_count {
                        // New transcript entries — send them as ChannelMessages
                        for entry in &call.transcript[prev_count..] {
                            if entry.speaker == "user" {
                                let source = ChannelSource {
                                    channel_type: "voice".to_string(),
                                    user_id: call.to_number.clone(),
                                    chat_id: Some(call.call_id.clone()),
                                    account_id: None,
                                    thread_id: None,
                                    reply_to_message_id: None,
                                    sender_type: zeus_core::SenderType::Human,
                                };

                                let msg = ChannelMessage::new(source, entry.text.clone());

                                if let Err(e) = tx.send(msg).await {
                                    error!("Failed to send voice channel message: {}", e);
                                    return;
                                }

                                debug!(
                                    call_id = %call.call_id,
                                    text = %entry.text,
                                    "Voice transcript → ChannelMessage"
                                );
                            }
                        }
                        last_transcript_counts.insert(call.call_id.clone(), call.transcript.len());
                    }
                }

                // Clean up completed calls from tracking
                last_transcript_counts.retain(|id, _| active.iter().any(|c| c.call_id == *id));

                tokio::time::sleep(tokio::time::Duration::from_millis(poll_interval)).await;
            }

            info!("Voice channel transcript monitor stopped");
        });

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        info!("Voice channel stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        // If there's a call_id (chat_id), play TTS on the active call
        if let Some(ref call_id) = to.chat_id {
            // Check if call is still active
            if let Some(call) = self.call_manager.get_call(call_id).await
                && call.state.is_active()
            {
                self.provider.play_tts(call_id, content).await?;
                self.call_manager
                    .add_transcript(call_id, TranscriptEntry::agent(content))
                    .await;
                debug!(call_id = %call_id, "Played TTS on active call");
                return Ok(());
            }
            warn!(call_id = %call_id, "Call not active, initiating new call");
        }

        // No active call — initiate a new one with the content as the greeting
        let phone = &to.user_id;
        if phone.is_empty() {
            return Err(Error::Channel(
                "Voice channel: phone number required in user_id".to_string(),
            ));
        }

        let call_sid = self.provider.initiate_call(phone, content).await?;
        self.call_manager
            .register_call(call_sid.clone(), phone.to_string())
            .await;

        info!(call_sid = %call_sid, to = %phone, "Initiated outbound voice call");
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn handle_webhook(
        &self,
        payload: &[u8],
        tx: &mpsc::Sender<ChannelMessage>,
    ) -> Result<()> {
        // Parse Twilio webhook form data (URL-encoded key=value pairs)
        let body = String::from_utf8_lossy(payload);
        let params: Vec<(String, String)> = body
            .split('&')
            .filter_map(|pair| {
                let mut parts = pair.splitn(2, '=');
                let key = parts.next()?;
                let value = parts.next().unwrap_or("");
                Some((
                    urlencoding::decode(key).unwrap_or_default().to_string(),
                    urlencoding::decode(value).unwrap_or_default().to_string(),
                ))
            })
            .collect();

        let call_sid = params
            .iter()
            .find(|(k, _)| k == "CallSid")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();

        let call_status = params
            .iter()
            .find(|(k, _)| k == "CallStatus")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();

        let from = params
            .iter()
            .find(|(k, _)| k == "From")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();

        debug!(
            call_sid = %call_sid,
            status = %call_status,
            from = %from,
            "Voice webhook received"
        );

        // Update call state
        let new_state = CallState::from_twilio_status(&call_status);
        self.call_manager
            .update_state(&call_sid, new_state.clone())
            .await;

        // If this is a new incoming call, register it and send a notification
        if call_status == "ringing" && !from.is_empty() {
            self.call_manager
                .register_call(call_sid.clone(), from.clone())
                .await;

            let source = ChannelSource {
                channel_type: "voice".to_string(),
                user_id: from,
                chat_id: Some(call_sid),
                account_id: None,
                thread_id: None,
                reply_to_message_id: None,
                sender_type: zeus_core::SenderType::Human,
            };

            let msg = ChannelMessage::new(source, "[Incoming voice call]".to_string());
            if let Err(e) = tx.send(msg).await {
                error!("Failed to send incoming call notification: {}", e);
            }
        }

        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> VoiceChannelConfig {
        VoiceChannelConfig {
            account_sid: "AC_TEST".to_string(),
            auth_token: "test_token".to_string(),
            from_number: "+15551234567".to_string(),
            webhook_base_url: "https://example.ngrok.io".to_string(),
            webhook_port: 8090,
            tts_voice: "Polly.Amy".to_string(),
            incoming_greeting: "Hello!".to_string(),
            poll_interval_ms: 1000,
        }
    }

    #[test]
    fn test_config_default() {
        let config = VoiceChannelConfig::default();
        assert_eq!(config.webhook_port, 8090);
        assert_eq!(config.tts_voice, "Polly.Amy");
        assert!(config.account_sid.is_empty());
    }

    #[test]
    fn test_config_validate_missing_sid() {
        let mut config = test_config();
        config.account_sid = String::new();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validate_missing_token() {
        let mut config = test_config();
        config.auth_token = String::new();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validate_missing_number() {
        let mut config = test_config();
        config.from_number = String::new();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validate_success() {
        let config = test_config();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_to_voice_config() {
        let config = test_config();
        let vc = config.to_voice_config();
        assert_eq!(vc.account_sid, "AC_TEST");
        assert_eq!(vc.from_number, "+15551234567");
        assert_eq!(vc.webhook_port, 8090);
    }

    #[test]
    fn test_config_serialization() {
        let config = test_config();
        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        let parsed: VoiceChannelConfig =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.account_sid, config.account_sid);
        assert_eq!(parsed.from_number, config.from_number);
        assert_eq!(parsed.webhook_port, config.webhook_port);
    }

    #[test]
    fn test_adapter_channel_type() {
        let adapter = VoiceAdapter::new(test_config());
        assert_eq!(adapter.channel_type(), "voice");
    }

    #[test]
    fn test_adapter_receive_mode() {
        let adapter = VoiceAdapter::new(test_config());
        let mode = adapter.receive_mode();
        match mode {
            ReceiveMode::Webhook { path } => assert_eq!(path, "/voice"),
            _ => panic!("Expected Webhook receive mode"),
        }
    }

    #[test]
    fn test_adapter_not_connected_initially() {
        let adapter = VoiceAdapter::new(test_config());
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_adapter_send_empty_phone() {
        let adapter = VoiceAdapter::new(test_config());
        let source = ChannelSource::new("voice", "");
        let result = adapter.send(&source, "Hello").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_adapter_active_calls_empty() {
        let adapter = VoiceAdapter::new(test_config());
        let calls = adapter.active_calls().await;
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn test_adapter_stop() {
        let adapter = VoiceAdapter::new(test_config());
        adapter.connected.store(true, Ordering::SeqCst);
        assert!(adapter.is_connected());

        adapter
            .stop()
            .await
            .expect("async operation should succeed");
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_call_manager_integration() {
        let adapter = VoiceAdapter::new(test_config());
        let cm = adapter.call_manager();

        // Register a call directly via the call manager
        cm.register_call("CA123".to_string(), "+15559876543".to_string())
            .await;

        let call = cm.get_call("CA123").await;
        assert!(call.is_some());
        assert_eq!(
            call.expect("operation should succeed").to_number,
            "+15559876543"
        );
    }

    #[tokio::test]
    async fn test_handle_webhook_ringing() {
        let adapter = VoiceAdapter::new(test_config());
        let (tx, mut rx) = mpsc::channel(10);

        // Simulate a Twilio ringing webhook
        let payload = b"CallSid=CA123&CallStatus=ringing&From=%2B15559876543&To=%2B15551234567";
        adapter
            .handle_webhook(payload, &tx)
            .await
            .expect("async operation should succeed");

        // Should have sent an incoming call notification
        let msg = rx.try_recv().expect("try_recv should succeed");
        assert_eq!(msg.source.channel_type, "voice");
        assert_eq!(msg.source.user_id, "+15559876543");
        assert_eq!(msg.source.chat_id, Some("CA123".to_string()));
        assert_eq!(msg.content, "[Incoming voice call]");
    }

    #[tokio::test]
    async fn test_handle_webhook_completed() {
        let adapter = VoiceAdapter::new(test_config());
        let (tx, mut rx) = mpsc::channel(10);

        // Register a call first
        adapter
            .call_manager
            .register_call("CA456".to_string(), "+15559876543".to_string())
            .await;

        // Simulate a completed webhook
        let payload = b"CallSid=CA456&CallStatus=completed&From=%2B15559876543";
        adapter
            .handle_webhook(payload, &tx)
            .await
            .expect("async operation should succeed");

        // Call should be in Completed state
        let call = adapter
            .call_manager
            .get_call("CA456")
            .await
            .expect("async operation should succeed");
        assert_eq!(call.state, CallState::Completed);

        // No new message for status updates (only ringing triggers notification)
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_send_on_active_call() {
        let adapter = VoiceAdapter::new(test_config());

        // Register and activate a call
        adapter
            .call_manager
            .register_call("CA789".to_string(), "+15559876543".to_string())
            .await;
        adapter
            .call_manager
            .update_state("CA789", CallState::Active)
            .await;

        // Send with chat_id pointing to active call — will try TTS (fails because
        // we don't have real Twilio credentials, but the path is exercised)
        let source = ChannelSource {
            channel_type: "voice".to_string(),
            user_id: "+15559876543".to_string(),
            chat_id: Some("CA789".to_string()),
            account_id: None,
            thread_id: None,
            reply_to_message_id: None,
            sender_type: zeus_core::SenderType::Human,
        };

        // This will fail at the HTTP level since we have test credentials,
        // but it exercises the "play TTS on active call" code path
        let result = adapter.send(&source, "Hello from Zeus").await;
        assert!(result.is_err()); // Expected: Twilio API error with test creds
    }

    #[test]
    fn test_env_overrides() {
        // Ensure env override method exists and returns correctly typed config
        let config = VoiceChannelConfig::default();
        let _overridden = config.with_env_overrides();
    }
}
