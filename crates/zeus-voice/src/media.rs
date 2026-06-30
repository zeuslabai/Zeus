//! WebSocket media stream handler for real-time bidirectional audio
//!
//! Handles Twilio's Media Streams protocol:
//! - Receives mu-law 8kHz G.711 audio from the caller
//! - Sends audio back to the caller
//! - Provides hooks for STT (speech-to-text) and TTS (text-to-speech)

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use tracing::warn;

/// Twilio Media Stream incoming message types
#[derive(Debug, Deserialize)]
#[serde(tag = "event")]
#[serde(rename_all = "lowercase")]
pub enum StreamMessage {
    /// Connection established
    Connected {
        #[serde(default)]
        protocol: Option<String>,
    },
    /// Stream started - contains stream SID and call SID
    Start {
        #[serde(rename = "streamSid")]
        stream_sid: String,
        start: StreamStartData,
    },
    /// Audio data from caller
    Media {
        #[serde(rename = "streamSid")]
        stream_sid: String,
        media: MediaPayload,
    },
    /// Stream stopped
    Stop {
        #[serde(rename = "streamSid")]
        stream_sid: String,
    },
    /// DTMF tone received
    #[serde(rename = "dtmf")]
    Dtmf {
        #[serde(rename = "streamSid")]
        stream_sid: String,
        #[serde(rename = "dtmf")]
        dtmf_data: DtmfData,
    },
}

#[derive(Debug, Deserialize)]
pub struct StreamStartData {
    #[serde(rename = "callSid")]
    pub call_sid: String,
    #[serde(rename = "accountSid")]
    pub account_sid: String,
    #[serde(default)]
    pub tracks: Vec<String>,
    #[serde(rename = "mediaFormat", default)]
    pub media_format: Option<MediaFormat>,
}

#[derive(Debug, Deserialize)]
pub struct MediaFormat {
    pub encoding: String,
    #[serde(rename = "sampleRate")]
    pub sample_rate: u32,
    pub channels: u32,
}

#[derive(Debug, Deserialize)]
pub struct MediaPayload {
    pub track: String,
    pub chunk: String,
    pub timestamp: String,
    pub payload: String, // base64-encoded mu-law audio
}

#[derive(Debug, Deserialize)]
pub struct DtmfData {
    pub digit: String,
}

/// Outgoing media message to Twilio
#[derive(Debug, Serialize)]
pub struct OutgoingMedia {
    pub event: String,
    #[serde(rename = "streamSid")]
    pub stream_sid: String,
    pub media: OutgoingPayload,
}

#[derive(Debug, Serialize)]
pub struct OutgoingPayload {
    pub payload: String, // base64-encoded mu-law audio
}

/// Clear the audio queue on the stream
#[derive(Debug, Serialize)]
pub struct ClearMessage {
    pub event: String,
    #[serde(rename = "streamSid")]
    pub stream_sid: String,
}

/// Handler for Twilio media streams
pub struct MediaStreamHandler {
    /// Audio buffer for incoming audio (mu-law 8kHz)
    audio_buffer: Vec<u8>,
    /// Maximum buffer size before processing (2 seconds at 8kHz = 16000 bytes)
    max_buffer_size: usize,
}

impl MediaStreamHandler {
    pub fn new() -> Self {
        Self {
            audio_buffer: Vec::new(),
            max_buffer_size: 16000, // ~2 seconds of 8kHz mu-law
        }
    }

    /// Process an incoming media message, returns accumulated audio if buffer is full
    pub fn process_media(&mut self, media: &MediaPayload) -> Option<Vec<u8>> {
        // Decode base64 payload to raw mu-law bytes
        match BASE64.decode(&media.payload) {
            Ok(audio_data) => {
                self.audio_buffer.extend_from_slice(&audio_data);

                if self.audio_buffer.len() >= self.max_buffer_size {
                    let audio = self.audio_buffer.drain(..).collect();
                    Some(audio)
                } else {
                    None
                }
            }
            Err(e) => {
                warn!("Failed to decode media payload: {}", e);
                None
            }
        }
    }

    /// Create an outgoing media message with audio data
    pub fn create_outgoing(stream_sid: &str, audio_data: &[u8]) -> OutgoingMedia {
        OutgoingMedia {
            event: "media".to_string(),
            stream_sid: stream_sid.to_string(),
            media: OutgoingPayload {
                payload: BASE64.encode(audio_data),
            },
        }
    }

    /// Create a clear message to stop current audio
    pub fn create_clear(stream_sid: &str) -> ClearMessage {
        ClearMessage {
            event: "clear".to_string(),
            stream_sid: stream_sid.to_string(),
        }
    }

    /// Get current buffer size
    pub fn buffer_size(&self) -> usize {
        self.audio_buffer.len()
    }

    /// Flush the buffer and return remaining audio
    pub fn flush(&mut self) -> Vec<u8> {
        self.audio_buffer.drain(..).collect()
    }
}

impl Default for MediaStreamHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_media_payload(audio_bytes: &[u8]) -> MediaPayload {
        MediaPayload {
            track: "inbound".to_string(),
            chunk: "1".to_string(),
            timestamp: "1234".to_string(),
            payload: BASE64.encode(audio_bytes),
        }
    }

    #[test]
    fn test_media_handler_new() {
        let handler = MediaStreamHandler::new();
        assert_eq!(handler.buffer_size(), 0);
    }

    #[test]
    fn test_media_handler_default() {
        let handler = MediaStreamHandler::default();
        assert_eq!(handler.buffer_size(), 0);
    }

    #[test]
    fn test_process_media_small_payload() {
        let mut handler = MediaStreamHandler::new();
        let payload = make_media_payload(&[0u8; 100]);

        // Small payload should buffer and return None
        let result = handler.process_media(&payload);
        assert!(result.is_none());
        assert_eq!(handler.buffer_size(), 100);
    }

    #[test]
    fn test_process_media_accumulates() {
        let mut handler = MediaStreamHandler::new();

        for _ in 0..10 {
            let payload = make_media_payload(&[0u8; 1000]);
            handler.process_media(&payload);
        }

        assert_eq!(handler.buffer_size(), 10000);
    }

    #[test]
    fn test_process_media_returns_when_full() {
        let mut handler = MediaStreamHandler::new();

        // Fill buffer to exactly the threshold
        let payload = make_media_payload(&[0xAB; 16000]);
        let result = handler.process_media(&payload);

        assert!(result.is_some());
        let audio = result.expect("operation should succeed");
        assert_eq!(audio.len(), 16000);
        assert!(audio.iter().all(|&b| b == 0xAB));

        // Buffer should be empty after draining
        assert_eq!(handler.buffer_size(), 0);
    }

    #[test]
    fn test_process_media_returns_when_over_threshold() {
        let mut handler = MediaStreamHandler::new();

        // Add 15000 bytes (under threshold)
        let payload1 = make_media_payload(&[0x01; 15000]);
        assert!(handler.process_media(&payload1).is_none());

        // Add 2000 more bytes (over threshold: 17000 >= 16000)
        let payload2 = make_media_payload(&[0x02; 2000]);
        let result = handler.process_media(&payload2);

        assert!(result.is_some());
        let audio = result.expect("operation should succeed");
        assert_eq!(audio.len(), 17000);
        assert_eq!(handler.buffer_size(), 0);
    }

    #[test]
    fn test_process_media_invalid_base64() {
        let mut handler = MediaStreamHandler::new();
        let payload = MediaPayload {
            track: "inbound".to_string(),
            chunk: "1".to_string(),
            timestamp: "1234".to_string(),
            payload: "!!!not-valid-base64!!!".to_string(),
        };

        let result = handler.process_media(&payload);
        assert!(result.is_none());
        assert_eq!(handler.buffer_size(), 0);
    }

    #[test]
    fn test_create_outgoing() {
        let audio = vec![0x01, 0x02, 0x03, 0x04];
        let msg = MediaStreamHandler::create_outgoing("MZ123", &audio);

        assert_eq!(msg.event, "media");
        assert_eq!(msg.stream_sid, "MZ123");

        // Verify the payload is valid base64 that decodes to our audio
        let decoded = BASE64
            .decode(&msg.media.payload)
            .expect("decode should succeed");
        assert_eq!(decoded, audio);
    }

    #[test]
    fn test_create_outgoing_empty_audio() {
        let msg = MediaStreamHandler::create_outgoing("MZ123", &[]);
        assert_eq!(msg.event, "media");
        let decoded = BASE64
            .decode(&msg.media.payload)
            .expect("decode should succeed");
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_create_clear() {
        let msg = MediaStreamHandler::create_clear("MZ456");
        assert_eq!(msg.event, "clear");
        assert_eq!(msg.stream_sid, "MZ456");
    }

    #[test]
    fn test_flush_empty() {
        let mut handler = MediaStreamHandler::new();
        let flushed = handler.flush();
        assert!(flushed.is_empty());
    }

    #[test]
    fn test_flush_with_data() {
        let mut handler = MediaStreamHandler::new();
        let payload = make_media_payload(&[0xCC; 500]);
        handler.process_media(&payload);
        assert_eq!(handler.buffer_size(), 500);

        let flushed = handler.flush();
        assert_eq!(flushed.len(), 500);
        assert!(flushed.iter().all(|&b| b == 0xCC));
        assert_eq!(handler.buffer_size(), 0);
    }

    #[test]
    fn test_outgoing_media_serialization() {
        let msg = MediaStreamHandler::create_outgoing("MZ789", &[0x01, 0x02]);
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");

        assert!(json.contains(r#""event":"media""#));
        assert!(json.contains(r#""streamSid":"MZ789""#));
        assert!(json.contains(r#""payload":"#));
    }

    #[test]
    fn test_clear_message_serialization() {
        let msg = MediaStreamHandler::create_clear("MZ789");
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");

        assert!(json.contains(r#""event":"clear""#));
        assert!(json.contains(r#""streamSid":"MZ789""#));
    }

    #[test]
    fn test_stream_message_connected_deserialization() {
        let json = r#"{"event":"connected","protocol":"Call"}"#;
        let msg: StreamMessage = serde_json::from_str(json).expect("should parse successfully");
        match msg {
            StreamMessage::Connected { protocol } => {
                assert_eq!(protocol.expect("operation should succeed"), "Call");
            }
            _ => panic!("Expected Connected variant"),
        }
    }

    #[test]
    fn test_stream_message_start_deserialization() {
        let json = r#"{
            "event": "start",
            "streamSid": "MZ123",
            "start": {
                "callSid": "CA456",
                "accountSid": "AC789",
                "tracks": ["inbound"],
                "mediaFormat": {
                    "encoding": "audio/x-mulaw",
                    "sampleRate": 8000,
                    "channels": 1
                }
            }
        }"#;
        let msg: StreamMessage = serde_json::from_str(json).expect("should parse successfully");
        match msg {
            StreamMessage::Start {
                stream_sid, start, ..
            } => {
                assert_eq!(stream_sid, "MZ123");
                assert_eq!(start.call_sid, "CA456");
                assert_eq!(start.account_sid, "AC789");
                assert_eq!(start.tracks, vec!["inbound"]);
                let fmt = start.media_format.expect("operation should succeed");
                assert_eq!(fmt.encoding, "audio/x-mulaw");
                assert_eq!(fmt.sample_rate, 8000);
                assert_eq!(fmt.channels, 1);
            }
            _ => panic!("Expected Start variant"),
        }
    }

    #[test]
    fn test_stream_message_media_deserialization() {
        let json = r#"{
            "event": "media",
            "streamSid": "MZ123",
            "media": {
                "track": "inbound",
                "chunk": "42",
                "timestamp": "12345",
                "payload": "AQID"
            }
        }"#;
        let msg: StreamMessage = serde_json::from_str(json).expect("should parse successfully");
        match msg {
            StreamMessage::Media {
                stream_sid, media, ..
            } => {
                assert_eq!(stream_sid, "MZ123");
                assert_eq!(media.track, "inbound");
                assert_eq!(media.chunk, "42");
                assert_eq!(media.timestamp, "12345");
                // "AQID" is base64 for [1, 2, 3]
                let decoded = BASE64
                    .decode(&media.payload)
                    .expect("decode should succeed");
                assert_eq!(decoded, vec![1, 2, 3]);
            }
            _ => panic!("Expected Media variant"),
        }
    }

    #[test]
    fn test_stream_message_stop_deserialization() {
        let json = r#"{"event":"stop","streamSid":"MZ123"}"#;
        let msg: StreamMessage = serde_json::from_str(json).expect("should parse successfully");
        match msg {
            StreamMessage::Stop { stream_sid } => {
                assert_eq!(stream_sid, "MZ123");
            }
            _ => panic!("Expected Stop variant"),
        }
    }

    #[test]
    fn test_stream_message_dtmf_deserialization() {
        let json = r#"{
            "event": "dtmf",
            "streamSid": "MZ123",
            "dtmf": {
                "digit": "5"
            }
        }"#;
        let msg: StreamMessage = serde_json::from_str(json).expect("should parse successfully");
        match msg {
            StreamMessage::Dtmf {
                stream_sid,
                dtmf_data,
            } => {
                assert_eq!(stream_sid, "MZ123");
                assert_eq!(dtmf_data.digit, "5");
            }
            _ => panic!("Expected Dtmf variant"),
        }
    }
}
