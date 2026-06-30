//! Discord voice channel integration via songbird
//!
//! Enables Zeus to join Discord voice channels and interact via speech:
//! 1. User speaks in voice channel → songbird receives Opus → decode to PCM
//! 2. VAD detects speech end → accumulate PCM → wrap as WAV → STT (Whisper)
//! 3. Transcribed text → agent processing → response text
//! 4. Response text → TTS (Piper/OpenAI) → WAV → songbird playback
//!
//! ## Architecture
//!
//! ```text
//! Discord Voice ──→ songbird ──→ Opus decode ──→ PCM 48kHz
//!                                                    │
//!                                          VAD (speech/silence)
//!                                                    │
//!                                          Resample → 16kHz WAV
//!                                                    │
//!                                              STT (Whisper)
//!                                                    │
//!                                            Agent → response
//!                                                    │
//!                                        TTS (Piper) → WAV
//!                                                    │
//!                                          songbird playback
//!                                                    │
//! Discord Voice ←── songbird ←── Opus encode ←── PCM
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use songbird::events::context_data::VoiceTick;
use songbird::id::{ChannelId as SbChannelId, GuildId as SbGuildId};
use songbird::{
    Config as SongbirdConfig, CoreEvent, Event, EventContext, EventHandler as VoiceEventHandler,
    Songbird,
};
use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, mpsc};
use tracing::{debug, info, warn};
use zeus_core::{Error, Result};

// ============================================================================
// Configuration
// ============================================================================

/// Discord voice channel configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordVoiceConfig {
    /// Auto-join these voice channels on startup (guild_id:channel_id pairs)
    #[serde(default)]
    pub auto_join_channels: Vec<String>,

    /// Minimum speech duration in ms before transcribing (prevents noise triggers)
    #[serde(default = "default_min_speech_ms")]
    pub min_speech_ms: u64,

    /// Silence duration in ms to detect end of speech
    #[serde(default = "default_silence_timeout_ms")]
    pub silence_timeout_ms: u64,

    /// Energy threshold for VAD (RMS amplitude, 0.0–1.0)
    #[serde(default = "default_energy_threshold")]
    pub energy_threshold: f64,

    /// TTS voice identifier
    #[serde(default = "default_tts_voice")]
    pub tts_voice: String,

    /// TTS provider: "piper", "openai", "elevenlabs"
    #[serde(default = "default_tts_provider")]
    pub tts_provider: String,

    /// Piper TTS server URL
    #[serde(default)]
    pub piper_url: Option<String>,

    /// STT provider: "groq" or "openai"
    #[serde(default)]
    pub stt_provider: Option<String>,

    /// Only respond when @mentioned or keyword detected
    #[serde(default)]
    pub require_wake_word: bool,

    /// Wake words that trigger processing (if require_wake_word is true)
    #[serde(default = "default_wake_words")]
    pub wake_words: Vec<String>,
}

fn default_min_speech_ms() -> u64 {
    500
}
fn default_silence_timeout_ms() -> u64 {
    1500
}
fn default_energy_threshold() -> f64 {
    0.02
}
fn default_tts_voice() -> String {
    "en_US-amy-medium".to_string()
}
fn default_tts_provider() -> String {
    "piper".to_string()
}
fn default_wake_words() -> Vec<String> {
    vec!["zeus".to_string(), "hey zeus".to_string()]
}

impl Default for DiscordVoiceConfig {
    fn default() -> Self {
        Self {
            auto_join_channels: Vec::new(),
            min_speech_ms: default_min_speech_ms(),
            silence_timeout_ms: default_silence_timeout_ms(),
            energy_threshold: default_energy_threshold(),
            tts_voice: default_tts_voice(),
            tts_provider: default_tts_provider(),
            piper_url: None,
            stt_provider: None,
            require_wake_word: false,
            wake_words: default_wake_words(),
        }
    }
}

// ============================================================================
// Voice event: transcribed speech from a user
// ============================================================================

/// A transcribed voice message from a Discord voice channel user
#[derive(Debug, Clone)]
pub struct VoiceTranscript {
    /// Guild ID
    pub guild_id: u64,
    /// Voice channel ID
    pub channel_id: u64,
    /// Speaking user's Discord ID
    pub user_id: u64,
    /// Transcribed text
    pub text: String,
    /// Duration of speech in milliseconds
    pub speech_duration_ms: u64,
}

// ============================================================================
// Per-user audio accumulator
// ============================================================================

/// Tracks audio state for a single user in a voice channel
struct UserAudioState {
    /// Accumulated PCM samples (48kHz stereo i16)
    pcm_buffer: Vec<i16>,
    /// Whether user is currently speaking (energy above threshold)
    is_speaking: bool,
    /// Consecutive silent frames since last speech
    silence_frames: u32,
    /// Total speech frames in current utterance
    speech_frames: u32,
    /// Timestamp of last speech activity
    _last_speech_ms: u64,
}

impl UserAudioState {
    fn new() -> Self {
        Self {
            pcm_buffer: Vec::with_capacity(48000 * 10), // 10 seconds buffer
            is_speaking: false,
            silence_frames: 0,
            speech_frames: 0,
            _last_speech_ms: 0,
        }
    }

    /// Calculate RMS energy of a PCM frame
    fn rms_energy(samples: &[i16]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
        (sum_sq / samples.len() as f64).sqrt() / 32768.0
    }

    /// Feed audio samples and return true if an utterance is complete
    fn feed(
        &mut self,
        samples: &[i16],
        energy_threshold: f64,
        silence_frames_threshold: u32,
    ) -> bool {
        let energy = Self::rms_energy(samples);

        if energy > energy_threshold {
            // Speech detected
            self.is_speaking = true;
            self.silence_frames = 0;
            self.speech_frames += 1;
            self.pcm_buffer.extend_from_slice(samples);
            false
        } else if self.is_speaking {
            // Silence after speech — hangover
            self.silence_frames += 1;
            // Still buffer a few silence frames (smooth edges)
            if self.silence_frames <= 5 {
                self.pcm_buffer.extend_from_slice(samples);
            }

            if self.silence_frames >= silence_frames_threshold {
                // End of utterance
                self.is_speaking = false;
                true
            } else {
                false
            }
        } else {
            // Pure silence, not speaking
            false
        }
    }

    /// Take the accumulated utterance and reset state
    fn take_utterance(&mut self) -> (Vec<i16>, u32) {
        let pcm = std::mem::take(&mut self.pcm_buffer);
        let frames = self.speech_frames;
        self.speech_frames = 0;
        self.silence_frames = 0;
        self.is_speaking = false;
        (pcm, frames)
    }
}

// ============================================================================
// Songbird voice event handler (receives decoded audio)
// ============================================================================

/// Songbird event handler that receives decoded audio per user
struct VoiceReceiveHandler {
    /// Per-user audio state
    users: Arc<Mutex<HashMap<u64, UserAudioState>>>,
    /// SSRC → Discord user_id mapping (populated from SpeakingStateUpdate events)
    ssrc_to_user: Arc<Mutex<HashMap<u32, u64>>>,
    /// Channel to send completed transcripts for processing
    transcript_tx: mpsc::Sender<VoiceTranscript>,
    /// Voice config
    config: DiscordVoiceConfig,
    /// Guild and channel IDs
    guild_id: u64,
    channel_id: u64,
}

#[async_trait]
impl VoiceEventHandler for VoiceReceiveHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        match ctx {
            EventContext::VoiceTick(tick) => {
                self.handle_voice_tick(tick).await;
            }
            EventContext::SpeakingStateUpdate(_) => {
                // Handled by SpeakingUpdateHandler (shares ssrc_to_user map)
            }
            EventContext::ClientDisconnect(disconnect) => {
                info!(
                    "Discord voice: user disconnected (user_id={})",
                    disconnect.user_id.0
                );
                // Clean up user state
                let mut users = self.users.lock().await;
                users.remove(&disconnect.user_id.0);
            }
            _ => {}
        }
        None
    }
}

impl VoiceReceiveHandler {
    async fn handle_voice_tick(&self, tick: &VoiceTick) {
        // Silence timeout in 20ms frames (discord sends audio in 20ms chunks)
        let silence_frames = (self.config.silence_timeout_ms / 20) as u32;
        let min_speech_frames = (self.config.min_speech_ms / 20) as u32;

        let mut users = self.users.lock().await;

        // Snapshot SSRC→user_id map for this tick (clone to release lock early)
        let ssrc_map = self.ssrc_to_user.lock().await.clone();

        for (&ssrc, data) in &tick.speaking {
            // Resolve real Discord user_id from SSRC, fall back to SSRC if unknown
            let user_id = ssrc_map.get(&ssrc).copied().unwrap_or(ssrc as u64);

            // Get or create user state keyed by resolved user_id
            let user_state = users.entry(user_id).or_insert_with(UserAudioState::new);

            // songbird provides decoded_voice as Option<Vec<i16>> when DecodeMode::Decode is set
            if let Some(ref decoded) = data.decoded_voice {
                let utterance_complete =
                    user_state.feed(decoded, self.config.energy_threshold, silence_frames);

                if utterance_complete {
                    let (pcm, speech_frames) = user_state.take_utterance();

                    // Skip if too short
                    if speech_frames < min_speech_frames {
                        debug!(
                            ssrc,
                            speech_frames,
                            min_speech_frames,
                            "Discord voice: utterance too short, skipping"
                        );
                        continue;
                    }

                    let speech_duration_ms = speech_frames as u64 * 20;
                    debug!(
                        ssrc,
                        speech_frames,
                        speech_duration_ms,
                        pcm_samples = pcm.len(),
                        "Discord voice: utterance complete, sending for transcription"
                    );

                    // Send for async transcription (non-blocking)
                    let transcript = VoiceTranscript {
                        guild_id: self.guild_id,
                        channel_id: self.channel_id,
                        user_id,
                        text: String::new(), // Will be filled by transcription
                        speech_duration_ms,
                    };

                    // Spawn transcription task to avoid blocking the voice tick
                    let tx = self.transcript_tx.clone();
                    let config = self.config.clone();
                    tokio::spawn(async move {
                        match transcribe_pcm_48k(&pcm, config.stt_provider.as_deref()).await {
                            Ok(text) if !text.is_empty() => {
                                let mut t = transcript;
                                t.text = text;

                                // Wake word check
                                if config.require_wake_word {
                                    let lower = t.text.to_lowercase();
                                    if !config.wake_words.iter().any(|w| lower.contains(w)) {
                                        debug!(
                                            text = %t.text,
                                            "Discord voice: no wake word detected, skipping"
                                        );
                                        return;
                                    }
                                }

                                info!(
                                    user_id = t.user_id,
                                    text = %t.text,
                                    duration_ms = t.speech_duration_ms,
                                    "Discord voice: transcribed speech"
                                );
                                let _ = tx.send(t).await;
                            }
                            Ok(_) => {
                                debug!("Discord voice: empty transcription result");
                            }
                            Err(e) => {
                                warn!("Discord voice: STT failed: {}", e);
                            }
                        }
                    });
                }
            }
        }
    }
}

// ============================================================================
// Audio conversion: PCM 48kHz stereo → WAV 16kHz mono for Whisper
// ============================================================================

/// Convert 48kHz stereo PCM (i16) to a WAV file suitable for Whisper STT.
///
/// Steps: stereo→mono, 48kHz→16kHz downsample, wrap in WAV header.
fn pcm_48k_stereo_to_wav_16k(pcm_48k: &[i16]) -> Vec<u8> {
    // 1. Stereo to mono (average L+R channels)
    let mono: Vec<i16> = pcm_48k
        .chunks_exact(2)
        .map(|pair| ((pair[0] as i32 + pair[1] as i32) / 2) as i16)
        .collect();

    // 2. Downsample 48kHz → 16kHz (ratio 3:1)
    let resampled: Vec<i16> = mono
        .chunks(3)
        .map(|chunk| {
            let sum: i32 = chunk.iter().map(|&s| s as i32).sum();
            (sum / chunk.len() as i32) as i16
        })
        .collect();

    // 3. Build WAV header (PCM 16-bit, 16kHz, mono)
    let data_size = (resampled.len() * 2) as u32;
    let file_size = 36 + data_size;
    let sample_rate = 16000u32;

    let mut wav = Vec::with_capacity(44 + data_size as usize);

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
    for &sample in &resampled {
        wav.extend_from_slice(&sample.to_le_bytes());
    }

    wav
}

/// Transcribe PCM 48kHz stereo audio to text via Whisper API.
async fn transcribe_pcm_48k(pcm_48k: &[i16], stt_provider: Option<&str>) -> Result<String> {
    if pcm_48k.is_empty() {
        return Ok(String::new());
    }

    let wav_data = pcm_48k_stereo_to_wav_16k(pcm_48k);

    // Select STT provider and API key
    let (endpoint, model, api_key) = match stt_provider {
        Some("openai") => {
            let key = std::env::var("OPENAI_API_KEY")
                .map_err(|_| Error::Internal("OPENAI_API_KEY not set for STT".into()))?;
            (
                "https://api.openai.com/v1/audio/transcriptions",
                "whisper-1",
                key,
            )
        }
        _ => {
            // Default to Groq (faster, cheaper)
            let key = std::env::var("GROQ_API_KEY")
                .or_else(|_| std::env::var("OPENAI_API_KEY"))
                .map_err(|_| {
                    Error::Internal("GROQ_API_KEY or OPENAI_API_KEY required for STT".into())
                })?;
            if std::env::var("GROQ_API_KEY").is_ok() {
                (
                    "https://api.groq.com/openai/v1/audio/transcriptions",
                    "whisper-large-v3",
                    key,
                )
            } else {
                (
                    "https://api.openai.com/v1/audio/transcriptions",
                    "whisper-1",
                    key,
                )
            }
        }
    };

    debug!(
        "Discord voice STT: {} bytes WAV → {} ({})",
        wav_data.len(),
        endpoint,
        model
    );

    let file_part = reqwest::multipart::Part::bytes(wav_data)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| Error::Internal(format!("MIME error: {e}")))?;

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model", model.to_string())
        .text("language", "en".to_string())
        .text("response_format", "json".to_string());

    let resp = reqwest::Client::new()
        .post(endpoint)
        .header("Authorization", format!("Bearer {api_key}"))
        .multipart(form)
        .send()
        .await
        .map_err(|e| Error::Internal(format!("STT request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Internal(format!("STT API {status}: {body}")));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| Error::Internal(format!("STT JSON parse failed: {e}")))?;

    Ok(json
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string())
}

// ============================================================================
// TTS → WAV → songbird playback
// ============================================================================

/// Synthesize text via TTS and return WAV audio bytes.
async fn synthesize_tts(text: &str, config: &DiscordVoiceConfig) -> Result<Vec<u8>> {
    match config.tts_provider.as_str() {
        "piper" => {
            let env_piper = std::env::var("ZEUS_PIPER_URL").ok();
            let base_url = config
                .piper_url
                .as_deref()
                .or(env_piper.as_deref())
                .unwrap_or("http://localhost:8104");
            let encoded = urlencoding::encode(text);
            let voice = &config.tts_voice;
            let url = format!("{base_url}/api/tts?text={encoded}&voice={voice}&format=wav");

            let resp = reqwest::Client::new()
                .get(&url)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
                .map_err(|e| Error::Internal(format!("Piper TTS failed: {e}")))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(Error::Internal(format!("Piper TTS {status}: {body}")));
            }

            let wav = resp
                .bytes()
                .await
                .map_err(|e| Error::Internal(format!("TTS read failed: {e}")))?
                .to_vec();

            debug!("Discord voice TTS: {} bytes WAV from Piper", wav.len());
            Ok(wav)
        }
        provider => Err(Error::Internal(format!(
            "Unsupported TTS provider for Discord voice: {provider}"
        ))),
    }
}

/// Convert WAV (any sample rate) to PCM f32 samples at the original rate.
///
/// Songbird's `Input::from` accepts raw PCM or various audio containers.
/// We provide WAV bytes directly as songbird can decode them.
#[allow(dead_code)]
fn wav_to_pcm_f32(wav_data: &[u8]) -> Result<(Vec<f32>, u32)> {
    // Parse WAV header to get sample rate and data
    if wav_data.len() < 44 || &wav_data[0..4] != b"RIFF" || &wav_data[8..12] != b"WAVE" {
        return Err(Error::Internal("Invalid WAV data".into()));
    }

    let mut offset = 12;
    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    let mut bits_per_sample = 0u16;
    let mut data_start = 0;
    let mut data_size = 0u32;

    while offset + 8 <= wav_data.len() {
        let chunk_id = &wav_data[offset..offset + 4];
        let chunk_size = u32::from_le_bytes([
            wav_data[offset + 4],
            wav_data[offset + 5],
            wav_data[offset + 6],
            wav_data[offset + 7],
        ]);

        if chunk_id == b"fmt " && chunk_size >= 16 && offset + 24 <= wav_data.len() {
            let fmt = &wav_data[offset + 8..];
            channels = u16::from_le_bytes([fmt[2], fmt[3]]);
            sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
            bits_per_sample = u16::from_le_bytes([fmt[14], fmt[15]]);
        } else if chunk_id == b"data" {
            data_start = offset + 8;
            data_size = chunk_size;
            break;
        }

        offset += 8 + chunk_size as usize;
        if offset % 2 != 0 {
            offset += 1;
        }
    }

    if data_start == 0 || sample_rate == 0 {
        return Err(Error::Internal("WAV: missing fmt or data chunk".into()));
    }

    let data_end = (data_start + data_size as usize).min(wav_data.len());
    let data = &wav_data[data_start..data_end];

    // Convert to f32 samples, mix to mono if stereo
    let samples_i16: Vec<i16> = match bits_per_sample {
        16 => data
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect(),
        8 => data.iter().map(|&b| ((b as i16) - 128) * 256).collect(),
        _ => {
            return Err(Error::Internal(format!(
                "Unsupported bits per sample: {bits_per_sample}"
            )));
        }
    };

    let mono = if channels > 1 {
        samples_i16
            .chunks_exact(channels as usize)
            .map(|ch| {
                let sum: i32 = ch.iter().map(|&s| s as i32).sum();
                (sum / channels as i32) as i16
            })
            .collect()
    } else {
        samples_i16
    };

    let f32_samples: Vec<f32> = mono.iter().map(|&s| s as f32 / 32768.0).collect();

    Ok((f32_samples, sample_rate))
}

// ============================================================================
// Discord Voice Session — manages a single voice channel connection
// ============================================================================

/// Manages a voice channel connection with STT/TTS pipeline
pub struct DiscordVoiceSession {
    /// Songbird manager reference
    songbird: Arc<Songbird>,
    /// Voice config
    config: DiscordVoiceConfig,
    /// Transcript receiver (for the gateway to consume)
    _transcript_rx: Option<mpsc::Receiver<VoiceTranscript>>,
    /// Active guild connections
    active_guilds: Arc<RwLock<HashMap<u64, u64>>>, // guild_id → channel_id
}

impl DiscordVoiceSession {
    /// Create a new voice session manager.
    ///
    /// Returns the session and a songbird instance to register with serenity.
    pub fn new(config: DiscordVoiceConfig) -> (Self, Arc<Songbird>) {
        let songbird = Songbird::serenity();

        let session = Self {
            songbird: songbird.clone(),
            config,
            _transcript_rx: None,
            active_guilds: Arc::new(RwLock::new(HashMap::new())),
        };

        (session, songbird)
    }

    /// Join a voice channel and start listening.
    pub async fn join(
        &mut self,
        guild_id: u64,
        channel_id: u64,
    ) -> Result<mpsc::Receiver<VoiceTranscript>> {
        info!(guild_id, channel_id, "Discord voice: joining voice channel");

        let (transcript_tx, transcript_rx) = mpsc::channel(64);

        let sb_guild = SbGuildId(
            NonZeroU64::new(guild_id).ok_or_else(|| Error::channel("guild_id must be non-zero"))?,
        );
        let sb_channel = SbChannelId(
            NonZeroU64::new(channel_id)
                .ok_or_else(|| Error::channel("channel_id must be non-zero"))?,
        );

        // Join the voice channel
        let handler = self
            .songbird
            .join(sb_guild, sb_channel)
            .await
            .map_err(|e| Error::channel(format!("Failed to join voice channel: {e}")))?;

        // Register event handlers for audio reception
        {
            let mut call = handler.lock().await;

            // Configure decode mode so we get decoded PCM in VoiceTick events
            let config =
                SongbirdConfig::default().decode_mode(songbird::driver::DecodeMode::Decode);
            call.set_config(config);

            // Shared SSRC → user_id map between VoiceTick and SpeakingStateUpdate handlers
            let ssrc_to_user = Arc::new(Mutex::new(HashMap::new()));

            let receiver = VoiceReceiveHandler {
                users: Arc::new(Mutex::new(HashMap::new())),
                ssrc_to_user: ssrc_to_user.clone(),
                transcript_tx: transcript_tx.clone(),
                config: self.config.clone(),
                guild_id,
                channel_id,
            };

            // Handler that populates SSRC → user_id from SpeakingStateUpdate events
            let speaking_handler = SpeakingUpdateHandler { ssrc_to_user };

            // Listen for decoded audio ticks
            call.add_global_event(CoreEvent::VoiceTick.into(), receiver);

            // Listen for SSRC → user_id mapping events
            call.add_global_event(CoreEvent::SpeakingStateUpdate.into(), speaking_handler);

            // Listen for disconnect events
            let disconnect_handler = DisconnectHandler;
            call.add_global_event(CoreEvent::ClientDisconnect.into(), disconnect_handler);
        }

        // Track active connection
        self.active_guilds
            .write()
            .await
            .insert(guild_id, channel_id);

        info!(guild_id, channel_id, "Discord voice: joined and listening");

        Ok(transcript_rx)
    }

    /// Leave a voice channel.
    pub async fn leave(&self, guild_id: u64) -> Result<()> {
        let sb_guild = SbGuildId(
            NonZeroU64::new(guild_id).ok_or_else(|| Error::channel("guild_id must be non-zero"))?,
        );
        self.songbird
            .leave(sb_guild)
            .await
            .map_err(|e| Error::channel(format!("Failed to leave voice channel: {e}")))?;

        self.active_guilds.write().await.remove(&guild_id);
        info!(guild_id, "Discord voice: left voice channel");
        Ok(())
    }

    /// Speak text in a voice channel (TTS → playback).
    pub async fn speak(&self, guild_id: u64, text: &str) -> Result<()> {
        let sb_guild = SbGuildId(
            NonZeroU64::new(guild_id).ok_or_else(|| Error::channel("guild_id must be non-zero"))?,
        );
        let handler = self
            .songbird
            .get(sb_guild)
            .ok_or_else(|| Error::channel("Not in a voice channel for this guild"))?;

        // Synthesize TTS
        let wav_data = synthesize_tts(text, &self.config).await?;

        // Create songbird input from WAV bytes (symphonia decodes the WAV container)
        let input = songbird::input::Input::from(wav_data);

        // Play audio
        let mut call = handler.lock().await;
        let _track = call.play_input(input);

        debug!(
            guild_id,
            text_len = text.len(),
            "Discord voice: playing TTS response"
        );
        Ok(())
    }

    /// Check if currently in a voice channel for a guild.
    pub async fn is_in_voice(&self, guild_id: u64) -> bool {
        self.active_guilds.read().await.contains_key(&guild_id)
    }

    /// Get the songbird manager reference.
    pub fn songbird(&self) -> &Arc<Songbird> {
        &self.songbird
    }

    /// Get current voice config.
    pub fn config(&self) -> &DiscordVoiceConfig {
        &self.config
    }
}

// ============================================================================
// Simple event handlers for connection tracking
// ============================================================================

/// Populates the shared SSRC → Discord user_id map from SpeakingStateUpdate events.
struct SpeakingUpdateHandler {
    ssrc_to_user: Arc<Mutex<HashMap<u32, u64>>>,
}

#[async_trait]
impl VoiceEventHandler for SpeakingUpdateHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::SpeakingStateUpdate(speaking) = ctx
            && let Some(user_id) = speaking.user_id
        {
            self.ssrc_to_user
                .lock()
                .await
                .insert(speaking.ssrc, user_id.0);
            debug!(
                ssrc = speaking.ssrc,
                user_id = user_id.0,
                speaking = ?speaking.speaking,
                "Discord voice: SSRC mapped to user"
            );
        }
        None
    }
}

struct DisconnectHandler;

#[async_trait]
impl VoiceEventHandler for DisconnectHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::ClientDisconnect(disconnect) = ctx {
            info!(
                user_id = disconnect.user_id.0,
                "Discord voice: user left voice channel"
            );
        }
        None
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discord_voice_config_defaults() {
        let config = DiscordVoiceConfig::default();
        assert_eq!(config.min_speech_ms, 500);
        assert_eq!(config.silence_timeout_ms, 1500);
        assert!((config.energy_threshold - 0.02).abs() < f64::EPSILON);
        assert_eq!(config.tts_voice, "en_US-amy-medium");
        assert_eq!(config.tts_provider, "piper");
        assert!(!config.require_wake_word);
        assert_eq!(config.wake_words, vec!["zeus", "hey zeus"]);
        assert!(config.auto_join_channels.is_empty());
    }

    #[test]
    fn test_discord_voice_config_serialization() {
        let config = DiscordVoiceConfig {
            auto_join_channels: vec!["123:456".to_string()],
            min_speech_ms: 300,
            silence_timeout_ms: 2000,
            energy_threshold: 0.05,
            tts_voice: "en_GB-alan-medium".to_string(),
            tts_provider: "piper".to_string(),
            piper_url: Some("http://localhost:8104".to_string()),
            stt_provider: Some("groq".to_string()),
            require_wake_word: true,
            wake_words: vec!["hey zeus".to_string()],
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: DiscordVoiceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.min_speech_ms, 300);
        assert_eq!(parsed.silence_timeout_ms, 2000);
        assert!(parsed.require_wake_word);
        assert_eq!(parsed.auto_join_channels, vec!["123:456"]);
    }

    #[test]
    fn test_user_audio_state_rms_energy() {
        // Silence
        let silence = vec![0i16; 100];
        assert_eq!(UserAudioState::rms_energy(&silence), 0.0);

        // Non-silence
        let loud: Vec<i16> = vec![16384; 100]; // ~0.5 amplitude
        let energy = UserAudioState::rms_energy(&loud);
        assert!(energy > 0.4 && energy < 0.6);

        // Empty
        assert_eq!(UserAudioState::rms_energy(&[]), 0.0);
    }

    #[test]
    fn test_user_audio_state_feed_silence() {
        let mut state = UserAudioState::new();
        let silence = vec![0i16; 960]; // 20ms at 48kHz

        // Feeding pure silence should never trigger utterance completion
        for _ in 0..100 {
            let complete = state.feed(&silence, 0.02, 75);
            assert!(!complete);
        }
        assert!(!state.is_speaking);
        assert_eq!(state.speech_frames, 0);
    }

    #[test]
    fn test_user_audio_state_feed_speech_then_silence() {
        let mut state = UserAudioState::new();
        let speech = vec![10000i16; 960]; // Loud audio
        let silence = vec![0i16; 960];

        // Feed speech frames
        for _ in 0..10 {
            state.feed(&speech, 0.02, 5);
        }
        assert!(state.is_speaking);
        assert_eq!(state.speech_frames, 10);

        // Feed silence until utterance completes
        let mut completed = false;
        for _ in 0..10 {
            if state.feed(&silence, 0.02, 5) {
                completed = true;
                break;
            }
        }
        assert!(completed);

        // Take the utterance
        let (pcm, frames) = state.take_utterance();
        assert_eq!(frames, 10);
        assert!(!pcm.is_empty());
    }

    #[test]
    fn test_pcm_48k_stereo_to_wav_16k() {
        // 1 second of 48kHz stereo silence
        let pcm = vec![0i16; 48000 * 2]; // stereo = 2 samples per frame
        let wav = pcm_48k_stereo_to_wav_16k(&pcm);

        // Should have WAV header + PCM data
        assert!(wav.len() > 44);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");

        // Verify sample rate is 16kHz
        let sample_rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
        assert_eq!(sample_rate, 16000);

        // Verify mono (1 channel)
        let channels = u16::from_le_bytes([wav[22], wav[23]]);
        assert_eq!(channels, 1);

        // Verify data size: 48000 stereo → 48000 mono → 16000 (3:1) = ~16000 samples × 2 bytes
        let data_size = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        let expected_samples = 48000 / 3; // 48k mono → 16k
        assert!((data_size as i32 - (expected_samples * 2) as i32).abs() <= 4);
    }

    #[test]
    fn test_pcm_48k_stereo_to_wav_16k_preserves_audio() {
        // Create a known pattern
        let mut pcm = Vec::new();
        for i in 0..4800 {
            let val = (i % 100) as i16 * 100;
            pcm.push(val); // Left
            pcm.push(val); // Right (same = mono)
        }
        let wav = pcm_48k_stereo_to_wav_16k(&pcm);

        // Parse it back
        let (f32_samples, rate) = wav_to_pcm_f32(&wav).unwrap();
        assert_eq!(rate, 16000);
        assert!(!f32_samples.is_empty());
    }

    #[test]
    fn test_wav_to_pcm_f32_valid() {
        // Build a minimal WAV
        let samples = vec![0i16, 1000, -1000, 500];
        let mut wav = Vec::new();
        let data_size = (samples.len() * 2) as u32;
        let file_size = 36 + data_size;

        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&file_size.to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&32000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        for &s in &samples {
            wav.extend_from_slice(&s.to_le_bytes());
        }

        let (f32s, rate) = wav_to_pcm_f32(&wav).unwrap();
        assert_eq!(rate, 16000);
        assert_eq!(f32s.len(), 4);
        assert!((f32s[0] - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_wav_to_pcm_f32_invalid() {
        assert!(wav_to_pcm_f32(&[]).is_err());
        assert!(wav_to_pcm_f32(&[0u8; 44]).is_err()); // No RIFF header
    }

    #[test]
    fn test_voice_transcript_fields() {
        let t = VoiceTranscript {
            guild_id: 123,
            channel_id: 456,
            user_id: 789,
            text: "hello zeus".to_string(),
            speech_duration_ms: 1500,
        };
        assert_eq!(t.guild_id, 123);
        assert_eq!(t.text, "hello zeus");
    }
}
