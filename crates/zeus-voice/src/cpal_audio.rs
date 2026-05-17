//! Cross-platform audio I/O using cpal
//!
//! Provides concrete implementations of [`AudioInputProvider`] and [`AudioOutputProvider`]
//! for local microphone capture and speaker playback using the cpal crate.
//!
//! - [`CpalAudioInput`] opens the default input device, captures PCM 16-bit 16kHz mono
//!   frames, and sends them over an mpsc channel.
//! - [`CpalAudioOutput`] decodes WAV data and plays it through the default output device.
//!
//! Because `cpal::Stream` is `!Send + !Sync`, all audio streams run on dedicated OS
//! threads and communicate via channels.

use async_trait::async_trait;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use crate::audio::{parse_wav_pcm, resample, stereo_to_mono};
use crate::talk_mode::{AudioInputProvider, AudioOutputProvider};

/// PCM frame size (samples per chunk sent through the channel)
const FRAME_SIZE: usize = 512;

/// Target sample rate for captured audio (16kHz for STT)
const TARGET_SAMPLE_RATE: u32 = 16_000;

// ============================================================================
// CpalAudioInput
// ============================================================================

/// Microphone capture using cpal.
///
/// Opens the default input device and streams PCM 16-bit 16kHz mono frames.
/// Each frame is [`FRAME_SIZE`] samples (512 = 32ms at 16kHz).
///
/// The cpal stream runs on a dedicated OS thread since `cpal::Stream` is `!Send`.
pub struct CpalAudioInput {
    capturing: Arc<AtomicBool>,
    /// Join handle for the capture thread
    thread_handle: tokio::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl CpalAudioInput {
    pub fn new() -> Self {
        Self {
            capturing: Arc::new(AtomicBool::new(false)),
            thread_handle: tokio::sync::Mutex::new(None),
        }
    }

    /// Check if a default input device exists.
    fn has_input_device() -> bool {
        let host = cpal::default_host();
        host.default_input_device().is_some()
    }
}

impl Default for CpalAudioInput {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AudioInputProvider for CpalAudioInput {
    async fn start_capture(&self) -> zeus_core::Result<mpsc::Receiver<Vec<i16>>> {
        if self.capturing.load(Ordering::SeqCst) {
            return Err(zeus_core::Error::Internal(
                "Already capturing audio".to_string(),
            ));
        }

        let (tx, rx) = mpsc::channel::<Vec<i16>>(64);
        let capturing = self.capturing.clone();
        capturing.store(true, Ordering::SeqCst);

        // Spawn a dedicated OS thread for the cpal stream (Stream is !Send)
        let cap_flag = capturing.clone();
        let handle = std::thread::Builder::new()
            .name("zeus-audio-input".to_string())
            .spawn(move || {
                if let Err(e) = run_capture_thread(tx, cap_flag.clone()) {
                    error!("Audio capture thread error: {}", e);
                    cap_flag.store(false, Ordering::SeqCst);
                }
            })
            .map_err(|e| {
                capturing.store(false, Ordering::SeqCst);
                zeus_core::Error::Internal(format!("Failed to spawn capture thread: {}", e))
            })?;

        *self.thread_handle.lock().await = Some(handle);
        info!("Audio capture started");

        Ok(rx)
    }

    async fn stop_capture(&self) {
        if self.capturing.swap(false, Ordering::SeqCst) {
            // Signal the thread to stop, then wait for it
            let handle = self.thread_handle.lock().await.take();
            if let Some(h) = handle {
                let _ = h.join();
            }
            info!("Audio capture stopped");
        }
    }

    fn is_available(&self) -> bool {
        Self::has_input_device()
    }
}

/// Run the capture loop on a dedicated thread.
///
/// Opens the default input device, builds a stream, and forwards PCM frames
/// through the channel until `capturing` is set to false.
fn run_capture_thread(
    tx: mpsc::Sender<Vec<i16>>,
    capturing: Arc<AtomicBool>,
) -> Result<(), String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or("No default audio input device found")?;

    let device_name = device.name().unwrap_or_else(|_| "unknown".to_string());
    debug!(device = %device_name, "Opening audio input device");

    let supported = device
        .supported_input_configs()
        .map_err(|e| format!("Failed to query input configs: {}", e))?;

    let config = find_best_input_config(supported, TARGET_SAMPLE_RATE)?;
    let device_sample_rate = config.sample_rate().0;
    let device_channels = config.channels() as u32;
    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();

    debug!(
        sample_rate = device_sample_rate,
        channels = device_channels,
        format = ?sample_format,
        "Selected input config"
    );

    let params = InputStreamParams {
        needs_resample: device_sample_rate != TARGET_SAMPLE_RATE,
        needs_mono_mix: device_channels > 1,
        device_rate: device_sample_rate,
        device_channels,
    };

    // Build the stream based on sample format
    let stream = match sample_format {
        cpal::SampleFormat::I16 => {
            build_input_stream_i16(&device, &stream_config, tx, capturing.clone(), &params)?
        }
        cpal::SampleFormat::F32 => {
            build_input_stream_f32(&device, &stream_config, tx, capturing.clone(), &params)?
        }
        other => {
            return Err(format!("Unsupported sample format: {:?}", other));
        }
    };

    stream
        .play()
        .map_err(|e| format!("Failed to start audio capture: {}", e))?;

    // Block this thread until capturing is stopped
    while capturing.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Stream is dropped here, stopping capture
    drop(stream);
    Ok(())
}

/// Find the best input config, preferring target sample rate and mono.
fn find_best_input_config(
    configs: cpal::SupportedInputConfigs,
    target_rate: u32,
) -> Result<cpal::SupportedStreamConfig, String> {
    let configs: Vec<_> = configs.collect();

    if configs.is_empty() {
        return Err("No supported input configurations".to_string());
    }

    // Try to find exact match: target rate mono i16
    for cfg in &configs {
        if cfg.channels() == 1
            && cfg.min_sample_rate().0 <= target_rate
            && cfg.max_sample_rate().0 >= target_rate
            && cfg.sample_format() == cpal::SampleFormat::I16
        {
            return Ok(cfg.with_sample_rate(cpal::SampleRate(target_rate)));
        }
    }

    // Try any config that supports target rate with i16
    for cfg in &configs {
        if cfg.min_sample_rate().0 <= target_rate
            && cfg.max_sample_rate().0 >= target_rate
            && cfg.sample_format() == cpal::SampleFormat::I16
        {
            return Ok(cfg.with_sample_rate(cpal::SampleRate(target_rate)));
        }
    }

    // Try any config that supports target rate with f32
    for cfg in &configs {
        if cfg.min_sample_rate().0 <= target_rate
            && cfg.max_sample_rate().0 >= target_rate
            && cfg.sample_format() == cpal::SampleFormat::F32
        {
            return Ok(cfg.with_sample_rate(cpal::SampleRate(target_rate)));
        }
    }

    // Fall back to default config of the first supported range
    let first = &configs[0];
    let rate = if first.min_sample_rate().0 <= 48000 && first.max_sample_rate().0 >= 48000 {
        48000
    } else if first.min_sample_rate().0 <= 44100 && first.max_sample_rate().0 >= 44100 {
        44100
    } else {
        first.max_sample_rate().0
    };

    Ok(first.with_sample_rate(cpal::SampleRate(rate)))
}

/// Parameters for building an input stream.
struct InputStreamParams {
    needs_resample: bool,
    needs_mono_mix: bool,
    device_rate: u32,
    device_channels: u32,
}

/// Build an input stream for i16 sample format.
#[allow(clippy::too_many_arguments)]
fn build_input_stream_i16(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    tx: mpsc::Sender<Vec<i16>>,
    capturing: Arc<AtomicBool>,
    params: &InputStreamParams,
) -> Result<cpal::Stream, String> {
    let needs_resample = params.needs_resample;
    let needs_mono_mix = params.needs_mono_mix;
    let device_rate = params.device_rate;
    let device_channels = params.device_channels;
    let mut accumulator: Vec<i16> = Vec::with_capacity(FRAME_SIZE * 2);

    let stream = device
        .build_input_stream(
            config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                if !capturing.load(Ordering::Relaxed) {
                    return;
                }

                let mono = if needs_mono_mix && device_channels > 1 {
                    let mut mono_buf = Vec::with_capacity(data.len() / device_channels as usize);
                    for chunk in data.chunks(device_channels as usize) {
                        let sum: i32 = chunk.iter().map(|&s| s as i32).sum();
                        mono_buf.push((sum / device_channels as i32) as i16);
                    }
                    mono_buf
                } else {
                    data.to_vec()
                };

                let resampled = if needs_resample {
                    resample(&mono, device_rate, TARGET_SAMPLE_RATE)
                } else {
                    mono
                };

                accumulator.extend_from_slice(&resampled);

                while accumulator.len() >= FRAME_SIZE {
                    let frame: Vec<i16> = accumulator.drain(..FRAME_SIZE).collect();
                    if tx.try_send(frame).is_err() {
                        break;
                    }
                }
            },
            move |err| {
                error!("Audio input error: {}", err);
            },
            None,
        )
        .map_err(|e| format!("Failed to build input stream: {}", e))?;

    Ok(stream)
}

/// Build an input stream for f32 sample format.
fn build_input_stream_f32(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    tx: mpsc::Sender<Vec<i16>>,
    capturing: Arc<AtomicBool>,
    params: &InputStreamParams,
) -> Result<cpal::Stream, String> {
    let needs_resample = params.needs_resample;
    let needs_mono_mix = params.needs_mono_mix;
    let device_rate = params.device_rate;
    let device_channels = params.device_channels;
    let mut accumulator: Vec<i16> = Vec::with_capacity(FRAME_SIZE * 2);

    let stream = device
        .build_input_stream(
            config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !capturing.load(Ordering::Relaxed) {
                    return;
                }

                let i16_data: Vec<i16> = data
                    .iter()
                    .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                    .collect();

                let mono = if needs_mono_mix && device_channels > 1 {
                    let mut mono_buf =
                        Vec::with_capacity(i16_data.len() / device_channels as usize);
                    for chunk in i16_data.chunks(device_channels as usize) {
                        let sum: i32 = chunk.iter().map(|&s| s as i32).sum();
                        mono_buf.push((sum / device_channels as i32) as i16);
                    }
                    mono_buf
                } else {
                    i16_data
                };

                let resampled = if needs_resample {
                    resample(&mono, device_rate, TARGET_SAMPLE_RATE)
                } else {
                    mono
                };

                accumulator.extend_from_slice(&resampled);

                while accumulator.len() >= FRAME_SIZE {
                    let frame: Vec<i16> = accumulator.drain(..FRAME_SIZE).collect();
                    if tx.try_send(frame).is_err() {
                        break;
                    }
                }
            },
            move |err| {
                error!("Audio input error: {}", err);
            },
            None,
        )
        .map_err(|e| format!("Failed to build input stream (f32): {}", e))?;

    Ok(stream)
}

// ============================================================================
// CpalAudioOutput
// ============================================================================

/// Speaker playback using cpal.
///
/// Decodes WAV data and plays it through the default output device.
/// Supports resampling from any WAV sample rate to the device's native rate.
///
/// The cpal stream runs on a dedicated OS thread since `cpal::Stream` is `!Send`.
pub struct CpalAudioOutput {
    playing: Arc<AtomicBool>,
}

impl CpalAudioOutput {
    pub fn new() -> Self {
        Self {
            playing: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Default for CpalAudioOutput {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AudioOutputProvider for CpalAudioOutput {
    async fn play_wav(&self, wav_data: &[u8]) -> zeus_core::Result<()> {
        // Stop any current playback
        self.stop_playback().await;

        // Parse WAV
        let (wav_rate, wav_channels, samples) = parse_wav_pcm(wav_data)
            .map_err(|e| zeus_core::Error::Internal(format!("WAV parse error: {}", e)))?;

        // Mix to mono if stereo (we'll expand back to device channels later)
        let mono = if wav_channels > 1 {
            stereo_to_mono(&samples)
        } else {
            samples
        };

        let playing = self.playing.clone();
        playing.store(true, Ordering::SeqCst);

        let play_flag = playing.clone();

        // Use a oneshot channel to get result from the playback thread
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();

        // Spawn a dedicated OS thread for playback
        std::thread::Builder::new()
            .name("zeus-audio-output".to_string())
            .spawn(move || {
                let result = run_playback_thread(mono, wav_rate, play_flag);
                let _ = done_tx.send(result);
            })
            .map_err(|e| {
                playing.store(false, Ordering::SeqCst);
                zeus_core::Error::Internal(format!("Failed to spawn playback thread: {}", e))
            })?;

        // Wait for playback to complete
        match done_rx.await {
            Ok(Ok(())) => {
                self.playing.store(false, Ordering::SeqCst);
                Ok(())
            }
            Ok(Err(e)) => {
                self.playing.store(false, Ordering::SeqCst);
                Err(zeus_core::Error::Internal(format!(
                    "Audio playback failed: {}",
                    e
                )))
            }
            Err(_) => {
                self.playing.store(false, Ordering::SeqCst);
                Err(zeus_core::Error::Internal(
                    "Playback thread exited unexpectedly".to_string(),
                ))
            }
        }
    }

    async fn stop_playback(&self) {
        self.playing.store(false, Ordering::SeqCst);
    }

    fn is_playing(&self) -> bool {
        self.playing.load(Ordering::SeqCst)
    }
}

/// Run playback on a dedicated thread.
fn run_playback_thread(
    mono_samples: Vec<i16>,
    wav_rate: u32,
    playing: Arc<AtomicBool>,
) -> Result<(), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("No default audio output device found")?;

    let device_name = device.name().unwrap_or_else(|_| "unknown".to_string());
    debug!(device = %device_name, "Opening audio output device");

    let supported = device
        .supported_output_configs()
        .map_err(|e| format!("Failed to query output configs: {}", e))?;

    let config = find_best_output_config(supported)?;
    let device_rate = config.sample_rate().0;
    let device_channels = config.channels() as usize;
    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();

    // Resample to device rate
    let resampled = if wav_rate != device_rate {
        resample(&mono_samples, wav_rate, device_rate)
    } else {
        mono_samples
    };

    // Expand mono to device channels
    let output_samples: Vec<i16> = if device_channels > 1 {
        let mut expanded = Vec::with_capacity(resampled.len() * device_channels);
        for &s in &resampled {
            for _ in 0..device_channels {
                expanded.push(s);
            }
        }
        expanded
    } else {
        resampled
    };

    let total_samples = output_samples.len();
    let sample_idx = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let finished = Arc::new(AtomicBool::new(false));

    let idx = sample_idx.clone();
    let fin = finished.clone();
    let play_flag = playing.clone();
    let samples = Arc::new(output_samples);

    let stream = match sample_format {
        cpal::SampleFormat::I16 => {
            let samples = samples.clone();
            device
                .build_output_stream(
                    &stream_config,
                    move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                        if !play_flag.load(Ordering::Relaxed) {
                            for sample in data.iter_mut() {
                                *sample = 0;
                            }
                            fin.store(true, Ordering::Relaxed);
                            return;
                        }
                        for sample in data.iter_mut() {
                            let pos = idx.fetch_add(1, Ordering::Relaxed);
                            if pos < samples.len() {
                                *sample = samples[pos];
                            } else {
                                *sample = 0;
                                fin.store(true, Ordering::Relaxed);
                            }
                        }
                    },
                    |err| error!("Audio output error: {}", err),
                    None,
                )
                .map_err(|e| format!("Failed to build output stream: {}", e))?
        }
        cpal::SampleFormat::F32 => {
            let samples = samples.clone();
            device
                .build_output_stream(
                    &stream_config,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        if !play_flag.load(Ordering::Relaxed) {
                            for sample in data.iter_mut() {
                                *sample = 0.0;
                            }
                            fin.store(true, Ordering::Relaxed);
                            return;
                        }
                        for sample in data.iter_mut() {
                            let pos = idx.fetch_add(1, Ordering::Relaxed);
                            if pos < samples.len() {
                                *sample = samples[pos] as f32 / i16::MAX as f32;
                            } else {
                                *sample = 0.0;
                                fin.store(true, Ordering::Relaxed);
                            }
                        }
                    },
                    |err| error!("Audio output error: {}", err),
                    None,
                )
                .map_err(|e| format!("Failed to build output stream (f32): {}", e))?
        }
        other => {
            return Err(format!("Unsupported output sample format: {:?}", other));
        }
    };

    stream
        .play()
        .map_err(|e| format!("Failed to start audio playback: {}", e))?;

    info!(
        samples = total_samples,
        rate = device_rate,
        "Audio playback started"
    );

    // Wait for playback to finish or be stopped
    while !finished.load(Ordering::SeqCst) && playing.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    drop(stream);
    playing.store(false, Ordering::SeqCst);
    debug!("Audio playback finished");

    Ok(())
}

/// Find the best output config, preferring common sample rates.
fn find_best_output_config(
    configs: cpal::SupportedOutputConfigs,
) -> Result<cpal::SupportedStreamConfig, String> {
    let configs: Vec<_> = configs.collect();

    if configs.is_empty() {
        return Err("No supported output configurations".to_string());
    }

    let preferred_rates = [48000u32, 44100, 16000, 22050, 96000];

    for rate in preferred_rates {
        for cfg in &configs {
            if cfg.min_sample_rate().0 <= rate && cfg.max_sample_rate().0 >= rate {
                return Ok(cfg.with_sample_rate(cpal::SampleRate(rate)));
            }
        }
    }

    let first = &configs[0];
    Ok(first.with_max_sample_rate())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpal_audio_input_creation() {
        let input = CpalAudioInput::new();
        assert!(!input.capturing.load(Ordering::SeqCst));
    }

    #[test]
    fn test_cpal_audio_input_default() {
        let input = CpalAudioInput::default();
        assert!(!input.capturing.load(Ordering::SeqCst));
    }

    #[test]
    fn test_cpal_audio_output_creation() {
        let output = CpalAudioOutput::new();
        assert!(!output.playing.load(Ordering::SeqCst));
        assert!(!output.is_playing());
    }

    #[test]
    fn test_cpal_audio_output_default() {
        let output = CpalAudioOutput::default();
        assert!(!output.is_playing());
    }

    #[test]
    fn test_frame_size_constant() {
        assert_eq!(FRAME_SIZE, 512);
    }

    #[test]
    fn test_target_sample_rate_constant() {
        assert_eq!(TARGET_SAMPLE_RATE, 16_000);
    }

    #[test]
    fn test_cpal_host_exists() {
        let host = cpal::default_host();
        let _name = host.id();
    }

    #[test]
    fn test_is_available_returns_bool() {
        let input = CpalAudioInput::new();
        let _available = input.is_available();
    }

    #[tokio::test]
    async fn test_stop_capture_when_not_capturing() {
        let input = CpalAudioInput::new();
        input.stop_capture().await;
        assert!(!input.capturing.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_stop_playback_when_not_playing() {
        let output = CpalAudioOutput::new();
        output.stop_playback().await;
        assert!(!output.is_playing());
    }

    #[tokio::test]
    async fn test_play_wav_invalid_data() {
        let output = CpalAudioOutput::new();
        let result = output.play_wav(b"not a wav file at all").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_play_wav_too_short() {
        let output = CpalAudioOutput::new();
        let result = output.play_wav(&[0u8; 10]).await;
        assert!(result.is_err());
    }
}
