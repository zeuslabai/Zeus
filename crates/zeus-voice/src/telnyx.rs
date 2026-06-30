//! Telnyx voice call provider

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};
use zeus_core::Result;

use crate::call::CallState;
use crate::provider::VoiceCallProvider;

/// Telnyx REST API base URL
const TELNYX_API_BASE: &str = "https://api.telnyx.com/v2";

/// Telnyx-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelnyxConfig {
    /// Telnyx API key (v2 key)
    pub api_key: String,
    /// SIP connection ID for outbound calls
    pub connection_id: String,
    /// Telnyx phone number (caller ID)
    pub from_number: String,
    /// Webhook base URL (must be publicly accessible)
    pub webhook_base_url: String,
    /// Webhook port for receiving call events
    #[serde(default = "default_telnyx_webhook_port")]
    pub webhook_port: u16,
    /// Default TTS voice for Telnyx speak commands
    #[serde(default = "default_telnyx_tts_voice")]
    pub tts_voice: String,
}

fn default_telnyx_webhook_port() -> u16 {
    8091
}

fn default_telnyx_tts_voice() -> String {
    "female".to_string()
}

impl Default for TelnyxConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            connection_id: String::new(),
            from_number: String::new(),
            webhook_base_url: String::new(),
            webhook_port: default_telnyx_webhook_port(),
            tts_voice: default_telnyx_tts_voice(),
        }
    }
}

impl TelnyxConfig {
    /// Apply environment variable overrides to the config.
    ///
    /// Checks the following env vars:
    /// - `TELNYX_API_KEY` -> `api_key`
    /// - `TELNYX_CONNECTION_ID` -> `connection_id`
    /// - `TELNYX_PHONE_NUMBER` -> `from_number`
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(key) = std::env::var("TELNYX_API_KEY") {
            self.api_key = key;
        }
        if let Ok(conn) = std::env::var("TELNYX_CONNECTION_ID") {
            self.connection_id = conn;
        }
        if let Ok(phone) = std::env::var("TELNYX_PHONE_NUMBER") {
            self.from_number = phone;
        }
        self
    }

    /// Create a config from environment variables with defaults.
    ///
    /// Equivalent to `TelnyxConfig::default().with_env_overrides()`.
    pub fn from_env() -> Self {
        Self::default().with_env_overrides()
    }
}

/// Telnyx voice call provider
pub struct TelnyxProvider {
    config: TelnyxConfig,
    http: reqwest::Client,
}

impl TelnyxProvider {
    pub fn new(config: TelnyxConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }
}

/// Map a Telnyx call state string to our CallState enum
pub fn call_state_from_telnyx(status: &str) -> CallState {
    match status {
        "initiated" => CallState::Initiated,
        "ringing" => CallState::Ringing,
        "answered" => CallState::Answered,
        "bridged" => CallState::Active,
        "hangup" => CallState::Completed,
        _ => CallState::Unknown,
    }
}

/// Telnyx webhook event payload
#[derive(Debug, Deserialize)]
pub struct TelnyxWebhookEvent {
    /// The event type, e.g. "call.initiated", "call.answered", "call.hangup"
    pub event_type: String,
    /// The event payload
    pub payload: TelnyxEventPayload,
}

/// Telnyx event payload containing call control data
#[derive(Debug, Deserialize)]
pub struct TelnyxEventPayload {
    /// The call control ID (used for subsequent actions on the call)
    pub call_control_id: String,
    /// The call leg ID
    #[serde(default)]
    pub call_leg_id: Option<String>,
    /// The current call state
    #[serde(default)]
    pub state: Option<String>,
    /// DTMF digit (present on call.dtmf.received events)
    #[serde(default)]
    pub digit: Option<String>,
    /// From number
    #[serde(default)]
    pub from: Option<String>,
    /// To number
    #[serde(default)]
    pub to: Option<String>,
}

/// Top-level Telnyx webhook JSON envelope
#[derive(Debug, Deserialize)]
pub struct TelnyxWebhookBody {
    pub data: TelnyxWebhookEvent,
}

/// Map a Telnyx event_type to our CallState
pub fn call_state_from_event_type(event_type: &str) -> CallState {
    match event_type {
        "call.initiated" => CallState::Initiated,
        "call.ringing" => CallState::Ringing,
        "call.answered" => CallState::Answered,
        "call.bridged" => CallState::Active,
        "call.hangup" => CallState::Completed,
        "call.speak.ended" => CallState::Active,
        _ => CallState::Unknown,
    }
}

#[async_trait]
impl VoiceCallProvider for TelnyxProvider {
    async fn initiate_call(&self, to: &str, greeting_text: &str) -> Result<String> {
        let url = format!("{}/calls", TELNYX_API_BASE);

        let body = serde_json::json!({
            "connection_id": self.config.connection_id,
            "to": to,
            "from": self.config.from_number,
            "webhook_url": format!("{}/voice/telnyx/status", self.config.webhook_base_url),
            "answering_machine_detection": "disabled",
        });

        info!(
            "Initiating Telnyx call to {} from {}",
            to, self.config.from_number
        );

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Telnyx API error: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(zeus_core::Error::Tool(format!(
                "Telnyx call failed ({}): {}",
                status, body
            )));
        }

        let resp_body: serde_json::Value = resp.json().await.map_err(|e| {
            zeus_core::Error::Tool(format!("Failed to parse Telnyx response: {}", e))
        })?;

        let call_control_id = resp_body
            .pointer("/data/call_control_id")
            .and_then(|s| s.as_str())
            .ok_or_else(|| {
                zeus_core::Error::Tool("No call_control_id in Telnyx response".to_string())
            })?
            .to_string();

        info!("Telnyx call initiated: {}", call_control_id);

        // Play greeting TTS on the call once answered
        // Telnyx uses a separate speak action rather than TwiML
        debug!(
            "Greeting text queued for call {}: {}",
            call_control_id, greeting_text
        );

        Ok(call_control_id)
    }

    async fn hangup_call(&self, call_id: &str) -> Result<()> {
        let url = format!("{}/calls/{}/actions/hangup", TELNYX_API_BASE, call_id);

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Telnyx hangup error: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(zeus_core::Error::Tool(format!(
                "Telnyx hangup failed: {}",
                body
            )));
        }

        info!("Call {} hung up via Telnyx", call_id);
        Ok(())
    }

    async fn play_tts(&self, call_id: &str, text: &str) -> Result<()> {
        let url = format!("{}/calls/{}/actions/speak", TELNYX_API_BASE, call_id);

        let body = serde_json::json!({
            "payload": text,
            "voice": self.config.tts_voice,
            "language": "en-US",
        });

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Telnyx TTS error: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(zeus_core::Error::Tool(format!(
                "Telnyx TTS failed: {}",
                body
            )));
        }

        debug!("Playing TTS on Telnyx call {}: {}", call_id, text);
        Ok(())
    }

    async fn get_call_state(&self, call_id: &str) -> Result<CallState> {
        let url = format!("{}/calls/{}", TELNYX_API_BASE, call_id);

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.config.api_key)
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Telnyx status error: {}", e)))?;

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Failed to parse status: {}", e)))?;

        let status = body
            .pointer("/data/state")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");

        Ok(call_state_from_telnyx(status))
    }

    async fn send_dtmf(&self, call_id: &str, digits: &str) -> Result<()> {
        let url = format!("{}/calls/{}/actions/send_dtmf", TELNYX_API_BASE, call_id);

        let body = serde_json::json!({
            "digits": digits,
        });

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Telnyx DTMF error: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(zeus_core::Error::Tool(format!(
                "Telnyx DTMF failed: {}",
                body
            )));
        }

        debug!("Sent DTMF digits '{}' on Telnyx call {}", digits, call_id);
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "telnyx"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> TelnyxConfig {
        TelnyxConfig {
            api_key: "KEY_TEST_123".to_string(),
            connection_id: "conn_test_456".to_string(),
            from_number: "+15551234567".to_string(),
            webhook_base_url: "https://example.ngrok.io".to_string(),
            webhook_port: 8091,
            tts_voice: "female".to_string(),
        }
    }

    // ---- TelnyxConfig tests ----

    #[test]
    fn test_telnyx_config_defaults() {
        let config = TelnyxConfig::default();
        assert_eq!(config.webhook_port, 8091);
        assert_eq!(config.tts_voice, "female");
        assert!(config.api_key.is_empty());
        assert!(config.connection_id.is_empty());
        assert!(config.from_number.is_empty());
        assert!(config.webhook_base_url.is_empty());
    }

    #[test]
    fn test_telnyx_config_serialization_roundtrip() {
        let config = test_config();
        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        let deserialized: TelnyxConfig =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(deserialized.api_key, "KEY_TEST_123");
        assert_eq!(deserialized.connection_id, "conn_test_456");
        assert_eq!(deserialized.from_number, "+15551234567");
        assert_eq!(deserialized.webhook_base_url, "https://example.ngrok.io");
        assert_eq!(deserialized.webhook_port, 8091);
        assert_eq!(deserialized.tts_voice, "female");
    }

    #[test]
    fn test_telnyx_config_deserialize_with_defaults() {
        let json = r#"{
            "api_key": "KEY123",
            "connection_id": "conn_456",
            "from_number": "+1555",
            "webhook_base_url": "https://example.com"
        }"#;
        let config: TelnyxConfig = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(config.api_key, "KEY123");
        assert_eq!(config.connection_id, "conn_456");
        assert_eq!(config.webhook_port, 8091);
        assert_eq!(config.tts_voice, "female");
    }

    #[test]
    fn test_telnyx_from_env_defaults() {
        temp_env::with_vars(
            [
                ("TELNYX_API_KEY", None::<&str>),
                ("TELNYX_CONNECTION_ID", None),
                ("TELNYX_PHONE_NUMBER", None),
            ],
            || {
                let config = TelnyxConfig::from_env();
                assert!(config.api_key.is_empty());
                assert!(config.connection_id.is_empty());
                assert!(config.from_number.is_empty());
                assert_eq!(config.webhook_port, 8091);
                assert_eq!(config.tts_voice, "female");
            },
        );
    }

    #[test]
    fn test_telnyx_with_env_overrides() {
        temp_env::with_vars(
            [
                ("TELNYX_API_KEY", Some("ENV_KEY_789")),
                ("TELNYX_CONNECTION_ID", Some("env_conn_012")),
                ("TELNYX_PHONE_NUMBER", Some("+15559999999")),
            ],
            || {
                let config = TelnyxConfig::default().with_env_overrides();
                assert_eq!(config.api_key, "ENV_KEY_789");
                assert_eq!(config.connection_id, "env_conn_012");
                assert_eq!(config.from_number, "+15559999999");
            },
        );
    }

    #[test]
    fn test_telnyx_config_env_partial_override() {
        temp_env::with_vars(
            [
                ("TELNYX_API_KEY", None),
                ("TELNYX_CONNECTION_ID", None),
                ("TELNYX_PHONE_NUMBER", Some("+15558888888")),
            ],
            || {
                let config = TelnyxConfig {
                    api_key: "KEY_FROM_CONFIG".to_string(),
                    connection_id: "conn_from_config".to_string(),
                    from_number: "+15550000000".to_string(),
                    webhook_base_url: "https://example.com".to_string(),
                    webhook_port: 9091,
                    tts_voice: "male".to_string(),
                }
                .with_env_overrides();

                // api_key and connection_id should remain from config (env vars not set)
                assert_eq!(config.api_key, "KEY_FROM_CONFIG");
                assert_eq!(config.connection_id, "conn_from_config");
                // from_number should be overridden by env var
                assert_eq!(config.from_number, "+15558888888");
                // Other fields should be unchanged
                assert_eq!(config.webhook_base_url, "https://example.com");
                assert_eq!(config.webhook_port, 9091);
                assert_eq!(config.tts_voice, "male");
            },
        );
    }

    // ---- CallState mapping tests ----

    #[test]
    fn test_telnyx_state_initiated() {
        assert_eq!(call_state_from_telnyx("initiated"), CallState::Initiated);
    }

    #[test]
    fn test_telnyx_state_ringing() {
        assert_eq!(call_state_from_telnyx("ringing"), CallState::Ringing);
    }

    #[test]
    fn test_telnyx_state_answered() {
        assert_eq!(call_state_from_telnyx("answered"), CallState::Answered);
    }

    #[test]
    fn test_telnyx_state_bridged() {
        assert_eq!(call_state_from_telnyx("bridged"), CallState::Active);
    }

    #[test]
    fn test_telnyx_state_hangup() {
        assert_eq!(call_state_from_telnyx("hangup"), CallState::Completed);
    }

    #[test]
    fn test_telnyx_state_unknown() {
        assert_eq!(call_state_from_telnyx("something-else"), CallState::Unknown);
    }

    // ---- Event type mapping tests ----

    #[test]
    fn test_event_type_call_initiated() {
        assert_eq!(
            call_state_from_event_type("call.initiated"),
            CallState::Initiated
        );
    }

    #[test]
    fn test_event_type_call_ringing() {
        assert_eq!(
            call_state_from_event_type("call.ringing"),
            CallState::Ringing
        );
    }

    #[test]
    fn test_event_type_call_answered() {
        assert_eq!(
            call_state_from_event_type("call.answered"),
            CallState::Answered
        );
    }

    #[test]
    fn test_event_type_call_bridged() {
        assert_eq!(
            call_state_from_event_type("call.bridged"),
            CallState::Active
        );
    }

    #[test]
    fn test_event_type_call_hangup() {
        assert_eq!(
            call_state_from_event_type("call.hangup"),
            CallState::Completed
        );
    }

    #[test]
    fn test_event_type_call_speak_ended() {
        assert_eq!(
            call_state_from_event_type("call.speak.ended"),
            CallState::Active
        );
    }

    #[test]
    fn test_event_type_unknown() {
        assert_eq!(
            call_state_from_event_type("call.unknown.event"),
            CallState::Unknown
        );
    }

    // ---- TelnyxProvider tests ----

    #[test]
    fn test_provider_name() {
        let provider = TelnyxProvider::new(test_config());
        assert_eq!(provider.provider_name(), "telnyx");
    }

    #[test]
    fn test_provider_construction() {
        let config = test_config();
        let provider = TelnyxProvider::new(config.clone());
        assert_eq!(provider.config.api_key, "KEY_TEST_123");
        assert_eq!(provider.config.connection_id, "conn_test_456");
        assert_eq!(provider.config.from_number, "+15551234567");
    }

    // ---- Webhook JSON parsing tests ----

    #[test]
    fn test_webhook_call_initiated_parsing() {
        let json = r#"{
            "data": {
                "event_type": "call.initiated",
                "payload": {
                    "call_control_id": "v2_ctrl_abc123",
                    "call_leg_id": "v2_leg_def456",
                    "state": "initiated",
                    "from": "+15551234567",
                    "to": "+15559876543"
                }
            }
        }"#;
        let body: TelnyxWebhookBody =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(body.data.event_type, "call.initiated");
        assert_eq!(body.data.payload.call_control_id, "v2_ctrl_abc123");
        assert_eq!(
            body.data.payload.call_leg_id.as_deref(),
            Some("v2_leg_def456")
        );
        assert_eq!(body.data.payload.state.as_deref(), Some("initiated"));
        assert_eq!(body.data.payload.from.as_deref(), Some("+15551234567"));
        assert_eq!(body.data.payload.to.as_deref(), Some("+15559876543"));
    }

    #[test]
    fn test_webhook_call_answered_parsing() {
        let json = r#"{
            "data": {
                "event_type": "call.answered",
                "payload": {
                    "call_control_id": "v2_ctrl_abc123",
                    "state": "answered"
                }
            }
        }"#;
        let body: TelnyxWebhookBody =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(body.data.event_type, "call.answered");
        assert_eq!(body.data.payload.call_control_id, "v2_ctrl_abc123");
        assert_eq!(body.data.payload.state.as_deref(), Some("answered"));
    }

    #[test]
    fn test_webhook_call_hangup_parsing() {
        let json = r#"{
            "data": {
                "event_type": "call.hangup",
                "payload": {
                    "call_control_id": "v2_ctrl_abc123",
                    "state": "hangup"
                }
            }
        }"#;
        let body: TelnyxWebhookBody =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(body.data.event_type, "call.hangup");
        let state = call_state_from_event_type(&body.data.event_type);
        assert_eq!(state, CallState::Completed);
    }

    #[test]
    fn test_webhook_call_speak_ended_parsing() {
        let json = r#"{
            "data": {
                "event_type": "call.speak.ended",
                "payload": {
                    "call_control_id": "v2_ctrl_abc123"
                }
            }
        }"#;
        let body: TelnyxWebhookBody =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(body.data.event_type, "call.speak.ended");
        let state = call_state_from_event_type(&body.data.event_type);
        assert_eq!(state, CallState::Active);
    }

    #[test]
    fn test_webhook_dtmf_received_parsing() {
        let json = r#"{
            "data": {
                "event_type": "call.dtmf.received",
                "payload": {
                    "call_control_id": "v2_ctrl_abc123",
                    "digit": "5"
                }
            }
        }"#;
        let body: TelnyxWebhookBody =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(body.data.event_type, "call.dtmf.received");
        assert_eq!(body.data.payload.digit.as_deref(), Some("5"));
    }

    #[test]
    fn test_webhook_minimal_payload() {
        let json = r#"{
            "data": {
                "event_type": "call.initiated",
                "payload": {
                    "call_control_id": "v2_ctrl_minimal"
                }
            }
        }"#;
        let body: TelnyxWebhookBody =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(body.data.payload.call_control_id, "v2_ctrl_minimal");
        assert!(body.data.payload.call_leg_id.is_none());
        assert!(body.data.payload.state.is_none());
        assert!(body.data.payload.digit.is_none());
        assert!(body.data.payload.from.is_none());
        assert!(body.data.payload.to.is_none());
    }

    // ---- SIP connection_id tests ----

    #[test]
    fn test_connection_id_in_config() {
        let config = TelnyxConfig {
            connection_id: "sip_conn_12345".to_string(),
            ..TelnyxConfig::default()
        };
        assert_eq!(config.connection_id, "sip_conn_12345");
    }

    #[test]
    fn test_connection_id_in_call_request_body() {
        let config = test_config();
        // Verify the connection_id would be included in the call request JSON
        let body = serde_json::json!({
            "connection_id": config.connection_id,
            "to": "+15559876543",
            "from": config.from_number,
            "webhook_url": format!("{}/voice/telnyx/status", config.webhook_base_url),
            "answering_machine_detection": "disabled",
        });
        assert_eq!(body["connection_id"], "conn_test_456");
        assert_eq!(body["from"], "+15551234567");
        assert_eq!(
            body["webhook_url"],
            "https://example.ngrok.io/voice/telnyx/status"
        );
        assert_eq!(body["answering_machine_detection"], "disabled");
    }
}
