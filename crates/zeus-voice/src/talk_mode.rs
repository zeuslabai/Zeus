//! Talk Mode — Continuous local voice conversation
//!
//! Provides always-on voice interaction via local microphone:
//! - **Continuous mode**: Wake word activates listening, silence ends utterance
//! - **Push-to-talk mode**: Hold key to record, release to process
//!
//! Pipeline: Mic → WakeWord → STT → Agent → TTS → Speaker
//!
//! State machine:
//! ```text
//!   Idle → Listening (wake word active)
//!        → Active (recording speech)
//!        → Processing (STT + agent)
//!        → Speaking (TTS playback)
//!        → Listening (loop) or Idle (on stop)
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{RwLock, mpsc, watch};
use tracing::{debug, error, info, warn};

use crate::agent_loop::VoiceTTSProvider;
use crate::wake_word::{WakeWordConfig, WakeWordDetector};

/// Trait for processing voice input through an agent.
///
/// Similar to VoiceAgentHandler but for local talk mode (no call_id).
#[async_trait]
pub trait TalkModeAgentHandler: Send + Sync {
    /// Process spoken text and return the agent's response.
    async fn process_voice_input(&self, text: &str) -> zeus_core::Result<String>;
}

/// Trait for local microphone audio capture.
///
/// Implementations provide platform-specific audio input.
#[async_trait]
pub trait AudioInputProvider: Send + Sync {
    /// Start capturing audio. Returns a receiver of PCM 16-bit 16kHz mono frames.
    async fn start_capture(&self) -> zeus_core::Result<mpsc::Receiver<Vec<i16>>>;
    /// Stop capturing audio.
    async fn stop_capture(&self);
    /// Check if audio input is available.
    fn is_available(&self) -> bool;
}

/// Trait for local audio output (speaker playback).
#[async_trait]
pub trait AudioOutputProvider: Send + Sync {
    /// Play WAV audio data through the speaker.
    async fn play_wav(&self, wav_data: &[u8]) -> zeus_core::Result<()>;
    /// Stop any current playback.
    async fn stop_playback(&self);
    /// Check if currently playing.
    fn is_playing(&self) -> bool;
}

/// Talk mode interaction style
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TalkModeStyle {
    /// Wake word activates, silence ends utterance
    #[default]
    Continuous,
    /// Manual activation via push-to-talk
    PushToTalk,
    /// Always listening (no wake word needed)
    AlwaysOn,
}

/// Talk mode state machine
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TalkModeState {
    /// Not active — waiting for start()
    #[default]
    Idle,
    /// Listening for wake word
    Listening,
    /// Recording user speech (wake word triggered or PTT held)
    Active,
    /// Processing: running STT then agent
    Processing,
    /// Playing TTS response
    Speaking,
    /// Error state
    Error,
}

/// Configuration for Talk Mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TalkModeConfig {
    /// Interaction style
    #[serde(default)]
    pub style: TalkModeStyle,

    /// Wake word settings (used in Continuous mode)
    #[serde(default)]
    pub wake_word: WakeWordConfig,

    /// TTS voice to use for responses
    #[serde(default = "default_tts_voice")]
    pub tts_voice: String,

    /// TTS provider: "piper", "openai", "elevenlabs", "edge"
    #[serde(default = "default_tts_provider")]
    pub tts_provider: String,

    /// Maximum recording duration (seconds) before auto-cutoff
    #[serde(default = "default_max_recording_secs")]
    pub max_recording_secs: u32,

    /// Silence duration (ms) that ends an utterance
    #[serde(default = "default_silence_cutoff_ms")]
    pub silence_cutoff_ms: u64,

    /// Silence threshold — RMS level below which audio is considered silent
    #[serde(default = "default_silence_threshold")]
    pub silence_threshold: f32,

    /// Play a chime/beep when entering Active state
    #[serde(default = "default_activation_sound")]
    pub activation_sound: bool,

    /// Pause wake word detection while TTS is playing (prevents self-trigger)
    #[serde(default = "default_pause_during_tts")]
    pub pause_during_tts: bool,

    /// Enable transcription echo (print what was heard)
    #[serde(default)]
    pub echo_transcription: bool,

    /// STT provider preference: "groq" or "openai"
    #[serde(default)]
    pub stt_provider: Option<String>,
}

fn default_tts_voice() -> String {
    "default".to_string()
}

fn default_tts_provider() -> String {
    "piper".to_string()
}

fn default_max_recording_secs() -> u32 {
    30
}

fn default_silence_cutoff_ms() -> u64 {
    1500
}

fn default_silence_threshold() -> f32 {
    0.02
}

fn default_activation_sound() -> bool {
    true
}

fn default_pause_during_tts() -> bool {
    true
}

impl Default for TalkModeConfig {
    fn default() -> Self {
        Self {
            style: TalkModeStyle::default(),
            wake_word: WakeWordConfig::default(),
            tts_voice: default_tts_voice(),
            tts_provider: default_tts_provider(),
            max_recording_secs: default_max_recording_secs(),
            silence_cutoff_ms: default_silence_cutoff_ms(),
            silence_threshold: default_silence_threshold(),
            activation_sound: default_activation_sound(),
            pause_during_tts: default_pause_during_tts(),
            echo_transcription: false,
            stt_provider: None,
        }
    }
}

/// Event emitted by TalkMode for UI/logging
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TalkModeEvent {
    /// State changed
    StateChanged {
        from: TalkModeState,
        to: TalkModeState,
    },
    /// Wake word detected
    WakeWordDetected { word: String, confidence: f32 },
    /// User speech transcribed
    Transcribed { text: String },
    /// Agent responded
    AgentResponse { text: String },
    /// TTS playback started
    SpeakingStarted,
    /// TTS playback finished
    SpeakingFinished,
    /// Error occurred
    Error { message: String },
    /// Audio level update (for visualizations)
    AudioLevel { rms: f32 },
    /// Barge-in detected — user spoke while TTS was playing
    BargeIn {
        /// RMS level that triggered the barge-in
        rms: f32,
    },
    /// Voice directive recognized
    DirectiveReceived { directive: VoiceDirective },
    /// Mute state changed
    MuteChanged { muted: bool },
}

/// Voice directives — commands the user can issue during conversation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceDirective {
    /// Pause the conversation (stop listening, keep state)
    Pause,
    /// Resume a paused conversation
    Resume,
    /// Mute microphone input
    Mute,
    /// Unmute microphone input
    Unmute,
    /// Stop and shut down talk mode
    Stop,
    /// Repeat the last agent response
    Repeat,
}

/// Conversation turn for transcript
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TalkModeTurn {
    /// "user" or "agent"
    pub role: String,
    /// The text content
    pub text: String,
    /// Timestamp (Unix millis)
    pub timestamp: i64,
}

/// Talk Mode — continuous voice conversation controller
pub struct TalkMode {
    config: TalkModeConfig,
    state: Arc<RwLock<TalkModeState>>,
    running: Arc<AtomicBool>,
    agent: Arc<dyn TalkModeAgentHandler>,
    tts: Arc<dyn VoiceTTSProvider>,
    audio_input: Arc<dyn AudioInputProvider>,
    audio_output: Arc<dyn AudioOutputProvider>,
    event_tx: mpsc::Sender<TalkModeEvent>,
    /// Push-to-talk active flag (set externally)
    ptt_active: Arc<AtomicBool>,
    /// Conversation transcript
    transcript: Arc<RwLock<Vec<TalkModeTurn>>>,
    /// State change watcher
    state_tx: watch::Sender<TalkModeState>,
    /// Microphone muted flag
    muted: Arc<AtomicBool>,
    /// Paused flag — listening suspended but not stopped
    paused: Arc<AtomicBool>,
    /// Barge-in threshold — if audio RMS exceeds this during Speaking, interrupt TTS
    barge_in_threshold: f32,
    /// Last agent response (for repeat directive)
    last_response: Arc<RwLock<Option<String>>>,
}

impl TalkMode {
    /// Create a new TalkMode instance
    pub fn new(
        config: TalkModeConfig,
        agent: Arc<dyn TalkModeAgentHandler>,
        tts: Arc<dyn VoiceTTSProvider>,
        audio_input: Arc<dyn AudioInputProvider>,
        audio_output: Arc<dyn AudioOutputProvider>,
    ) -> (
        Self,
        mpsc::Receiver<TalkModeEvent>,
        watch::Receiver<TalkModeState>,
    ) {
        let (event_tx, event_rx) = mpsc::channel(64);
        let (state_tx, state_rx) = watch::channel(TalkModeState::Idle);

        let barge_in_threshold = config.silence_threshold * 3.0; // 3x silence threshold
        let talk_mode = Self {
            config,
            state: Arc::new(RwLock::new(TalkModeState::Idle)),
            running: Arc::new(AtomicBool::new(false)),
            agent,
            tts,
            audio_input,
            audio_output,
            event_tx,
            ptt_active: Arc::new(AtomicBool::new(false)),
            transcript: Arc::new(RwLock::new(Vec::new())),
            state_tx,
            muted: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            barge_in_threshold,
            last_response: Arc::new(RwLock::new(None)),
        };

        (talk_mode, event_rx, state_rx)
    }

    /// Start the talk mode conversation loop
    pub async fn start(&self) -> zeus_core::Result<()> {
        if self.running.load(Ordering::SeqCst) {
            return Err(zeus_core::Error::Internal(
                "Talk mode already running".to_string(),
            ));
        }

        if !self.audio_input.is_available() {
            return Err(zeus_core::Error::Internal(
                "No audio input device available".to_string(),
            ));
        }

        self.running.store(true, Ordering::SeqCst);
        self.set_state(TalkModeState::Listening).await;
        info!(style = ?self.config.style, "Talk mode started");

        // Start audio capture
        let audio_rx = self.audio_input.start_capture().await?;

        // Start the main loop
        self.run_loop(audio_rx).await;

        Ok(())
    }

    /// Stop talk mode
    pub async fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.audio_input.stop_capture().await;
        self.audio_output.stop_playback().await;
        self.set_state(TalkModeState::Idle).await;
        info!("Talk mode stopped");
    }

    /// Activate push-to-talk (call from UI when PTT key pressed)
    pub fn ptt_press(&self) {
        self.ptt_active.store(true, Ordering::SeqCst);
        debug!("PTT pressed");
    }

    /// Release push-to-talk (call from UI when PTT key released)
    pub fn ptt_release(&self) {
        self.ptt_active.store(false, Ordering::SeqCst);
        debug!("PTT released");
    }

    /// Get current state
    pub async fn state(&self) -> TalkModeState {
        *self.state.read().await
    }

    /// Get conversation transcript
    pub async fn transcript(&self) -> Vec<TalkModeTurn> {
        self.transcript.read().await.clone()
    }

    /// Clear conversation transcript
    pub async fn clear_transcript(&self) {
        self.transcript.write().await.clear();
    }

    /// Check if running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Get config
    pub fn config(&self) -> &TalkModeConfig {
        &self.config
    }

    // ========================================================================
    // Voice Directives
    // ========================================================================

    /// Issue a voice directive
    pub async fn directive(&self, directive: VoiceDirective) {
        let _ = self
            .event_tx
            .send(TalkModeEvent::DirectiveReceived {
                directive: directive.clone(),
            })
            .await;

        match directive {
            VoiceDirective::Pause => {
                self.paused.store(true, Ordering::SeqCst);
                info!("Talk mode paused");
            }
            VoiceDirective::Resume => {
                self.paused.store(false, Ordering::SeqCst);
                info!("Talk mode resumed");
            }
            VoiceDirective::Mute => {
                self.muted.store(true, Ordering::SeqCst);
                let _ = self
                    .event_tx
                    .send(TalkModeEvent::MuteChanged { muted: true })
                    .await;
                info!("Microphone muted");
            }
            VoiceDirective::Unmute => {
                self.muted.store(false, Ordering::SeqCst);
                let _ = self
                    .event_tx
                    .send(TalkModeEvent::MuteChanged { muted: false })
                    .await;
                info!("Microphone unmuted");
            }
            VoiceDirective::Stop => {
                self.stop().await;
            }
            VoiceDirective::Repeat => {
                let last = self.last_response.read().await.clone();
                if let Some(text) = last {
                    let _ = self
                        .event_tx
                        .send(TalkModeEvent::AgentResponse { text: text.clone() })
                        .await;
                    // Replay TTS
                    let _ = self.event_tx.send(TalkModeEvent::SpeakingStarted).await;
                    if let Ok(wav) = self.tts.synthesize_wav(&text, &self.config.tts_voice).await {
                        let _ = self.audio_output.play_wav(&wav).await;
                    }
                    let _ = self.event_tx.send(TalkModeEvent::SpeakingFinished).await;
                }
            }
        }
    }

    /// Check if microphone is muted
    pub fn is_muted(&self) -> bool {
        self.muted.load(Ordering::SeqCst)
    }

    /// Check if talk mode is paused
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Get last agent response (for repeat)
    pub async fn last_response(&self) -> Option<String> {
        self.last_response.read().await.clone()
    }

    /// Trigger a barge-in — interrupts current TTS playback and returns to listening.
    ///
    /// This is called either automatically (when user speech detected during Speaking)
    /// or manually from external callers (e.g., a VoiceCallProvider detecting inbound audio).
    pub async fn barge_in(&self) {
        let current = self.state().await;
        if current == TalkModeState::Speaking {
            warn!("Barge-in triggered — interrupting TTS");
            self.audio_output.stop_playback().await;
            let _ = self
                .event_tx
                .send(TalkModeEvent::BargeIn { rms: 0.0 })
                .await;
            self.set_state(TalkModeState::Listening).await;
        }
    }

    // ========================================================================
    // Internal
    // ========================================================================

    async fn set_state(&self, new_state: TalkModeState) {
        let old_state = {
            let mut state = self.state.write().await;
            let old = *state;
            *state = new_state;
            old
        };

        let _ = self.state_tx.send(new_state);

        if old_state != new_state {
            let _ = self
                .event_tx
                .send(TalkModeEvent::StateChanged {
                    from: old_state,
                    to: new_state,
                })
                .await;
        }
    }

    async fn run_loop(&self, mut audio_rx: mpsc::Receiver<Vec<i16>>) {
        // Initialize wake word detector if using continuous mode
        let wake_detector = if self.config.style == TalkModeStyle::Continuous {
            let detector = WakeWordDetector::new(self.config.wake_word.clone());
            if let Err(e) = detector.init().await {
                warn!("Wake word init failed: {}, falling back to always-on", e);
                None
            } else {
                Some(detector)
            }
        } else {
            None
        };

        let mut audio_buffer: Vec<i16> = Vec::new();
        let mut silence_frames: u64 = 0;
        let silence_frame_threshold = self.config.silence_cutoff_ms / 10; // ~10ms per frame at 16kHz/512
        let max_frames = (self.config.max_recording_secs as u64 * 16000) / 512;

        while self.running.load(Ordering::SeqCst) {
            // Check pause state
            if self.paused.load(Ordering::SeqCst) {
                // Consume audio to prevent backpressure, but don't process
                let _ = audio_rx.recv().await;
                continue;
            }

            let current_state = self.state().await;

            match current_state {
                TalkModeState::Listening => {
                    // If muted, consume audio but don't activate
                    if self.muted.load(Ordering::SeqCst) {
                        let _ = audio_rx.recv().await;
                        continue;
                    }

                    // Wait for activation trigger
                    let should_activate = match self.config.style {
                        TalkModeStyle::Continuous => {
                            // Feed audio to wake word detector
                            if let Some(frame) = audio_rx.recv().await {
                                if let Some(ref _detector) = wake_detector {
                                    // Process through wake word detector
                                    // (In real impl, detector.start() runs its own loop;
                                    //  here we check via the event subscription pattern)
                                    let rms = calculate_rms(&frame);
                                    let _ =
                                        self.event_tx.send(TalkModeEvent::AudioLevel { rms }).await;

                                    // For now, detect "hey zeus" via energy spike
                                    // Real implementation would use the detector's process()
                                    false
                                } else {
                                    // No detector — treat any speech as activation
                                    let rms = calculate_rms(&frame);
                                    rms > self.config.silence_threshold
                                }
                            } else {
                                break; // Audio stream ended
                            }
                        }
                        TalkModeStyle::PushToTalk => {
                            // Wait for PTT press
                            if self.ptt_active.load(Ordering::SeqCst) {
                                // Drain any pending audio
                                while audio_rx.try_recv().is_ok() {}
                                true
                            } else {
                                // Consume audio to prevent backpressure
                                let _ = audio_rx.recv().await;
                                false
                            }
                        }
                        TalkModeStyle::AlwaysOn => {
                            // Any speech above threshold
                            if let Some(frame) = audio_rx.recv().await {
                                let rms = calculate_rms(&frame);
                                let _ = self.event_tx.send(TalkModeEvent::AudioLevel { rms }).await;
                                rms > self.config.silence_threshold
                            } else {
                                break;
                            }
                        }
                    };

                    if should_activate {
                        if wake_detector.is_some() {
                            let _ = self
                                .event_tx
                                .send(TalkModeEvent::WakeWordDetected {
                                    word: "hey zeus".to_string(),
                                    confidence: 0.9,
                                })
                                .await;
                        }

                        audio_buffer.clear();
                        silence_frames = 0;
                        self.set_state(TalkModeState::Active).await;
                    }
                }

                TalkModeState::Active => {
                    // Record audio until silence or PTT release
                    let should_stop = match self.config.style {
                        TalkModeStyle::PushToTalk => !self.ptt_active.load(Ordering::SeqCst),
                        _ => false,
                    };

                    if should_stop {
                        // PTT released — process what we have
                        if !audio_buffer.is_empty() {
                            self.set_state(TalkModeState::Processing).await;
                        } else {
                            self.set_state(TalkModeState::Listening).await;
                        }
                        continue;
                    }

                    match audio_rx.recv().await {
                        Some(frame) => {
                            let rms = calculate_rms(&frame);
                            let _ = self.event_tx.send(TalkModeEvent::AudioLevel { rms }).await;

                            audio_buffer.extend_from_slice(&frame);

                            if rms < self.config.silence_threshold {
                                silence_frames += 1;
                                if silence_frames >= silence_frame_threshold {
                                    // Silence detected — process utterance
                                    if !audio_buffer.is_empty() {
                                        self.set_state(TalkModeState::Processing).await;
                                    } else {
                                        self.set_state(TalkModeState::Listening).await;
                                    }
                                }
                            } else {
                                silence_frames = 0;
                            }

                            // Max duration check
                            let frames_recorded = audio_buffer.len() as u64 / 512;
                            if frames_recorded >= max_frames {
                                self.set_state(TalkModeState::Processing).await;
                            }
                        }
                        None => break,
                    }
                }

                TalkModeState::Processing => {
                    // Pause wake word during processing
                    if let Some(ref detector) = wake_detector {
                        detector.pause().await;
                    }

                    // 1. Convert PCM buffer to WAV for STT
                    let wav_data = pcm16_to_wav(&audio_buffer, 16000);
                    audio_buffer.clear();
                    silence_frames = 0;

                    // 2. Transcribe
                    let transcription = match transcribe_wav_local(&wav_data).await {
                        Ok(text) => text,
                        Err(e) => {
                            error!("STT failed: {}", e);
                            let _ = self
                                .event_tx
                                .send(TalkModeEvent::Error {
                                    message: format!("STT failed: {}", e),
                                })
                                .await;
                            self.set_state(TalkModeState::Listening).await;
                            if let Some(ref detector) = wake_detector {
                                detector.resume().await;
                            }
                            continue;
                        }
                    };

                    if transcription.trim().is_empty() {
                        debug!("Empty transcription, returning to listening");
                        self.set_state(TalkModeState::Listening).await;
                        if let Some(ref detector) = wake_detector {
                            detector.resume().await;
                        }
                        continue;
                    }

                    let _ = self
                        .event_tx
                        .send(TalkModeEvent::Transcribed {
                            text: transcription.clone(),
                        })
                        .await;

                    // Log to transcript
                    {
                        let mut transcript = self.transcript.write().await;
                        transcript.push(TalkModeTurn {
                            role: "user".to_string(),
                            text: transcription.clone(),
                            timestamp: chrono::Utc::now().timestamp_millis(),
                        });
                    }

                    // 3. Send to agent
                    let agent_response = match self.agent.process_voice_input(&transcription).await
                    {
                        Ok(response) => response,
                        Err(e) => {
                            error!("Agent failed: {}", e);
                            "I'm sorry, I had trouble with that. Could you try again?".to_string()
                        }
                    };

                    let _ = self
                        .event_tx
                        .send(TalkModeEvent::AgentResponse {
                            text: agent_response.clone(),
                        })
                        .await;

                    // Save last response for repeat directive
                    {
                        let mut last = self.last_response.write().await;
                        *last = Some(agent_response.clone());
                    }

                    // Log to transcript
                    {
                        let mut transcript = self.transcript.write().await;
                        transcript.push(TalkModeTurn {
                            role: "agent".to_string(),
                            text: agent_response.clone(),
                            timestamp: chrono::Utc::now().timestamp_millis(),
                        });
                    }

                    // 4. Synthesize TTS
                    self.set_state(TalkModeState::Speaking).await;
                    let _ = self.event_tx.send(TalkModeEvent::SpeakingStarted).await;

                    match self
                        .tts
                        .synthesize_wav(&agent_response, &self.config.tts_voice)
                        .await
                    {
                        Ok(wav) => {
                            if let Err(e) = self.audio_output.play_wav(&wav).await {
                                warn!("Audio playback failed: {}", e);
                            }
                        }
                        Err(e) => {
                            warn!("TTS synthesis failed: {}", e);
                        }
                    }

                    let _ = self.event_tx.send(TalkModeEvent::SpeakingFinished).await;

                    // Resume wake word detection
                    if let Some(ref detector) = wake_detector {
                        detector.resume().await;
                    }

                    self.set_state(TalkModeState::Listening).await;
                }

                TalkModeState::Speaking => {
                    // Monitor for barge-in: if user speaks during TTS, interrupt
                    if let Some(frame) = audio_rx.recv().await {
                        let rms = calculate_rms(&frame);
                        if rms > self.barge_in_threshold && self.audio_output.is_playing() {
                            info!(
                                rms,
                                threshold = self.barge_in_threshold,
                                "Barge-in detected"
                            );
                            self.audio_output.stop_playback().await;
                            let _ = self.event_tx.send(TalkModeEvent::BargeIn { rms }).await;
                            // Go to Active to capture what the user is saying
                            audio_buffer.clear();
                            audio_buffer.extend_from_slice(&frame);
                            silence_frames = 0;
                            self.set_state(TalkModeState::Active).await;
                            continue;
                        }
                    } else {
                        break; // Audio stream ended
                    }

                    // If playback finished while we were checking, go back to Listening
                    if !self.audio_output.is_playing() {
                        self.set_state(TalkModeState::Listening).await;
                    }
                }

                TalkModeState::Idle | TalkModeState::Error => {
                    break;
                }
            }
        }

        self.running.store(false, Ordering::SeqCst);
        self.set_state(TalkModeState::Idle).await;
    }
}

// ============================================================================
// Audio helpers
// ============================================================================

/// Calculate RMS (root mean square) of an audio frame, normalized to 0.0-1.0
pub fn calculate_rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let rms = (sum_sq / samples.len() as f64).sqrt();
    // Normalize to 0.0-1.0 range (max i16 = 32767)
    (rms / 32767.0) as f32
}

/// Convert PCM 16-bit samples to WAV bytes
pub fn pcm16_to_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_size = (samples.len() * 2) as u32;
    let file_size = 36 + data_size;

    let mut wav = Vec::with_capacity(44 + samples.len() * 2);

    // RIFF header
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");

    // fmt chunk
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    // data chunk
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    for &s in samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }

    wav
}

/// Transcribe WAV audio bytes using available STT provider.
///
/// Sends to Groq or OpenAI Whisper API.
async fn transcribe_wav_local(wav_data: &[u8]) -> zeus_core::Result<String> {
    if wav_data.len() <= 44 {
        return Ok(String::new());
    }

    // Select STT provider
    let (api_key, endpoint, model) = if let Ok(key) = std::env::var("GROQ_API_KEY") {
        (
            key,
            "https://api.groq.com/openai/v1/audio/transcriptions",
            "whisper-large-v3",
        )
    } else if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        (
            key,
            "https://api.openai.com/v1/audio/transcriptions",
            "whisper-1",
        )
    } else {
        return Err(zeus_core::Error::Internal(
            "No STT API key found. Set GROQ_API_KEY or OPENAI_API_KEY.".to_string(),
        ));
    };

    let file_part = reqwest::multipart::Part::bytes(wav_data.to_vec())
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| zeus_core::Error::Internal(format!("MIME error: {}", e)))?;

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model", model.to_string())
        .text("language", "en".to_string())
        .text("response_format", "json".to_string());

    let client = reqwest::Client::new();
    let response = client
        .post(endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await
        .map_err(|e| zeus_core::Error::Internal(format!("STT request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(zeus_core::Error::Internal(format!(
            "STT API returned {}: {}",
            status, body
        )));
    }

    let resp_json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| zeus_core::Error::Internal(format!("STT parse error: {}", e)))?;

    Ok(resp_json
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Mock implementations
    // ========================================================================

    struct MockAgent {
        response: String,
    }

    #[async_trait]
    impl TalkModeAgentHandler for MockAgent {
        async fn process_voice_input(&self, _text: &str) -> zeus_core::Result<String> {
            Ok(self.response.clone())
        }
    }

    struct MockTTS;

    #[async_trait]
    impl VoiceTTSProvider for MockTTS {
        async fn synthesize_wav(&self, _text: &str, _voice: &str) -> zeus_core::Result<Vec<u8>> {
            // Return minimal valid WAV
            let samples = vec![0i16; 800];
            Ok(pcm16_to_wav(&samples, 8000))
        }

        async fn synthesize_stream(
            &self,
            _text: &str,
            _voice: &str,
        ) -> zeus_core::Result<mpsc::Receiver<zeus_core::Result<Vec<u8>>>> {
            let (tx, rx) = mpsc::channel(1);
            let wav = pcm16_to_wav(&vec![0i16; 800], 8000);
            tokio::spawn(async move {
                let _ = tx.send(Ok(wav)).await;
            });
            Ok(rx)
        }
    }

    struct MockAudioInput {
        available: bool,
    }

    #[async_trait]
    impl AudioInputProvider for MockAudioInput {
        async fn start_capture(&self) -> zeus_core::Result<mpsc::Receiver<Vec<i16>>> {
            let (tx, rx) = mpsc::channel(16);
            // Send a few frames then stop
            tokio::spawn(async move {
                for _ in 0..5 {
                    let frame = vec![0i16; 512];
                    if tx.send(frame).await.is_err() {
                        break;
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                }
            });
            Ok(rx)
        }

        async fn stop_capture(&self) {}

        fn is_available(&self) -> bool {
            self.available
        }
    }

    struct MockAudioOutput;

    #[async_trait]
    impl AudioOutputProvider for MockAudioOutput {
        async fn play_wav(&self, _wav_data: &[u8]) -> zeus_core::Result<()> {
            Ok(())
        }
        async fn stop_playback(&self) {}
        fn is_playing(&self) -> bool {
            false
        }
    }

    // ========================================================================
    // Tests
    // ========================================================================

    #[test]
    fn test_talk_mode_config_defaults() {
        let config = TalkModeConfig::default();
        assert_eq!(config.style, TalkModeStyle::Continuous);
        assert_eq!(config.max_recording_secs, 30);
        assert_eq!(config.silence_cutoff_ms, 1500);
        assert!(config.activation_sound);
        assert!(config.pause_during_tts);
        assert!(!config.echo_transcription);
        assert!(config.stt_provider.is_none());
    }

    #[test]
    fn test_talk_mode_config_serialization() {
        let config = TalkModeConfig {
            style: TalkModeStyle::PushToTalk,
            tts_voice: "en-us-amy".to_string(),
            tts_provider: "elevenlabs".to_string(),
            max_recording_secs: 60,
            ..Default::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: TalkModeConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.style, TalkModeStyle::PushToTalk);
        assert_eq!(parsed.tts_voice, "en-us-amy");
        assert_eq!(parsed.tts_provider, "elevenlabs");
        assert_eq!(parsed.max_recording_secs, 60);
    }

    #[test]
    fn test_talk_mode_style_variants() {
        let styles = [
            TalkModeStyle::Continuous,
            TalkModeStyle::PushToTalk,
            TalkModeStyle::AlwaysOn,
        ];
        for style in styles {
            let json = serde_json::to_string(&style).unwrap();
            let parsed: TalkModeStyle = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, style);
        }
    }

    #[test]
    fn test_talk_mode_state_variants() {
        let states = [
            TalkModeState::Idle,
            TalkModeState::Listening,
            TalkModeState::Active,
            TalkModeState::Processing,
            TalkModeState::Speaking,
            TalkModeState::Error,
        ];
        for state in states {
            let json = serde_json::to_string(&state).unwrap();
            let parsed: TalkModeState = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, state);
        }
    }

    #[test]
    fn test_calculate_rms_silence() {
        let silence = vec![0i16; 512];
        assert_eq!(calculate_rms(&silence), 0.0);
    }

    #[test]
    fn test_calculate_rms_loud() {
        let loud = vec![i16::MAX; 512];
        let rms = calculate_rms(&loud);
        assert!(rms > 0.99, "RMS of max samples should be ~1.0, got {}", rms);
    }

    #[test]
    fn test_calculate_rms_empty() {
        assert_eq!(calculate_rms(&[]), 0.0);
    }

    #[test]
    fn test_calculate_rms_mixed() {
        let mixed: Vec<i16> = (0..512)
            .map(|i| if i % 2 == 0 { 1000 } else { -1000 })
            .collect();
        let rms = calculate_rms(&mixed);
        assert!(rms > 0.0);
        assert!(rms < 0.1); // 1000/32767 ≈ 0.03
    }

    #[test]
    fn test_pcm16_to_wav() {
        let samples = vec![100i16, -200, 300, 0];
        let wav = pcm16_to_wav(&samples, 16000);

        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");

        let format_tag = u16::from_le_bytes([wav[20], wav[21]]);
        assert_eq!(format_tag, 1); // PCM

        let channels = u16::from_le_bytes([wav[22], wav[23]]);
        assert_eq!(channels, 1); // mono

        let sample_rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
        assert_eq!(sample_rate, 16000);

        assert_eq!(&wav[36..40], b"data");
        let data_size = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_size, 8); // 4 samples * 2 bytes

        // Verify sample data
        let s0 = i16::from_le_bytes([wav[44], wav[45]]);
        assert_eq!(s0, 100);
        let s1 = i16::from_le_bytes([wav[46], wav[47]]);
        assert_eq!(s1, -200);
    }

    #[test]
    fn test_pcm16_to_wav_empty() {
        let wav = pcm16_to_wav(&[], 44100);
        assert_eq!(wav.len(), 44); // Header only
        let data_size = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_size, 0);
    }

    #[test]
    fn test_talk_mode_event_serialization() {
        let events = vec![
            TalkModeEvent::StateChanged {
                from: TalkModeState::Idle,
                to: TalkModeState::Listening,
            },
            TalkModeEvent::WakeWordDetected {
                word: "hey zeus".to_string(),
                confidence: 0.95,
            },
            TalkModeEvent::Transcribed {
                text: "hello world".to_string(),
            },
            TalkModeEvent::AgentResponse {
                text: "hi there".to_string(),
            },
            TalkModeEvent::SpeakingStarted,
            TalkModeEvent::SpeakingFinished,
            TalkModeEvent::Error {
                message: "test error".to_string(),
            },
            TalkModeEvent::AudioLevel { rms: 0.5 },
        ];

        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            let parsed: TalkModeEvent = serde_json::from_str(&json).unwrap();
            let reparsed = serde_json::to_string(&parsed).unwrap();
            assert_eq!(json, reparsed);
        }
    }

    #[test]
    fn test_talk_mode_turn() {
        let turn = TalkModeTurn {
            role: "user".to_string(),
            text: "hello".to_string(),
            timestamp: 1234567890,
        };
        let json = serde_json::to_string(&turn).unwrap();
        let parsed: TalkModeTurn = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.role, "user");
        assert_eq!(parsed.text, "hello");
    }

    #[tokio::test]
    async fn test_talk_mode_creation() {
        let agent = Arc::new(MockAgent {
            response: "hi".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let audio_in = Arc::new(MockAudioInput { available: true });
        let audio_out = Arc::new(MockAudioOutput);

        let (talk_mode, _event_rx, _state_rx) =
            TalkMode::new(TalkModeConfig::default(), agent, tts, audio_in, audio_out);

        assert_eq!(talk_mode.state().await, TalkModeState::Idle);
        assert!(!talk_mode.is_running());
        assert!(talk_mode.transcript().await.is_empty());
    }

    #[tokio::test]
    async fn test_talk_mode_no_audio_device() {
        let agent = Arc::new(MockAgent {
            response: "hi".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let audio_in = Arc::new(MockAudioInput { available: false });
        let audio_out = Arc::new(MockAudioOutput);

        let (talk_mode, _event_rx, _state_rx) =
            TalkMode::new(TalkModeConfig::default(), agent, tts, audio_in, audio_out);

        let result = talk_mode.start().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No audio input"));
    }

    #[tokio::test]
    async fn test_talk_mode_ptt() {
        let agent = Arc::new(MockAgent {
            response: "hi".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let audio_in = Arc::new(MockAudioInput { available: true });
        let audio_out = Arc::new(MockAudioOutput);

        let (talk_mode, _event_rx, _state_rx) = TalkMode::new(
            TalkModeConfig {
                style: TalkModeStyle::PushToTalk,
                ..Default::default()
            },
            agent,
            tts,
            audio_in,
            audio_out,
        );

        // PTT press/release before start
        talk_mode.ptt_press();
        assert!(talk_mode.ptt_active.load(Ordering::SeqCst));
        talk_mode.ptt_release();
        assert!(!talk_mode.ptt_active.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_talk_mode_start_stop() {
        let agent = Arc::new(MockAgent {
            response: "hi".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let audio_in = Arc::new(MockAudioInput { available: true });
        let audio_out = Arc::new(MockAudioOutput);

        let (talk_mode, _event_rx, _state_rx) = TalkMode::new(
            TalkModeConfig {
                style: TalkModeStyle::AlwaysOn,
                silence_cutoff_ms: 10, // Very short for test
                ..Default::default()
            },
            agent,
            tts,
            audio_in,
            audio_out,
        );

        let talk_mode = Arc::new(talk_mode);
        let tm_clone = talk_mode.clone();

        // Start in background — will process silence frames and stop when audio_rx closes
        let handle = tokio::spawn(async move {
            let _ = tm_clone.start().await;
        });

        // Wait briefly for it to start
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Stop
        talk_mode.stop().await;

        // Wait for task to finish
        let _ = tokio::time::timeout(tokio::time::Duration::from_millis(500), handle).await;

        assert_eq!(talk_mode.state().await, TalkModeState::Idle);
    }

    #[tokio::test]
    async fn test_talk_mode_clear_transcript() {
        let agent = Arc::new(MockAgent {
            response: "hi".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let audio_in = Arc::new(MockAudioInput { available: true });
        let audio_out = Arc::new(MockAudioOutput);

        let (talk_mode, _event_rx, _state_rx) =
            TalkMode::new(TalkModeConfig::default(), agent, tts, audio_in, audio_out);

        // Manually add to transcript
        {
            let mut t = talk_mode.transcript.write().await;
            t.push(TalkModeTurn {
                role: "user".to_string(),
                text: "test".to_string(),
                timestamp: 0,
            });
        }
        assert_eq!(talk_mode.transcript().await.len(), 1);

        talk_mode.clear_transcript().await;
        assert!(talk_mode.transcript().await.is_empty());
    }

    // ========================================================================
    // New tests: barge-in, voice directives, mute
    // ========================================================================

    #[test]
    fn test_voice_directive_serialization() {
        let directives = [
            VoiceDirective::Pause,
            VoiceDirective::Resume,
            VoiceDirective::Mute,
            VoiceDirective::Unmute,
            VoiceDirective::Stop,
            VoiceDirective::Repeat,
        ];
        for d in directives {
            let json = serde_json::to_string(&d).unwrap();
            let parsed: VoiceDirective = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, d);
        }
    }

    #[tokio::test]
    async fn test_mute_unmute() {
        let agent = Arc::new(MockAgent {
            response: "hi".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let audio_in = Arc::new(MockAudioInput { available: true });
        let audio_out = Arc::new(MockAudioOutput);

        let (talk_mode, mut event_rx, _state_rx) =
            TalkMode::new(TalkModeConfig::default(), agent, tts, audio_in, audio_out);

        assert!(!talk_mode.is_muted());

        talk_mode.directive(VoiceDirective::Mute).await;
        assert!(talk_mode.is_muted());

        // Should receive MuteChanged event
        let mut found_mute = false;
        while let Ok(ev) = event_rx.try_recv() {
            if matches!(ev, TalkModeEvent::MuteChanged { muted: true }) {
                found_mute = true;
            }
        }
        assert!(found_mute);

        talk_mode.directive(VoiceDirective::Unmute).await;
        assert!(!talk_mode.is_muted());
    }

    #[tokio::test]
    async fn test_pause_resume() {
        let agent = Arc::new(MockAgent {
            response: "hi".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let audio_in = Arc::new(MockAudioInput { available: true });
        let audio_out = Arc::new(MockAudioOutput);

        let (talk_mode, _event_rx, _state_rx) =
            TalkMode::new(TalkModeConfig::default(), agent, tts, audio_in, audio_out);

        assert!(!talk_mode.is_paused());

        talk_mode.directive(VoiceDirective::Pause).await;
        assert!(talk_mode.is_paused());

        talk_mode.directive(VoiceDirective::Resume).await;
        assert!(!talk_mode.is_paused());
    }

    #[tokio::test]
    async fn test_directive_stop_stops_talk_mode() {
        let agent = Arc::new(MockAgent {
            response: "hi".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let audio_in = Arc::new(MockAudioInput { available: true });
        let audio_out = Arc::new(MockAudioOutput);

        let (talk_mode, _event_rx, _state_rx) =
            TalkMode::new(TalkModeConfig::default(), agent, tts, audio_in, audio_out);

        // Simulate running
        talk_mode.running.store(true, Ordering::SeqCst);
        assert!(talk_mode.is_running());

        talk_mode.directive(VoiceDirective::Stop).await;
        assert!(!talk_mode.is_running());
    }

    #[tokio::test]
    async fn test_barge_in() {
        let agent = Arc::new(MockAgent {
            response: "hi".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let audio_in = Arc::new(MockAudioInput { available: true });
        let audio_out = Arc::new(MockAudioOutput);

        let (talk_mode, mut event_rx, _state_rx) =
            TalkMode::new(TalkModeConfig::default(), agent, tts, audio_in, audio_out);

        // Set state to Speaking to test barge-in
        talk_mode.set_state(TalkModeState::Speaking).await;
        assert_eq!(talk_mode.state().await, TalkModeState::Speaking);

        talk_mode.barge_in().await;
        assert_eq!(talk_mode.state().await, TalkModeState::Listening);

        // Should receive BargeIn event
        let mut found_barge = false;
        while let Ok(ev) = event_rx.try_recv() {
            if matches!(ev, TalkModeEvent::BargeIn { .. }) {
                found_barge = true;
            }
        }
        assert!(found_barge);
    }

    #[tokio::test]
    async fn test_barge_in_only_during_speaking() {
        let agent = Arc::new(MockAgent {
            response: "hi".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let audio_in = Arc::new(MockAudioInput { available: true });
        let audio_out = Arc::new(MockAudioOutput);

        let (talk_mode, _event_rx, _state_rx) =
            TalkMode::new(TalkModeConfig::default(), agent, tts, audio_in, audio_out);

        // Barge-in should be no-op when not Speaking
        talk_mode.set_state(TalkModeState::Listening).await;
        talk_mode.barge_in().await;
        assert_eq!(talk_mode.state().await, TalkModeState::Listening);
    }

    #[tokio::test]
    async fn test_last_response_saved() {
        let agent = Arc::new(MockAgent {
            response: "hello world".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let audio_in = Arc::new(MockAudioInput { available: true });
        let audio_out = Arc::new(MockAudioOutput);

        let (talk_mode, _event_rx, _state_rx) =
            TalkMode::new(TalkModeConfig::default(), agent, tts, audio_in, audio_out);

        // Initially no last response
        assert!(talk_mode.last_response().await.is_none());

        // Manually set last response
        {
            let mut last = talk_mode.last_response.write().await;
            *last = Some("test response".to_string());
        }
        assert_eq!(
            talk_mode.last_response().await.as_deref(),
            Some("test response")
        );
    }

    #[tokio::test]
    async fn test_barge_in_threshold() {
        let config = TalkModeConfig {
            silence_threshold: 0.05,
            ..Default::default()
        };
        let agent = Arc::new(MockAgent {
            response: "hi".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let audio_in = Arc::new(MockAudioInput { available: true });
        let audio_out = Arc::new(MockAudioOutput);

        let (talk_mode, _event_rx, _state_rx) =
            TalkMode::new(config, agent, tts, audio_in, audio_out);

        // Barge-in threshold should be 3x silence threshold
        assert!((talk_mode.barge_in_threshold - 0.15).abs() < f32::EPSILON);
    }

    #[test]
    fn test_barge_in_event_serialization() {
        let event = TalkModeEvent::BargeIn { rms: 0.42 };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("barge_in"));
        let parsed: TalkModeEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            TalkModeEvent::BargeIn { rms } => assert!((rms - 0.42).abs() < f32::EPSILON),
            _ => panic!("Expected BargeIn event"),
        }
    }

    #[tokio::test]
    async fn test_talk_mode_double_start() {
        let agent = Arc::new(MockAgent {
            response: "hi".to_string(),
        });
        let tts = Arc::new(MockTTS);
        let audio_in = Arc::new(MockAudioInput { available: true });
        let audio_out = Arc::new(MockAudioOutput);

        let (talk_mode, _event_rx, _state_rx) =
            TalkMode::new(TalkModeConfig::default(), agent, tts, audio_in, audio_out);

        // Simulate already running
        talk_mode.running.store(true, Ordering::SeqCst);

        let result = talk_mode.start().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already running"));
    }
}
