//! Audio format conversion for voice pipeline
//!
//! Converts TTS output (PCM WAV) to mu-law 8kHz format required by
//! Twilio media streams, and vice versa.

/// Linear 16-bit PCM sample to mu-law compressed byte.
///
/// Uses the ITU-T G.711 mu-law encoding standard.
/// Input: signed 16-bit PCM sample (-32768..32767)
/// Output: mu-law encoded byte (0..255)
pub fn pcm16_to_mulaw(sample: i16) -> u8 {
    const BIAS: i32 = 0x84; // 132
    const CLIP: i32 = 32635;

    let sign: i32;
    let mut sample = sample as i32;

    // Get the sign and magnitude
    if sample < 0 {
        sign = 0x80;
        sample = -sample;
    } else {
        sign = 0;
    }

    // Clip the magnitude
    if sample > CLIP {
        sample = CLIP;
    }
    sample += BIAS;

    // Find the segment (exponent)
    let mut exponent: i32 = 7;
    let mut mask: i32 = 0x4000;
    while exponent > 0 {
        if (sample & mask) != 0 {
            break;
        }
        exponent -= 1;
        mask >>= 1;
    }

    // Extract the mantissa
    let mantissa = (sample >> (exponent + 3)) & 0x0F;

    // Combine sign, exponent, mantissa and complement
    let mulaw_byte = sign | (exponent << 4) | mantissa;
    !mulaw_byte as u8 // complement
}

/// Convert a buffer of signed 16-bit PCM samples to mu-law bytes.
pub fn pcm16_buf_to_mulaw(pcm: &[i16]) -> Vec<u8> {
    pcm.iter().map(|&s| pcm16_to_mulaw(s)).collect()
}

/// Parse a WAV file and extract raw PCM samples as i16.
///
/// Supports:
/// - PCM 16-bit (format tag 1)
/// - PCM 8-bit (format tag 1, converted to 16-bit)
///
/// Returns (sample_rate, channels, samples).
pub fn parse_wav_pcm(wav_data: &[u8]) -> Result<(u32, u16, Vec<i16>), String> {
    if wav_data.len() < 44 {
        return Err("WAV data too short".to_string());
    }
    if &wav_data[0..4] != b"RIFF" || &wav_data[8..12] != b"WAVE" {
        return Err("Not a valid WAV file".to_string());
    }

    // Parse fmt chunk
    let mut offset = 12;
    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    let mut bits_per_sample = 0u16;
    let mut format_tag = 0u16;
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

        if chunk_id == b"fmt " {
            if chunk_size < 16 || offset + 8 + 16 > wav_data.len() {
                return Err("fmt chunk too small".to_string());
            }
            let fmt = &wav_data[offset + 8..];
            format_tag = u16::from_le_bytes([fmt[0], fmt[1]]);
            channels = u16::from_le_bytes([fmt[2], fmt[3]]);
            sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
            bits_per_sample = u16::from_le_bytes([fmt[14], fmt[15]]);
        } else if chunk_id == b"data" {
            data_start = offset + 8;
            data_size = chunk_size;
            break;
        }

        offset += 8 + chunk_size as usize;
        // Align to even boundary
        if offset % 2 != 0 {
            offset += 1;
        }
    }

    if data_start == 0 {
        return Err("No data chunk found".to_string());
    }

    if format_tag != 1 {
        return Err(format!(
            "Unsupported WAV format tag: {} (only PCM=1 supported)",
            format_tag
        ));
    }

    let data_end = (data_start + data_size as usize).min(wav_data.len());
    let data = &wav_data[data_start..data_end];

    let samples = match bits_per_sample {
        16 => data
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect(),
        8 => {
            // 8-bit PCM is unsigned (0-255), center is 128
            data.iter().map(|&b| ((b as i16) - 128) * 256).collect()
        }
        _ => return Err(format!("Unsupported bits per sample: {}", bits_per_sample)),
    };

    Ok((sample_rate, channels, samples))
}

/// Simple linear interpolation resampler.
///
/// Resamples from `from_rate` to `to_rate` Hz.
pub fn resample(samples: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }

    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = (samples.len() as f64 / ratio).ceil() as usize;
    let mut output = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let src_idx = src_pos as usize;
        let frac = src_pos - src_idx as f64;

        if src_idx + 1 < samples.len() {
            let s = samples[src_idx] as f64 * (1.0 - frac) + samples[src_idx + 1] as f64 * frac;
            output.push(s.round() as i16);
        } else if src_idx < samples.len() {
            output.push(samples[src_idx]);
        }
    }

    output
}

/// Mix stereo interleaved samples down to mono.
pub fn stereo_to_mono(samples: &[i16]) -> Vec<i16> {
    samples
        .chunks_exact(2)
        .map(|pair| ((pair[0] as i32 + pair[1] as i32) / 2) as i16)
        .collect()
}

/// Convert WAV audio bytes to mu-law 8kHz mono suitable for Twilio.
///
/// Handles:
/// - WAV parsing (PCM 16-bit or 8-bit)
/// - Stereo to mono conversion
/// - Resampling to 8000 Hz
/// - PCM to mu-law encoding
pub fn wav_to_mulaw_8k(wav_data: &[u8]) -> Result<Vec<u8>, String> {
    let (sample_rate, channels, samples) = parse_wav_pcm(wav_data)?;

    // Stereo to mono if needed
    let mono = if channels > 1 {
        stereo_to_mono(&samples)
    } else {
        samples
    };

    // Resample to 8000 Hz
    let resampled = resample(&mono, sample_rate, 8000);

    // Convert to mu-law
    Ok(pcm16_buf_to_mulaw(&resampled))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pcm16_to_mulaw_silence() {
        // Silence (0) should encode to ~0xFF (mu-law silence)
        let result = pcm16_to_mulaw(0);
        assert_eq!(result, 0xFF);
    }

    #[test]
    fn test_pcm16_to_mulaw_max() {
        let result = pcm16_to_mulaw(i16::MAX);
        // Positive max should encode to a low mu-law value (loud positive)
        // In mu-law, the complement means positive samples have bit 7 set
        // after the NOT operation. The exact value depends on the encoding.
        assert!(result < 0x90); // positive, high magnitude
    }

    #[test]
    fn test_pcm16_to_mulaw_negative() {
        let result = pcm16_to_mulaw(-1000);
        // Negative samples have sign bit set (bit 7) in the complement
        // Just verify it's different from positive
        let pos_result = pcm16_to_mulaw(1000);
        assert_ne!(result, pos_result);
    }

    #[test]
    fn test_pcm16_buf_to_mulaw() {
        let pcm = vec![0i16, 1000, -1000, i16::MAX, i16::MIN + 1];
        let mulaw = pcm16_buf_to_mulaw(&pcm);
        assert_eq!(mulaw.len(), 5);
    }

    #[test]
    fn test_resample_same_rate() {
        let samples = vec![100, 200, 300, 400];
        let result = resample(&samples, 44100, 44100);
        assert_eq!(result, samples);
    }

    #[test]
    fn test_resample_downsample() {
        // 16000 Hz → 8000 Hz should halve the samples (approximately)
        let samples: Vec<i16> = (0..16000).map(|i| (i % 256) as i16).collect();
        let result = resample(&samples, 16000, 8000);
        assert!((result.len() as i32 - 8000).abs() <= 1);
    }

    #[test]
    fn test_resample_empty() {
        let result = resample(&[], 44100, 8000);
        assert!(result.is_empty());
    }

    #[test]
    fn test_stereo_to_mono() {
        let stereo = vec![100i16, 200, 300, 400]; // 2 stereo frames
        let mono = stereo_to_mono(&stereo);
        assert_eq!(mono.len(), 2);
        assert_eq!(mono[0], 150); // (100 + 200) / 2
        assert_eq!(mono[1], 350); // (300 + 400) / 2
    }

    #[test]
    fn test_parse_wav_pcm_valid() {
        // Build a minimal valid WAV: 16-bit PCM, 44100 Hz, mono, 4 samples
        let samples: Vec<i16> = vec![0, 1000, -1000, 500];
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
        wav.extend_from_slice(&44100u32.to_le_bytes()); // sample rate
        wav.extend_from_slice(&(44100u32 * 2).to_le_bytes()); // byte rate
        wav.extend_from_slice(&2u16.to_le_bytes()); // block align
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        for &s in &samples {
            wav.extend_from_slice(&s.to_le_bytes());
        }

        let (rate, channels, parsed) = parse_wav_pcm(&wav).unwrap();
        assert_eq!(rate, 44100);
        assert_eq!(channels, 1);
        assert_eq!(parsed, samples);
    }

    #[test]
    fn test_parse_wav_pcm_too_short() {
        assert!(parse_wav_pcm(&[0u8; 10]).is_err());
    }

    #[test]
    fn test_parse_wav_pcm_invalid_header() {
        let mut wav = vec![0u8; 44];
        wav[0..4].copy_from_slice(b"XXXX");
        assert!(parse_wav_pcm(&wav).is_err());
    }

    #[test]
    fn test_wav_to_mulaw_8k() {
        // Build a 16kHz mono WAV and convert
        let num_samples = 16000; // 1 second at 16kHz
        let samples: Vec<i16> = (0..num_samples)
            .map(|i| ((i * 100) % 32000) as i16)
            .collect();

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
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&(16000u32 * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        for &s in &samples {
            wav.extend_from_slice(&s.to_le_bytes());
        }

        let mulaw = wav_to_mulaw_8k(&wav).unwrap();
        // 16kHz → 8kHz should produce ~8000 mu-law bytes
        assert!((mulaw.len() as i32 - 8000).abs() <= 1);
    }
}
