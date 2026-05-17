//! Voice agent loop — bridges STT → Agent → TTS → audio response
//!
//! Implements the full bidirectional voice pipeline:
//! 1. Receive audio from caller (mu-law 8kHz via Twilio WebSocket)
//! 2. Transcribe speech to text (STT via Groq/OpenAI Whisper)
//! 3. Send transcript to agent for processing
//! 4. Synthesize agent response to audio (TTS via Piper/OpenAI)
//! 5. Convert TTS output to mu-law 8kHz and stream back to caller

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use zeus_core::Result;

use crate::audio::wav_to_mulaw_8k;
use crate::call::{CallManager, TranscriptEntry};

/// Trait for processing voice messages through an agent.
///
/// Implementors bridge the voice pipeline to the agent system.
/// The default implementation is a no-op that echoes back a generic response.
#[async_trait]
pub trait VoiceAgentHandler: Send + Sync {
    /// Process a user's spoken message and return the agent's text response.
    ///
    /// # Arguments
    /// * `call_id` — Unique call identifier (Twilio CallSid)
    /// * `text` — Transcribed user speech
    ///
    /// # Returns
    /// The agent's response text to be synthesized into speech.
    async fn process_message(&self, call_id: &str, text: &str) -> Result<String>;
}

/// TTS synthesis provider interface for the voice loop.
///
/// Abstraction over zeus-tts to keep the voice crate loosely coupled.
#[async_trait]
pub trait VoiceTTSProvider: Send + Sync {
    /// Synthesize text to WAV audio bytes.
    ///
    /// Must return audio in WAV format (PCM 16-bit preferred).
    /// The voice loop will handle conversion to mu-law 8kHz.
    async fn synthesize_wav(&self, text: &str, voice: &str) -> Result<Vec<u8>>;

    /// Synthesize text and stream WAV audio chunks.
    ///
    /// Returns a channel receiver that yields audio byte chunks as they
    /// arrive from the TTS provider. This enables low-latency playback
    /// by sending audio to the caller before the full response is ready.
    async fn synthesize_stream(
        &self,
        text: &str,
        voice: &str,
    ) -> Result<mpsc::Receiver<Result<Vec<u8>>>>;
}

/// Configuration for the voice agent loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceLoopConfig {
    /// TTS voice to use for agent responses
    #[serde(default = "default_voice")]
    pub voice: String,

    /// Whether to use streaming TTS for lower latency
    #[serde(default = "default_streaming")]
    pub streaming_tts: bool,

    /// STT provider preference: "groq" or "openai"
    #[serde(default)]
    pub stt_provider: Option<String>,

    /// Piper TTS server URL (e.g., "http://localhost:8104")
    #[serde(default)]
    pub piper_url: Option<String>,

    /// TTS provider: "piper", "openai", "elevenlabs", "edge"
    #[serde(default = "default_tts_provider")]
    pub tts_provider: String,

    /// Maximum silence duration (ms) before processing speech
    #[serde(default = "default_silence_timeout")]
    pub silence_timeout_ms: u64,
    /// Number of TTS segments to pre-synthesize in the background.
    /// Set to 0 to disable prefetching.
    #[serde(default = "default_prefetch_segments")]
    pub prefetch_segments: usize,
}

fn default_voice() -> String {
    "default".to_string()
}

fn default_streaming() -> bool {
    true
}

fn default_tts_provider() -> String {
    "piper".to_string()
}

fn default_silence_timeout() -> u64 {
    1500
}
pub fn default_prefetch_segments() -> usize {
    3
}

impl Default for VoiceLoopConfig {
    fn default() -> Self {
        Self {
            voice: default_voice(),
            streaming_tts: default_streaming(),
            stt_provider: None,
            piper_url: None,
            tts_provider: default_tts_provider(),
            silence_timeout_ms: default_silence_timeout(),
            prefetch_segments: default_prefetch_segments(),
        }
    }
}

/// A command sent from the voice loop to the WebSocket writer.
#[derive(Debug)]
pub enum VoiceCommand {
    /// Send mu-law audio bytes to the caller
    SendAudio {
        stream_sid: String,
        mulaw_bytes: Vec<u8>,
    },
    /// Clear the audio queue (interrupt current playback)
    ClearAudio { stream_sid: String },
    /// Add a transcript entry
    AddTranscript {
        call_id: String,
        entry: TranscriptEntry,
    },
}

/// The voice agent loop orchestrator.
///
/// Manages the full pipeline from user speech to agent audio response.
pub struct VoiceAgentLoop {
    pub(crate) agent: Arc<dyn VoiceAgentHandler>,
    pub(crate) tts: Arc<dyn VoiceTTSProvider>,
    pub(crate) call_manager: Arc<CallManager>,
    pub(crate) config: VoiceLoopConfig,
}

impl VoiceAgentLoop {
    pub fn new(
        agent: Arc<dyn VoiceAgentHandler>,
        tts: Arc<dyn VoiceTTSProvider>,
        call_manager: Arc<CallManager>,
        config: VoiceLoopConfig,
    ) -> Self {
        Self {
            agent,
            tts,
            call_manager,
            config,
        }
    }

    /// Process a transcribed user message through the full pipeline.
    ///
    /// 1. Sends text to agent
    /// 2. Synthesizes response via TTS
    /// 3. Converts to mu-law 8kHz
    /// 4. Sends audio commands to the WebSocket writer
    ///
    /// Returns the agent's text response for transcript logging.
    pub async fn process_utterance(
        &self,
        call_id: &str,
        stream_sid: &str,
        user_text: &str,
        cmd_tx: &mpsc::Sender<VoiceCommand>,
    ) -> Option<String> {
        if user_text.trim().is_empty() {
            return None;
        }

        info!(
            "Processing utterance for call {}: \"{}\"",
            call_id, user_text
        );

        // Log user transcript
        let _ = cmd_tx
            .send(VoiceCommand::AddTranscript {
                call_id: call_id.to_string(),
                entry: TranscriptEntry::user(user_text),
            })
            .await;

        // 1. Get agent response
        let agent_response = match self.agent.process_message(call_id, user_text).await {
            Ok(response) => response,
            Err(e) => {
                error!("Agent failed for call {}: {}", call_id, e);
                "I'm sorry, I had trouble processing that. Could you repeat?".to_string()
            }
        };

        info!(
            "Agent response for call {}: \"{}\"",
            call_id,
            if agent_response.len() > 100 {
                format!("{}...", zeus_core::truncate_str(&agent_response, 100))
            } else {
                agent_response.clone()
            }
        );

        // Log agent transcript
        let _ = cmd_tx
            .send(VoiceCommand::AddTranscript {
                call_id: call_id.to_string(),
                entry: TranscriptEntry::agent(&agent_response),
            })
            .await;

        // 2. Clear any current playback (barge-in)
        let _ = cmd_tx
            .send(VoiceCommand::ClearAudio {
                stream_sid: stream_sid.to_string(),
            })
            .await;

        // 3. Synthesize and send audio
        if self.config.streaming_tts {
            self.synthesize_streaming(stream_sid, &agent_response, cmd_tx)
                .await;
        } else {
            self.synthesize_buffered(stream_sid, &agent_response, cmd_tx)
                .await;
        }

        Some(agent_response)
    }

    /// Buffered TTS: synthesize entire response, then send
    async fn synthesize_buffered(
        &self,
        stream_sid: &str,
        text: &str,
        cmd_tx: &mpsc::Sender<VoiceCommand>,
    ) {
        match self.tts.synthesize_wav(text, &self.config.voice).await {
            Ok(wav_data) => {
                match wav_to_mulaw_8k(&wav_data) {
                    Ok(mulaw) => {
                        debug!("TTS produced {} mu-law bytes", mulaw.len());
                        // Send in chunks to avoid overwhelming the WebSocket
                        // Twilio accepts ~20ms chunks (160 bytes at 8kHz)
                        for chunk in mulaw.chunks(640) {
                            // 80ms chunks
                            let _ = cmd_tx
                                .send(VoiceCommand::SendAudio {
                                    stream_sid: stream_sid.to_string(),
                                    mulaw_bytes: chunk.to_vec(),
                                })
                                .await;
                        }
                    }
                    Err(e) => {
                        warn!("Failed to convert WAV to mu-law: {}", e);
                    }
                }
            }
            Err(e) => {
                warn!("TTS synthesis failed: {}", e);
            }
        }
    }

    /// Streaming TTS: send audio chunks as they arrive for lower latency
    async fn synthesize_streaming(
        &self,
        stream_sid: &str,
        text: &str,
        cmd_tx: &mpsc::Sender<VoiceCommand>,
    ) {
        match self.tts.synthesize_stream(text, &self.config.voice).await {
            Ok(mut rx) => {
                let mut total_bytes = 0usize;
                let mut wav_accumulator = Vec::new();

                while let Some(chunk_result) = rx.recv().await {
                    match chunk_result {
                        Ok(chunk) => {
                            wav_accumulator.extend_from_slice(&chunk);

                            // Try to convert accumulated WAV data
                            // For streaming, we accumulate until we have enough for a valid WAV
                            if wav_accumulator.len() > 44 + 1600 {
                                // At least header + 100ms
                                match wav_to_mulaw_8k(&wav_accumulator) {
                                    Ok(mulaw) => {
                                        total_bytes += mulaw.len();
                                        for audio_chunk in mulaw.chunks(640) {
                                            let _ = cmd_tx
                                                .send(VoiceCommand::SendAudio {
                                                    stream_sid: stream_sid.to_string(),
                                                    mulaw_bytes: audio_chunk.to_vec(),
                                                })
                                                .await;
                                        }
                                        wav_accumulator.clear();
                                    }
                                    Err(_) => {
                                        // Not enough data yet or invalid chunk, keep accumulating
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!("TTS stream chunk error: {}", e);
                            break;
                        }
                    }
                }

                // Process any remaining accumulated data
                if !wav_accumulator.is_empty()
                    && let Ok(mulaw) = wav_to_mulaw_8k(&wav_accumulator)
                {
                    total_bytes += mulaw.len();
                    for audio_chunk in mulaw.chunks(640) {
                        let _ = cmd_tx
                            .send(VoiceCommand::SendAudio {
                                stream_sid: stream_sid.to_string(),
                                mulaw_bytes: audio_chunk.to_vec(),
                            })
                            .await;
                    }
                }

                debug!(
                    "Streaming TTS complete: {} total mu-law bytes sent",
                    total_bytes
                );
            }
            Err(e) => {
                warn!("TTS streaming init failed, falling back to buffered: {}", e);
                self.synthesize_buffered(stream_sid, text, cmd_tx).await;
            }
        }
    }
}

/// Format an outgoing media JSON message for Twilio WebSocket.
pub fn format_outgoing_media(stream_sid: &str, mulaw_bytes: &[u8]) -> String {
    let payload = BASE64.encode(mulaw_bytes);
    serde_json::json!({
        "event": "media",
        "streamSid": stream_sid,
        "media": {
            "payload": payload
        }
    })
    .to_string()
}

/// Format a clear message for Twilio WebSocket.
pub fn format_clear_message(stream_sid: &str) -> String {
    serde_json::json!({
        "event": "clear",
        "streamSid": stream_sid
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeus_core::Error;

    // Mock agent handler for testing
    struct MockAgent {
        response: String,
    }

    #[async_trait]
    impl VoiceAgentHandler for MockAgent {
        async fn process_message(&self, _call_id: &str, _text: &str) -> Result<String> {
            Ok(self.response.clone())
        }
    }

    struct FailingAgent;

    #[async_trait]
    impl VoiceAgentHandler for FailingAgent {
        async fn process_message(&self, _call_id: &str, _text: &str) -> Result<String> {
            Err(Error::Internal("agent error".to_string()))
        }
    }

    // Mock TTS provider
    struct MockTTS;

    #[async_trait]
    impl VoiceTTSProvider for MockTTS {
        async fn synthesize_wav(&self, _text: &str, _voice: &str) -> Result<Vec<u8>> {
            // Return a minimal valid WAV with silence
            let samples = vec![0i16; 800]; // 100ms at 8kHz
            let mut wav = Vec::new();
            let data_size = (samples.len() * 2) as u32;
            let file_size = 36 + data_size;

            wav.extend_from_slice(b"RIFF");
            wav.extend_from_slice(&file_size.to_le_bytes());
            wav.extend_from_slice(b"WAVE");
            wav.extend_from_slice(b"fmt ");
            wav.extend_from_slice(&16u32.to_le_bytes());
            wav.extend_from_slice(&1u16.to_le_bytes());
            wav.extend_from_slice(&1u16.to_le_bytes());
            wav.extend_from_slice(&8000u32.to_le_bytes());
            wav.extend_from_slice(&(8000u32 * 2).to_le_bytes());
            wav.extend_from_slice(&2u16.to_le_bytes());
            wav.extend_from_slice(&16u16.to_le_bytes());
            wav.extend_from_slice(b"data");
            wav.extend_from_slice(&data_size.to_le_bytes());
            for &s in &samples {
                wav.extend_from_slice(&s.to_le_bytes());
            }

            Ok(wav)
        }

        async fn synthesize_stream(
            &self,
            text: &str,
            voice: &str,
        ) -> Result<mpsc::Receiver<Result<Vec<u8>>>> {
            let (tx, rx) = mpsc::channel(1);
            let wav = self.synthesize_wav(text, voice).await?;
            tokio::spawn(async move {
                let _ = tx.send(Ok(wav)).await;
            });
            Ok(rx)
        }
    }

    #[test]
    fn test_voice_loop_config_defaults() {
        let config = VoiceLoopConfig::default();
        assert_eq!(config.voice, "default");
        assert!(config.streaming_tts);
        assert!(config.stt_provider.is_none());
        assert!(config.piper_url.is_none());
        assert_eq!(config.tts_provider, "piper");
        assert_eq!(config.silence_timeout_ms, 1500);
    }

    #[test]
    fn test_voice_loop_config_serialization() {
        let config = VoiceLoopConfig {
            voice: "en-us-amy".to_string(),
            streaming_tts: false,
            stt_provider: Some("groq".to_string()),
            piper_url: Some("http://localhost:8104".to_string()),
            tts_provider: "piper".to_string(),
            silence_timeout_ms: 2000,
            prefetch_segments: 3,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: VoiceLoopConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.voice, "en-us-amy");
        assert!(!parsed.streaming_tts);
        assert_eq!(
            parsed.piper_url.as_deref(),
            Some("http://localhost:8104")
        );
    }

    #[test]
    fn test_format_outgoing_media() {
        let json_str = format_outgoing_media("MZ123", &[0x80, 0xFF]);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["event"], "media");
        assert_eq!(parsed["streamSid"], "MZ123");
        assert!(parsed["media"]["payload"].is_string());
    }

    #[test]
    fn test_format_clear_message() {
        let json_str = format_clear_message("MZ456");
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["event"], "clear");
        assert_eq!(parsed["streamSid"], "MZ456");
    }

    #[tokio::test]
    async fn test_voice_loop_process_utterance() {
        let agent = Arc::new(MockAgent {
            response: "Hello there!".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let call_manager = Arc::new(CallManager::new());
        let config = VoiceLoopConfig {
            streaming_tts: false,
            ..Default::default()
        };

        let voice_loop = VoiceAgentLoop::new(agent, tts, call_manager, config);
        let (cmd_tx, mut cmd_rx) = mpsc::channel(32);

        let result = voice_loop
            .process_utterance("CA123", "MZ456", "How are you?", &cmd_tx)
            .await;

        assert_eq!(result, Some("Hello there!".to_string()));

        // Should receive: user transcript, agent transcript, clear, then audio
        let mut commands = Vec::new();
        while let Ok(cmd) = cmd_rx.try_recv() {
            commands.push(cmd);
        }

        assert!(commands.len() >= 3); // At least: user transcript, agent transcript, clear
    }

    #[tokio::test]
    async fn test_voice_loop_empty_utterance() {
        let agent = Arc::new(MockAgent {
            response: "test".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let call_manager = Arc::new(CallManager::new());
        let config = VoiceLoopConfig::default();

        let voice_loop = VoiceAgentLoop::new(agent, tts, call_manager, config);
        let (cmd_tx, _cmd_rx) = mpsc::channel(32);

        let result = voice_loop
            .process_utterance("CA123", "MZ456", "  ", &cmd_tx)
            .await;

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_voice_loop_agent_failure_graceful() {
        let agent = Arc::new(FailingAgent);
        let tts = Arc::new(MockTTS);
        let call_manager = Arc::new(CallManager::new());
        let config = VoiceLoopConfig {
            streaming_tts: false,
            ..Default::default()
        };

        let voice_loop = VoiceAgentLoop::new(agent, tts, call_manager, config);
        let (cmd_tx, _cmd_rx) = mpsc::channel(32);

        let result = voice_loop
            .process_utterance("CA123", "MZ456", "hello", &cmd_tx)
            .await;

        // Should still return a response (the fallback message)
        assert!(result.is_some());
        assert!(result.unwrap().contains("sorry"));
    }
}
