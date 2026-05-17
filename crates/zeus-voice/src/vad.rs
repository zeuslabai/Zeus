//! Voice Activity Detection (VAD) — Detect speech vs silence in audio
//!
//! Provides energy-based and zero-crossing-rate (ZCR) voice activity
//! detection for the talk mode pipeline. Runs on raw PCM audio frames
//! and emits speech start/end events with configurable thresholds.
//!
//! ```text
//! Audio frames → VAD → SpeechStart / SpeechEnd events
//! ```
//!
//! Features:
//! - Energy threshold with adaptive noise floor
//! - Zero-crossing rate as secondary speech indicator
//! - Hangover frames to prevent choppy speech segmentation
//! - Pre-speech buffer to capture speech onset
//! - Per-session statistics (speech duration, silence duration, segments)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tracing::debug;

// ============================================================================
// Configuration
// ============================================================================

/// VAD configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VadConfig {
    /// Energy threshold for speech detection (RMS amplitude)
    pub energy_threshold: f64,
    /// Zero-crossing rate threshold (crossings per frame)
    pub zcr_threshold: f64,
    /// Enable adaptive noise floor estimation
    pub adaptive_threshold: bool,
    /// Noise floor adaptation rate (0.0–1.0, lower = slower adaptation)
    pub adaptation_rate: f64,
    /// Number of consecutive speech frames before triggering SpeechStart
    pub speech_onset_frames: u32,
    /// Number of consecutive silence frames before triggering SpeechEnd (hangover)
    pub hangover_frames: u32,
    /// Number of pre-speech frames to buffer (captures speech onset)
    pub pre_speech_buffer_frames: usize,
    /// Sample rate in Hz (for ZCR normalization)
    pub sample_rate: u32,
    /// Frame size in samples
    pub frame_size: usize,
    /// Minimum speech segment duration in frames
    pub min_speech_frames: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            energy_threshold: 0.02,
            zcr_threshold: 0.3,
            adaptive_threshold: true,
            adaptation_rate: 0.05,
            speech_onset_frames: 3,
            hangover_frames: 15,
            pre_speech_buffer_frames: 5,
            sample_rate: 16000,
            frame_size: 512,
            min_speech_frames: 5,
        }
    }
}

// ============================================================================
// VAD Events
// ============================================================================

/// Events emitted by the VAD
#[derive(Debug, Clone, PartialEq)]
pub enum VadEvent {
    /// Speech activity detected — audio is voice
    SpeechStart,
    /// Speech ended — audio returned to silence
    SpeechEnd {
        /// Duration of the speech segment in frames
        duration_frames: u32,
    },
    /// Frame processed but no state transition
    NoChange { is_speech: bool },
}

// ============================================================================
// VAD State
// ============================================================================

/// Internal VAD state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VadState {
    /// No speech detected
    Silence,
    /// Potential speech onset (counting consecutive speech frames)
    SpeechOnset,
    /// Active speech
    Speech,
    /// Potential speech end (hangover counting)
    SpeechHangover,
}

impl std::fmt::Display for VadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Silence => write!(f, "silence"),
            Self::SpeechOnset => write!(f, "speech-onset"),
            Self::Speech => write!(f, "speech"),
            Self::SpeechHangover => write!(f, "speech-hangover"),
        }
    }
}

// ============================================================================
// Frame Analysis
// ============================================================================

/// Analysis results for a single audio frame
#[derive(Debug, Clone)]
pub struct FrameAnalysis {
    /// RMS energy of the frame
    pub energy: f64,
    /// Zero-crossing rate (normalized 0.0–1.0)
    pub zcr: f64,
    /// Whether this frame is classified as speech
    pub is_speech: bool,
    /// Current noise floor estimate
    pub noise_floor: f64,
}

// ============================================================================
// VAD Engine
// ============================================================================

/// Voice Activity Detection engine
pub struct VadEngine {
    config: VadConfig,
    state: VadState,
    /// Consecutive speech frames counter (for onset detection)
    speech_count: u32,
    /// Consecutive silence frames counter (for hangover)
    silence_count: u32,
    /// Current speech segment length in frames
    segment_frames: u32,
    /// Adaptive noise floor estimate
    noise_floor: f64,
    /// Pre-speech ring buffer
    pre_speech_buffer: VecDeque<Vec<f32>>,
    /// Statistics
    stats: VadStats,
    /// Timestamp of last speech start
    speech_start_time: Option<DateTime<Utc>>,
}

/// VAD statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VadStats {
    pub total_frames: u64,
    pub speech_frames: u64,
    pub silence_frames: u64,
    pub speech_segments: u64,
    pub total_speech_duration_frames: u64,
    pub shortest_segment_frames: Option<u64>,
    pub longest_segment_frames: Option<u64>,
}

impl VadEngine {
    pub fn new(config: VadConfig) -> Self {
        let noise_floor = config.energy_threshold * 0.5;
        Self {
            config,
            state: VadState::Silence,
            speech_count: 0,
            silence_count: 0,
            segment_frames: 0,
            noise_floor,
            pre_speech_buffer: VecDeque::new(),
            stats: VadStats::default(),
            speech_start_time: None,
        }
    }

    /// Process a single audio frame (f32 PCM samples, -1.0 to 1.0)
    pub fn process_frame(&mut self, samples: &[f32]) -> VadEvent {
        self.stats.total_frames += 1;

        let analysis = self.analyze_frame(samples);

        // Buffer for pre-speech capture
        if self.state == VadState::Silence || self.state == VadState::SpeechOnset {
            self.pre_speech_buffer.push_back(samples.to_vec());
            while self.pre_speech_buffer.len() > self.config.pre_speech_buffer_frames {
                self.pre_speech_buffer.pop_front();
            }
        }

        // Update adaptive noise floor during silence
        if !analysis.is_speech && self.config.adaptive_threshold {
            self.noise_floor = self.noise_floor * (1.0 - self.config.adaptation_rate)
                + analysis.energy * self.config.adaptation_rate;
        }

        // State machine transitions
        match self.state {
            VadState::Silence => {
                if analysis.is_speech {
                    self.speech_count = 1;
                    self.state = VadState::SpeechOnset;
                    self.stats.silence_frames += 1;
                    VadEvent::NoChange { is_speech: false }
                } else {
                    self.stats.silence_frames += 1;
                    VadEvent::NoChange { is_speech: false }
                }
            }
            VadState::SpeechOnset => {
                if analysis.is_speech {
                    self.speech_count += 1;
                    if self.speech_count >= self.config.speech_onset_frames {
                        // Confirmed speech start
                        self.state = VadState::Speech;
                        self.segment_frames = self.speech_count;
                        self.speech_start_time = Some(Utc::now());
                        self.stats.speech_frames += self.speech_count as u64;
                        debug!(
                            energy = analysis.energy,
                            zcr = analysis.zcr,
                            "VAD: speech started"
                        );
                        VadEvent::SpeechStart
                    } else {
                        self.stats.silence_frames += 1;
                        VadEvent::NoChange { is_speech: false }
                    }
                } else {
                    // False alarm — back to silence
                    self.speech_count = 0;
                    self.state = VadState::Silence;
                    self.stats.silence_frames += 1;
                    VadEvent::NoChange { is_speech: false }
                }
            }
            VadState::Speech => {
                self.segment_frames += 1;
                if analysis.is_speech {
                    self.silence_count = 0;
                    self.stats.speech_frames += 1;
                    VadEvent::NoChange { is_speech: true }
                } else {
                    // Start hangover
                    self.silence_count = 1;
                    self.state = VadState::SpeechHangover;
                    self.stats.speech_frames += 1; // count hangover as speech
                    VadEvent::NoChange { is_speech: true }
                }
            }
            VadState::SpeechHangover => {
                self.segment_frames += 1;
                if analysis.is_speech {
                    // Speech resumed — back to active
                    self.silence_count = 0;
                    self.state = VadState::Speech;
                    self.stats.speech_frames += 1;
                    VadEvent::NoChange { is_speech: true }
                } else {
                    self.silence_count += 1;
                    if self.silence_count >= self.config.hangover_frames {
                        // Speech ended
                        let duration = self.segment_frames;
                        self.state = VadState::Silence;
                        self.silence_count = 0;
                        self.speech_count = 0;
                        self.speech_start_time = None;
                        self.pre_speech_buffer.clear();

                        // Update stats
                        if duration >= self.config.min_speech_frames {
                            self.stats.speech_segments += 1;
                            self.stats.total_speech_duration_frames += duration as u64;
                            self.stats.shortest_segment_frames = Some(
                                self.stats
                                    .shortest_segment_frames
                                    .map_or(duration as u64, |s| s.min(duration as u64)),
                            );
                            self.stats.longest_segment_frames = Some(
                                self.stats
                                    .longest_segment_frames
                                    .map_or(duration as u64, |s| s.max(duration as u64)),
                            );
                        }

                        self.segment_frames = 0;
                        debug!(duration_frames = duration, "VAD: speech ended");
                        VadEvent::SpeechEnd {
                            duration_frames: duration,
                        }
                    } else {
                        self.stats.speech_frames += 1; // hangover still counts as speech
                        VadEvent::NoChange { is_speech: true }
                    }
                }
            }
        }
    }

    /// Analyze a single frame for energy and ZCR
    fn analyze_frame(&self, samples: &[f32]) -> FrameAnalysis {
        let energy = compute_rms(samples);
        let zcr = compute_zcr(samples);

        // Determine effective threshold
        let effective_threshold = if self.config.adaptive_threshold {
            (self.noise_floor * 3.0).max(self.config.energy_threshold)
        } else {
            self.config.energy_threshold
        };

        let is_speech = energy > effective_threshold && zcr < self.config.zcr_threshold;

        FrameAnalysis {
            energy,
            zcr,
            is_speech,
            noise_floor: self.noise_floor,
        }
    }

    /// Get current VAD state
    pub fn state(&self) -> VadState {
        self.state
    }

    /// Whether speech is currently active
    pub fn is_speaking(&self) -> bool {
        matches!(self.state, VadState::Speech | VadState::SpeechHangover)
    }

    /// Get statistics
    pub fn stats(&self) -> &VadStats {
        &self.stats
    }

    /// Reset state (not stats)
    pub fn reset(&mut self) {
        self.state = VadState::Silence;
        self.speech_count = 0;
        self.silence_count = 0;
        self.segment_frames = 0;
        self.pre_speech_buffer.clear();
        self.speech_start_time = None;
    }

    /// Reset everything including stats
    pub fn reset_all(&mut self) {
        self.reset();
        self.stats = VadStats::default();
        self.noise_floor = self.config.energy_threshold * 0.5;
    }

    /// Get speech ratio (0.0–1.0)
    pub fn speech_ratio(&self) -> f64 {
        if self.stats.total_frames == 0 {
            return 0.0;
        }
        self.stats.speech_frames as f64 / self.stats.total_frames as f64
    }

    /// Get current noise floor estimate
    pub fn noise_floor(&self) -> f64 {
        self.noise_floor
    }

    /// Get pre-speech buffer contents
    pub fn pre_speech_buffer(&self) -> &VecDeque<Vec<f32>> {
        &self.pre_speech_buffer
    }

    /// Get config
    pub fn config(&self) -> &VadConfig {
        &self.config
    }

    /// Update config
    pub fn set_config(&mut self, config: VadConfig) {
        self.config = config;
    }
}

impl Default for VadEngine {
    fn default() -> Self {
        Self::new(VadConfig::default())
    }
}

// ============================================================================
// Audio Analysis Utilities
// ============================================================================

/// Compute RMS (root mean square) energy of audio samples
fn compute_rms(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum_sq / samples.len() as f64).sqrt()
}

/// Compute zero-crossing rate (normalized 0.0–1.0)
fn compute_zcr(samples: &[f32]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let crossings = samples
        .windows(2)
        .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
        .count();
    crossings as f64 / (samples.len() - 1) as f64
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_engine() -> VadEngine {
        VadEngine::new(VadConfig {
            energy_threshold: 0.02,
            zcr_threshold: 0.5,
            adaptive_threshold: false, // deterministic for tests
            speech_onset_frames: 2,
            hangover_frames: 3,
            pre_speech_buffer_frames: 2,
            sample_rate: 16000,
            frame_size: 160,
            min_speech_frames: 2,
            ..Default::default()
        })
    }

    /// Generate a silent frame (all zeros)
    fn silence_frame(size: usize) -> Vec<f32> {
        vec![0.0; size]
    }

    /// Generate a speech-like frame (sine wave with amplitude)
    fn speech_frame(size: usize, amplitude: f32) -> Vec<f32> {
        (0..size)
            .map(|i| amplitude * (2.0 * std::f32::consts::PI * 200.0 * i as f32 / 16000.0).sin())
            .collect()
    }

    /// Generate a noisy frame (high ZCR, random-ish)
    fn noise_frame(size: usize) -> Vec<f32> {
        (0..size)
            .map(|i| if i % 2 == 0 { 0.05 } else { -0.05 })
            .collect()
    }

    #[test]
    fn test_initial_state_silence() {
        let engine = test_engine();
        assert_eq!(engine.state(), VadState::Silence);
        assert!(!engine.is_speaking());
    }

    #[test]
    fn test_silence_stays_silent() {
        let mut engine = test_engine();
        let event = engine.process_frame(&silence_frame(160));
        assert_eq!(event, VadEvent::NoChange { is_speech: false });
        assert_eq!(engine.state(), VadState::Silence);
    }

    #[test]
    fn test_speech_onset_detection() {
        let mut engine = test_engine();
        // First speech frame → onset state
        engine.process_frame(&speech_frame(160, 0.5));
        assert_eq!(engine.state(), VadState::SpeechOnset);

        // Second speech frame → confirmed speech (onset_frames = 2)
        let event = engine.process_frame(&speech_frame(160, 0.5));
        assert_eq!(event, VadEvent::SpeechStart);
        assert_eq!(engine.state(), VadState::Speech);
        assert!(engine.is_speaking());
    }

    #[test]
    fn test_false_alarm_returns_to_silence() {
        let mut engine = test_engine();
        // One speech frame
        engine.process_frame(&speech_frame(160, 0.5));
        assert_eq!(engine.state(), VadState::SpeechOnset);

        // Then silence → false alarm
        let event = engine.process_frame(&silence_frame(160));
        assert_eq!(event, VadEvent::NoChange { is_speech: false });
        assert_eq!(engine.state(), VadState::Silence);
    }

    #[test]
    fn test_speech_end_with_hangover() {
        let mut engine = test_engine();
        // Start speech (2 frames for onset)
        engine.process_frame(&speech_frame(160, 0.5));
        engine.process_frame(&speech_frame(160, 0.5));
        assert!(engine.is_speaking());

        // Add more speech frames to meet min_speech_frames
        engine.process_frame(&speech_frame(160, 0.5));

        // Now silence frames — hangover = 3
        engine.process_frame(&silence_frame(160));
        assert_eq!(engine.state(), VadState::SpeechHangover);
        engine.process_frame(&silence_frame(160));
        assert_eq!(engine.state(), VadState::SpeechHangover);

        // Third silence frame → speech end
        let event = engine.process_frame(&silence_frame(160));
        match event {
            VadEvent::SpeechEnd { duration_frames } => {
                assert!(duration_frames >= 5); // onset + speech + hangover
            }
            _ => panic!("Expected SpeechEnd, got {:?}", event),
        }
        assert_eq!(engine.state(), VadState::Silence);
    }

    #[test]
    fn test_hangover_recovery() {
        let mut engine = test_engine();
        // Start speech
        engine.process_frame(&speech_frame(160, 0.5));
        engine.process_frame(&speech_frame(160, 0.5));
        assert!(engine.is_speaking());

        // Brief silence (< hangover)
        engine.process_frame(&silence_frame(160));
        assert_eq!(engine.state(), VadState::SpeechHangover);

        // Speech resumes
        let event = engine.process_frame(&speech_frame(160, 0.5));
        assert_eq!(event, VadEvent::NoChange { is_speech: true });
        assert_eq!(engine.state(), VadState::Speech);
    }

    #[test]
    fn test_stats_tracking() {
        let mut engine = test_engine();
        // 2 speech onset + 1 more speech + 3 hangover silence = 6 frames total
        engine.process_frame(&speech_frame(160, 0.5));
        engine.process_frame(&speech_frame(160, 0.5));
        engine.process_frame(&speech_frame(160, 0.5));
        engine.process_frame(&silence_frame(160));
        engine.process_frame(&silence_frame(160));
        engine.process_frame(&silence_frame(160));

        let stats = engine.stats();
        assert_eq!(stats.total_frames, 6);
        assert!(stats.speech_frames > 0);
        assert_eq!(stats.speech_segments, 1);
    }

    #[test]
    fn test_speech_ratio() {
        let mut engine = test_engine();
        assert!((engine.speech_ratio() - 0.0).abs() < f64::EPSILON);

        // All speech
        for _ in 0..10 {
            engine.process_frame(&speech_frame(160, 0.5));
        }
        assert!(engine.speech_ratio() > 0.0);
    }

    #[test]
    fn test_reset_state() {
        let mut engine = test_engine();
        engine.process_frame(&speech_frame(160, 0.5));
        engine.process_frame(&speech_frame(160, 0.5));
        assert!(engine.is_speaking());

        engine.reset();
        assert_eq!(engine.state(), VadState::Silence);
        assert!(!engine.is_speaking());
        // Stats preserved after reset
        assert!(engine.stats().total_frames > 0);
    }

    #[test]
    fn test_reset_all() {
        let mut engine = test_engine();
        engine.process_frame(&speech_frame(160, 0.5));
        engine.reset_all();
        assert_eq!(engine.state(), VadState::Silence);
        assert_eq!(engine.stats().total_frames, 0);
    }

    #[test]
    fn test_compute_rms() {
        assert!((compute_rms(&[]) - 0.0).abs() < f64::EPSILON);
        assert!((compute_rms(&[0.0, 0.0]) - 0.0).abs() < f64::EPSILON);

        let rms = compute_rms(&[1.0, -1.0, 1.0, -1.0]);
        assert!((rms - 1.0).abs() < 0.01);

        let rms2 = compute_rms(&[0.5, -0.5]);
        assert!((rms2 - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_compute_zcr() {
        assert!((compute_zcr(&[]) - 0.0).abs() < f64::EPSILON);
        assert!((compute_zcr(&[1.0]) - 0.0).abs() < f64::EPSILON);

        // Alternating signs → max ZCR
        let zcr = compute_zcr(&[1.0, -1.0, 1.0, -1.0]);
        assert!((zcr - 1.0).abs() < f64::EPSILON);

        // No crossings
        let zcr2 = compute_zcr(&[1.0, 2.0, 3.0, 4.0]);
        assert!((zcr2 - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_noise_rejected_by_zcr() {
        let mut engine = test_engine();
        // Noise frame has high ZCR → should not be detected as speech
        let event = engine.process_frame(&noise_frame(160));
        assert_eq!(event, VadEvent::NoChange { is_speech: false });
    }

    #[test]
    fn test_pre_speech_buffer() {
        let mut engine = test_engine();
        // Feed some silence frames
        engine.process_frame(&silence_frame(160));
        engine.process_frame(&silence_frame(160));
        engine.process_frame(&silence_frame(160));

        // Pre-speech buffer should hold last 2 (config)
        assert_eq!(engine.pre_speech_buffer().len(), 2);
    }

    #[test]
    fn test_adaptive_noise_floor() {
        let mut engine = VadEngine::new(VadConfig {
            adaptive_threshold: true,
            adaptation_rate: 0.5, // fast adaptation for test
            energy_threshold: 0.02,
            speech_onset_frames: 2,
            hangover_frames: 3,
            zcr_threshold: 0.5,
            ..Default::default()
        });

        let initial_floor = engine.noise_floor();

        // Feed low-energy silence — noise floor should decrease
        for _ in 0..5 {
            engine.process_frame(&silence_frame(160));
        }
        assert!(engine.noise_floor() <= initial_floor);
    }

    #[test]
    fn test_multiple_speech_segments() {
        let mut engine = test_engine();

        // First segment
        engine.process_frame(&speech_frame(160, 0.5));
        engine.process_frame(&speech_frame(160, 0.5));
        engine.process_frame(&speech_frame(160, 0.5));
        for _ in 0..3 {
            engine.process_frame(&silence_frame(160));
        }

        // Second segment
        engine.process_frame(&speech_frame(160, 0.5));
        engine.process_frame(&speech_frame(160, 0.5));
        engine.process_frame(&speech_frame(160, 0.5));
        for _ in 0..3 {
            engine.process_frame(&silence_frame(160));
        }

        assert_eq!(engine.stats().speech_segments, 2);
    }

    #[test]
    fn test_state_display() {
        assert_eq!(format!("{}", VadState::Silence), "silence");
        assert_eq!(format!("{}", VadState::SpeechOnset), "speech-onset");
        assert_eq!(format!("{}", VadState::Speech), "speech");
        assert_eq!(format!("{}", VadState::SpeechHangover), "speech-hangover");
    }

    #[test]
    fn test_config_update() {
        let mut engine = test_engine();
        engine.set_config(VadConfig {
            energy_threshold: 0.1,
            ..Default::default()
        });
        assert!((engine.config().energy_threshold - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn test_default_engine() {
        let engine = VadEngine::default();
        assert_eq!(engine.state(), VadState::Silence);
        assert_eq!(engine.config().sample_rate, 16000);
        assert_eq!(engine.config().frame_size, 512);
    }

    #[test]
    fn test_segment_duration_stats() {
        let mut engine = test_engine();

        // Short segment: onset(2) + 1 speech + hangover(3) = 6 frames
        engine.process_frame(&speech_frame(160, 0.5));
        engine.process_frame(&speech_frame(160, 0.5));
        engine.process_frame(&speech_frame(160, 0.5));
        for _ in 0..3 {
            engine.process_frame(&silence_frame(160));
        }

        // Long segment: onset(2) + 5 speech + hangover(3) = 10 frames
        engine.process_frame(&speech_frame(160, 0.5));
        engine.process_frame(&speech_frame(160, 0.5));
        for _ in 0..5 {
            engine.process_frame(&speech_frame(160, 0.5));
        }
        for _ in 0..3 {
            engine.process_frame(&silence_frame(160));
        }

        let stats = engine.stats();
        assert_eq!(stats.speech_segments, 2);
        assert!(stats.shortest_segment_frames.is_some());
        assert!(stats.longest_segment_frames.is_some());
        assert!(stats.longest_segment_frames.unwrap() >= stats.shortest_segment_frames.unwrap());
    }
}
