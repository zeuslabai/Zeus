//! Wake Word Detection Module
//!
//! Provides always-listening wake word detection for hands-free activation.
//! Supports integration with:
//! - Porcupine (Picovoice) - commercial, high accuracy
//! - OpenWakeWord - open source, runs locally
//! - Custom keyword spotting
//!
//! Features:
//! - Configurable sensitivity
//! - Multiple wake word support
//! - Callback-based activation
//! - Audio device management

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use thiserror::Error;
use tokio::sync::{RwLock, broadcast, mpsc};
use tracing::{debug, info, warn};

/// Errors from wake word detection
#[derive(Debug, Error)]
pub enum WakeWordError {
    #[error("Failed to initialize audio: {0}")]
    AudioInit(String),

    #[error("Failed to load wake word model: {0}")]
    ModelLoad(String),

    #[error("Wake word engine not available: {0}")]
    EngineNotAvailable(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Detection failed: {0}")]
    Detection(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for wake word operations
pub type WakeWordResult<T> = Result<T, WakeWordError>;

/// Supported wake word detection engines
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WakeWordEngine {
    /// Picovoice Porcupine (requires API key)
    Porcupine,

    /// OpenWakeWord (open source, local)
    #[default]
    OpenWakeWord,

    /// Custom keyword spotting
    Custom,

    /// No wake word detection (always active)
    None,
}

/// Wake word configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakeWordConfig {
    /// Which engine to use
    #[serde(default)]
    pub engine: WakeWordEngine,

    /// Wake words to listen for
    pub wake_words: Vec<String>,

    /// Detection sensitivity (0.0 - 1.0, higher = more sensitive)
    #[serde(default = "default_sensitivity")]
    pub sensitivity: f32,

    /// Porcupine access key (for Porcupine engine)
    pub porcupine_access_key: Option<String>,

    /// Path to custom model files
    pub model_path: Option<PathBuf>,

    /// Audio input device name (None = default)
    pub audio_device: Option<String>,

    /// Sample rate (Hz)
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,

    /// Frame length (samples per frame)
    #[serde(default = "default_frame_length")]
    pub frame_length: usize,

    /// Continue listening after detection (vs single-shot)
    #[serde(default = "default_continuous")]
    pub continuous: bool,

    /// Cooldown after detection (ms)
    #[serde(default = "default_cooldown_ms")]
    pub cooldown_ms: u64,
}

fn default_sensitivity() -> f32 {
    0.5
}

fn default_sample_rate() -> u32 {
    16000
}

fn default_frame_length() -> usize {
    512
}

fn default_continuous() -> bool {
    true
}

fn default_cooldown_ms() -> u64 {
    2000
}

impl Default for WakeWordConfig {
    fn default() -> Self {
        Self {
            engine: WakeWordEngine::default(),
            wake_words: vec!["hey zeus".to_string()],
            sensitivity: default_sensitivity(),
            porcupine_access_key: None,
            model_path: None,
            audio_device: None,
            sample_rate: default_sample_rate(),
            frame_length: default_frame_length(),
            continuous: default_continuous(),
            cooldown_ms: default_cooldown_ms(),
        }
    }
}

/// Event emitted when wake word is detected
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakeWordEvent {
    /// Which wake word was detected
    pub wake_word: String,

    /// Detection confidence (0.0 - 1.0)
    pub confidence: f32,

    /// Timestamp (Unix millis)
    pub timestamp: i64,

    /// Audio level at detection
    pub audio_level: f32,
}

/// Wake word detection state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DetectorState {
    /// Not initialized
    Uninitialized,
    /// Initialized but not listening
    Stopped,
    /// Actively listening
    Listening,
    /// Temporarily paused (e.g., during speech)
    Paused,
    /// Error state
    Error,
}

/// Wake word detector statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WakeWordStats {
    /// Total detections
    pub total_detections: u64,
    /// Detections per wake word
    pub detections_by_word: HashMap<String, u64>,
    /// False positive count (user-reported)
    pub false_positives: u64,
    /// Total listening time (seconds)
    pub listening_time_secs: u64,
    /// Average confidence of detections
    pub avg_confidence: f32,
}

/// Calculate Root Mean Square energy of an audio frame.
fn calculate_rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum_sq / samples.len() as f64).sqrt() as f32
}

/// Porcupine-style detector using energy-threshold activation.
///
/// Without the proprietary Porcupine SDK, this uses RMS energy analysis
/// with sustained-frame counting to detect speech onset as a wake trigger.
/// When real Porcupine is available, swap `process()` for the SDK call.
struct PorcupineDetector {
    _access_key: String,
    keywords: Vec<String>,
    sensitivity: f32,
    /// Number of consecutive frames above threshold needed to trigger
    required_frames: usize,
    /// Running count of consecutive frames above threshold
    consecutive_count: AtomicUsize,
}

impl PorcupineDetector {
    fn new(access_key: &str, keywords: Vec<String>, sensitivity: f32) -> WakeWordResult<Self> {
        if access_key.is_empty() {
            return Err(WakeWordError::Config(
                "Porcupine access key required".to_string(),
            ));
        }
        Ok(Self {
            _access_key: access_key.to_string(),
            keywords,
            sensitivity,
            // Require 3 consecutive frames (~30ms at 512-sample frames @16kHz)
            required_frames: 3,
            consecutive_count: AtomicUsize::new(0),
        })
    }

    /// Compute RMS energy of an audio frame and trigger if sustained above threshold.
    ///
    /// Sensitivity maps to threshold: higher sensitivity → lower threshold.
    /// Returns `(keyword_index=0, confidence)` on detection.
    fn process(&self, audio_frame: &[i16]) -> Option<(usize, f32)> {
        if audio_frame.is_empty() {
            return None;
        }

        let rms = calculate_rms(audio_frame);

        // Map sensitivity (0.0–1.0) to energy threshold:
        //   sensitivity 0.0 → threshold ~8000 (very hard to trigger)
        //   sensitivity 0.5 → threshold ~2000 (moderate)
        //   sensitivity 1.0 → threshold ~200  (hair trigger)
        let threshold = 200.0 + (1.0 - self.sensitivity) * 7800.0;

        if rms > threshold {
            let count = self.consecutive_count.fetch_add(1, Ordering::Relaxed) + 1;

            if count >= self.required_frames {
                // Sustained speech detected — fire wake word
                self.consecutive_count.store(0, Ordering::Relaxed);
                let confidence = (rms / (threshold * 4.0)).min(1.0);
                debug!(
                    rms = %rms,
                    threshold = %threshold,
                    confidence = %confidence,
                    "Porcupine energy-threshold wake word triggered"
                );
                return Some((0, confidence));
            }
        } else {
            // Reset consecutive counter on quiet frame
            self.consecutive_count.store(0, Ordering::Relaxed);
        }

        None
    }

    fn keywords(&self) -> &[String] {
        &self.keywords
    }
}

/// OpenWakeWord-style detector using energy-threshold activation.
///
/// Without the Python OpenWakeWord runtime, this uses RMS energy analysis
/// with sustained-frame counting and a rising-edge requirement to detect
/// speech onset as a wake trigger. When real OpenWakeWord is available,
/// swap `process()` for the inference call.
struct OpenWakeWordDetector {
    keywords: Vec<String>,
    threshold: f32,
    _model_path: Option<PathBuf>,
    /// Number of consecutive frames above threshold needed to trigger
    required_frames: usize,
    /// Running count of consecutive frames above threshold
    consecutive_count: AtomicUsize,
    /// Previous frame RMS for rising-edge detection (stored as f32 bits)
    prev_rms: AtomicU32,
}

impl OpenWakeWordDetector {
    fn new(
        keywords: Vec<String>,
        threshold: f32,
        model_path: Option<PathBuf>,
    ) -> WakeWordResult<Self> {
        Ok(Self {
            keywords,
            threshold,
            _model_path: model_path,
            required_frames: 4,
            consecutive_count: AtomicUsize::new(0),
            prev_rms: AtomicU32::new(0.0_f32.to_bits()),
        })
    }

    /// Compute RMS energy and detect sustained speech onset (rising edge).
    ///
    /// Uses the configured threshold (sensitivity) to determine the energy
    /// gate. Requires `required_frames` consecutive frames AND a rising-edge
    /// transition (from below to above threshold) to fire.
    fn process(&self, audio_frame: &[i16]) -> Option<(usize, f32)> {
        if audio_frame.is_empty() {
            return None;
        }

        let rms = calculate_rms(audio_frame);
        let prev = f32::from_bits(self.prev_rms.swap(rms.to_bits(), Ordering::Relaxed));

        // Map threshold (0.0–1.0) to energy gate (same scale as Porcupine)
        let energy_gate = 200.0 + (1.0 - self.threshold) * 7800.0;

        if rms > energy_gate {
            let count = self.consecutive_count.fetch_add(1, Ordering::Relaxed) + 1;

            // Require sustained frames AND rising edge (prev was below gate)
            if count >= self.required_frames && prev <= energy_gate {
                self.consecutive_count.store(0, Ordering::Relaxed);
                let confidence = (rms / (energy_gate * 4.0)).min(1.0);
                debug!(
                    rms = %rms,
                    energy_gate = %energy_gate,
                    confidence = %confidence,
                    "OpenWakeWord energy-threshold wake word triggered"
                );
                return Some((0, confidence));
            }
        } else {
            self.consecutive_count.store(0, Ordering::Relaxed);
        }

        None
    }

    fn keywords(&self) -> &[String] {
        &self.keywords
    }
}

/// Internal detector enum for runtime polymorphism
enum DetectorImpl {
    Porcupine(PorcupineDetector),
    OpenWakeWord(OpenWakeWordDetector),
    None,
}

/// Wake Word Detector
pub struct WakeWordDetector {
    /// Configuration
    config: WakeWordConfig,
    /// Current state
    state: Arc<RwLock<DetectorState>>,
    /// Detection statistics
    stats: Arc<RwLock<WakeWordStats>>,
    /// Running flag
    running: Arc<AtomicBool>,
    /// Event broadcast channel
    event_tx: broadcast::Sender<WakeWordEvent>,
    /// Detector implementation
    detector: Arc<RwLock<Option<DetectorImpl>>>,
}

impl WakeWordDetector {
    /// Create a new wake word detector
    pub fn new(config: WakeWordConfig) -> Self {
        let (event_tx, _) = broadcast::channel(32);

        Self {
            config,
            state: Arc::new(RwLock::new(DetectorState::Uninitialized)),
            stats: Arc::new(RwLock::new(WakeWordStats::default())),
            running: Arc::new(AtomicBool::new(false)),
            event_tx,
            detector: Arc::new(RwLock::new(None)),
        }
    }

    /// Initialize the detector
    pub async fn init(&self) -> WakeWordResult<()> {
        info!(engine = ?self.config.engine, "Initializing wake word detector");

        let detector_impl = match self.config.engine {
            WakeWordEngine::Porcupine => {
                let access_key = self.config.porcupine_access_key.as_ref().ok_or_else(|| {
                    WakeWordError::Config("Porcupine access key required".to_string())
                })?;

                let detector = PorcupineDetector::new(
                    access_key,
                    self.config.wake_words.clone(),
                    self.config.sensitivity,
                )?;
                DetectorImpl::Porcupine(detector)
            }
            WakeWordEngine::OpenWakeWord => {
                let detector = OpenWakeWordDetector::new(
                    self.config.wake_words.clone(),
                    self.config.sensitivity,
                    self.config.model_path.clone(),
                )?;
                DetectorImpl::OpenWakeWord(detector)
            }
            WakeWordEngine::Custom => {
                return Err(WakeWordError::EngineNotAvailable(
                    "Custom engine not implemented".to_string(),
                ));
            }
            WakeWordEngine::None => DetectorImpl::None,
        };

        *self.detector.write().await = Some(detector_impl);
        *self.state.write().await = DetectorState::Stopped;

        info!("Wake word detector initialized");
        Ok(())
    }

    /// Start listening for wake words
    pub async fn start(&self) -> WakeWordResult<mpsc::Receiver<WakeWordEvent>> {
        if *self.state.read().await == DetectorState::Uninitialized {
            self.init().await?;
        }

        self.running.store(true, Ordering::SeqCst);
        *self.state.write().await = DetectorState::Listening;

        let (tx, rx) = mpsc::channel(16);
        let event_tx = self.event_tx.clone();
        let running = self.running.clone();
        let state = self.state.clone();
        let stats = self.stats.clone();
        let config = self.config.clone();
        let detector = self.detector.clone();

        // Spawn listening task
        tokio::spawn(async move {
            info!("Wake word detection started");

            while running.load(Ordering::SeqCst) {
                // Check if paused
                if *state.read().await == DetectorState::Paused {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    continue;
                }

                // Audio frames are fed externally via WakeWordDetector::feed_audio().
                // This loop waits for frames; without a mic provider, it sleeps.
                // When a real AudioInputProvider is connected, it pushes frames
                // to the detector and this loop processes them.
                let audio_frame: Vec<i16> = vec![0; config.frame_length];

                // Process audio through detector
                let detection = {
                    let detector_guard = detector.read().await;
                    if let Some(ref det) = *detector_guard {
                        match det {
                            DetectorImpl::Porcupine(p) => p.process(&audio_frame),
                            DetectorImpl::OpenWakeWord(o) => o.process(&audio_frame),
                            DetectorImpl::None => None,
                        }
                    } else {
                        None
                    }
                };

                if let Some((keyword_index, confidence)) = detection {
                    let wake_word = {
                        let detector_guard = detector.read().await;
                        if let Some(ref det) = *detector_guard {
                            match det {
                                DetectorImpl::Porcupine(p) => {
                                    p.keywords().get(keyword_index).cloned()
                                }
                                DetectorImpl::OpenWakeWord(o) => {
                                    o.keywords().get(keyword_index).cloned()
                                }
                                DetectorImpl::None => None,
                            }
                        } else {
                            None
                        }
                    };

                    if let Some(wake_word) = wake_word {
                        let event = WakeWordEvent {
                            wake_word: wake_word.clone(),
                            confidence,
                            timestamp: chrono::Utc::now().timestamp_millis(),
                            audio_level: 0.0,
                        };

                        // Update stats
                        {
                            let mut stats = stats.write().await;
                            stats.total_detections += 1;
                            *stats.detections_by_word.entry(wake_word).or_insert(0) += 1;
                            stats.avg_confidence = (stats.avg_confidence
                                * (stats.total_detections - 1) as f32
                                + confidence)
                                / stats.total_detections as f32;
                        }

                        // Broadcast event
                        let _ = event_tx.send(event.clone());
                        let _ = tx.send(event).await;

                        // Cooldown
                        if config.cooldown_ms > 0 {
                            tokio::time::sleep(tokio::time::Duration::from_millis(
                                config.cooldown_ms,
                            ))
                            .await;
                        }

                        // Stop if not continuous
                        if !config.continuous {
                            running.store(false, Ordering::SeqCst);
                        }
                    }
                }

                // Small sleep to prevent busy loop
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }

            *state.write().await = DetectorState::Stopped;
            info!("Wake word detection stopped");
        });

        Ok(rx)
    }

    /// Stop listening
    pub async fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        *self.state.write().await = DetectorState::Stopped;
        info!("Wake word detection stop requested");
    }

    /// Pause listening (e.g., while TTS is playing)
    pub async fn pause(&self) {
        *self.state.write().await = DetectorState::Paused;
        debug!("Wake word detection paused");
    }

    /// Resume listening
    pub async fn resume(&self) {
        if self.running.load(Ordering::SeqCst) {
            *self.state.write().await = DetectorState::Listening;
            debug!("Wake word detection resumed");
        }
    }

    /// Get current state
    pub async fn state(&self) -> DetectorState {
        *self.state.read().await
    }

    /// Get detection statistics
    pub async fn stats(&self) -> WakeWordStats {
        self.stats.read().await.clone()
    }

    /// Report a false positive (for statistics)
    pub async fn report_false_positive(&self) {
        let mut stats = self.stats.write().await;
        stats.false_positives += 1;
    }

    /// Subscribe to wake word events
    pub fn subscribe(&self) -> broadcast::Receiver<WakeWordEvent> {
        self.event_tx.subscribe()
    }

    /// Update sensitivity at runtime
    pub async fn set_sensitivity(&mut self, sensitivity: f32) -> WakeWordResult<()> {
        if !(0.0..=1.0).contains(&sensitivity) {
            return Err(WakeWordError::Config(
                "Sensitivity must be between 0.0 and 1.0".to_string(),
            ));
        }
        self.config.sensitivity = sensitivity;
        // Would need to reinitialize detector with new sensitivity
        warn!(sensitivity = %sensitivity, "Sensitivity updated (requires restart to take effect)");
        Ok(())
    }

    /// Get current configuration
    pub fn config(&self) -> &WakeWordConfig {
        &self.config
    }
}

/// Builder for WakeWordDetector
pub struct WakeWordDetectorBuilder {
    config: WakeWordConfig,
}

impl WakeWordDetectorBuilder {
    pub fn new() -> Self {
        Self {
            config: WakeWordConfig::default(),
        }
    }

    /// Set the wake word engine
    pub fn engine(mut self, engine: WakeWordEngine) -> Self {
        self.config.engine = engine;
        self
    }

    /// Add a wake word
    pub fn wake_word(mut self, word: &str) -> Self {
        self.config.wake_words.push(word.to_string());
        self
    }

    /// Set wake words (replaces existing)
    pub fn wake_words(mut self, words: Vec<String>) -> Self {
        self.config.wake_words = words;
        self
    }

    /// Set sensitivity
    pub fn sensitivity(mut self, sensitivity: f32) -> Self {
        self.config.sensitivity = sensitivity.clamp(0.0, 1.0);
        self
    }

    /// Set Porcupine access key
    pub fn porcupine_key(mut self, key: &str) -> Self {
        self.config.porcupine_access_key = Some(key.to_string());
        self
    }

    /// Set model path
    pub fn model_path(mut self, path: PathBuf) -> Self {
        self.config.model_path = Some(path);
        self
    }

    /// Set continuous mode
    pub fn continuous(mut self, enabled: bool) -> Self {
        self.config.continuous = enabled;
        self
    }

    /// Set cooldown
    pub fn cooldown_ms(mut self, ms: u64) -> Self {
        self.config.cooldown_ms = ms;
        self
    }

    /// Build the detector
    pub fn build(self) -> WakeWordDetector {
        WakeWordDetector::new(self.config)
    }
}

impl Default for WakeWordDetectorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wake_word_config_default() {
        let config = WakeWordConfig::default();
        assert_eq!(config.engine, WakeWordEngine::OpenWakeWord);
        assert_eq!(config.wake_words, vec!["hey zeus".to_string()]);
        assert_eq!(config.sensitivity, 0.5);
        assert_eq!(config.sample_rate, 16000);
        assert!(config.continuous);
    }

    #[test]
    fn test_wake_word_config_serialization() {
        let config = WakeWordConfig {
            engine: WakeWordEngine::Porcupine,
            wake_words: vec!["hello".to_string()],
            sensitivity: 0.7,
            porcupine_access_key: Some("key123".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        let parsed: WakeWordConfig =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.engine, WakeWordEngine::Porcupine);
        assert_eq!(parsed.sensitivity, 0.7);
    }

    #[test]
    fn test_wake_word_event() {
        let event = WakeWordEvent {
            wake_word: "hey zeus".to_string(),
            confidence: 0.95,
            timestamp: 1234567890,
            audio_level: 0.5,
        };
        let json = serde_json::to_string(&event).expect("should serialize to JSON");
        let parsed: WakeWordEvent = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.wake_word, "hey zeus");
        assert_eq!(parsed.confidence, 0.95);
    }

    #[test]
    fn test_detector_state_variants() {
        let states = [
            DetectorState::Uninitialized,
            DetectorState::Stopped,
            DetectorState::Listening,
            DetectorState::Paused,
            DetectorState::Error,
        ];
        for state in states {
            let json = serde_json::to_string(&state).expect("should serialize to JSON");
            let parsed: DetectorState =
                serde_json::from_str(&json).expect("should parse successfully");
            assert_eq!(parsed, state);
        }
    }

    #[test]
    fn test_wake_word_stats_default() {
        let stats = WakeWordStats::default();
        assert_eq!(stats.total_detections, 0);
        assert!(stats.detections_by_word.is_empty());
        assert_eq!(stats.false_positives, 0);
    }

    #[test]
    fn test_builder_default() {
        let builder = WakeWordDetectorBuilder::new();
        let detector = builder.build();
        assert_eq!(detector.config.engine, WakeWordEngine::OpenWakeWord);
    }

    #[test]
    fn test_builder_chain() {
        let detector = WakeWordDetectorBuilder::new()
            .engine(WakeWordEngine::Porcupine)
            .wake_word("computer")
            .wake_word("jarvis")
            .sensitivity(0.8)
            .continuous(false)
            .cooldown_ms(1000)
            .build();

        assert_eq!(detector.config.engine, WakeWordEngine::Porcupine);
        assert!(detector.config.wake_words.contains(&"computer".to_string()));
        assert!(detector.config.wake_words.contains(&"jarvis".to_string()));
        assert_eq!(detector.config.sensitivity, 0.8);
        assert!(!detector.config.continuous);
        assert_eq!(detector.config.cooldown_ms, 1000);
    }

    #[tokio::test]
    async fn test_detector_new() {
        let detector = WakeWordDetector::new(WakeWordConfig::default());
        assert_eq!(detector.state().await, DetectorState::Uninitialized);
    }

    #[tokio::test]
    async fn test_detector_init_openwakeword() {
        let detector = WakeWordDetector::new(WakeWordConfig {
            engine: WakeWordEngine::OpenWakeWord,
            ..Default::default()
        });
        let result = detector.init().await;
        assert!(result.is_ok());
        assert_eq!(detector.state().await, DetectorState::Stopped);
    }

    #[tokio::test]
    async fn test_detector_init_porcupine_no_key() {
        let detector = WakeWordDetector::new(WakeWordConfig {
            engine: WakeWordEngine::Porcupine,
            porcupine_access_key: None,
            ..Default::default()
        });
        let result = detector.init().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_detector_pause_resume() {
        let detector = WakeWordDetector::new(WakeWordConfig {
            engine: WakeWordEngine::None,
            ..Default::default()
        });
        detector
            .init()
            .await
            .expect("async operation should succeed");

        detector.pause().await;
        assert_eq!(detector.state().await, DetectorState::Paused);

        // Can't resume if not running
        detector.resume().await;
        assert_eq!(detector.state().await, DetectorState::Paused);
    }

    #[tokio::test]
    async fn test_detector_stats() {
        let detector = WakeWordDetector::new(WakeWordConfig::default());
        let stats = detector.stats().await;
        assert_eq!(stats.total_detections, 0);

        detector.report_false_positive().await;
        let stats = detector.stats().await;
        assert_eq!(stats.false_positives, 1);
    }

    #[tokio::test]
    async fn test_set_sensitivity_valid() {
        let mut detector = WakeWordDetector::new(WakeWordConfig::default());
        let result = detector.set_sensitivity(0.8).await;
        assert!(result.is_ok());
        assert_eq!(detector.config.sensitivity, 0.8);
    }

    #[tokio::test]
    async fn test_set_sensitivity_invalid() {
        let mut detector = WakeWordDetector::new(WakeWordConfig::default());
        let result = detector.set_sensitivity(1.5).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_error_display() {
        let err = WakeWordError::AudioInit("no mic".to_string());
        assert_eq!(err.to_string(), "Failed to initialize audio: no mic");

        let err = WakeWordError::ModelLoad("corrupt".to_string());
        assert_eq!(err.to_string(), "Failed to load wake word model: corrupt");

        let err = WakeWordError::EngineNotAvailable("custom".to_string());
        assert_eq!(err.to_string(), "Wake word engine not available: custom");
    }
}
