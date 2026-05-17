//! Plivo voice call provider

use async_trait::async_trait;
use tracing::{debug, info};
use zeus_core::Result;

use crate::call::CallState;
use crate::provider::VoiceCallProvider;

/// Plivo REST API base URL
const PLIVO_API_BASE: &str = "https://api.plivo.com/v1/Account";

/// Plivo voice call configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlivoConfig {
    /// Plivo Auth ID
    pub auth_id: String,
    /// Plivo Auth Token
    pub auth_token: String,
    /// Plivo phone number (caller ID)
    pub from_number: String,
    /// Webhook base URL (must be publicly accessible)
    /// e.g., "https://your-domain.com" or ngrok tunnel
    pub webhook_base_url: String,
    /// Webhook port for receiving call events
    #[serde(default = "default_webhook_port")]
    pub webhook_port: u16,
    /// Default TTS voice for Plivo Speak
    #[serde(default = "default_tts_voice")]
    pub tts_voice: String,
    /// Optional custom answer URL for call flow control
    #[serde(default)]
    pub answer_url: Option<String>,
}

fn default_webhook_port() -> u16 {
    8090
}

fn default_tts_voice() -> String {
    "WOMAN".to_string()
}

impl Default for PlivoConfig {
    fn default() -> Self {
        Self {
            auth_id: String::new(),
            auth_token: String::new(),
            from_number: String::new(),
            webhook_base_url: String::new(),
            webhook_port: default_webhook_port(),
            tts_voice: default_tts_voice(),
            answer_url: None,
        }
    }
}

impl PlivoConfig {
    /// Apply environment variable overrides to the config.
    ///
    /// Checks the following env vars:
    /// - `PLIVO_AUTH_ID` -> `auth_id`
    /// - `PLIVO_AUTH_TOKEN` -> `auth_token`
    /// - `PLIVO_PHONE_NUMBER` -> `from_number`
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(id) = std::env::var("PLIVO_AUTH_ID") {
            self.auth_id = id;
        }
        if let Ok(token) = std::env::var("PLIVO_AUTH_TOKEN") {
            self.auth_token = token;
        }
        if let Ok(phone) = std::env::var("PLIVO_PHONE_NUMBER") {
            self.from_number = phone;
        }
        self
    }

    /// Create a config from environment variables with defaults.
    ///
    /// Equivalent to `PlivoConfig::default().with_env_overrides()`.
    pub fn from_env() -> Self {
        Self::default().with_env_overrides()
    }
}

/// Plivo voice call provider
pub struct PlivoProvider {
    config: PlivoConfig,
    http: reqwest::Client,
}

impl PlivoProvider {
    pub fn new(config: PlivoConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    /// Build the Plivo XML for initial call with greeting
    pub fn greeting_xml(&self, text: &str) -> String {
        format!(
            r#"<Response>
    <Speak voice="{}" language="en-US">{}</Speak>
    <Wait length="60"/>
</Response>"#,
            self.config.tts_voice,
            xml_escape(text),
        )
    }

    /// Build Plivo XML for sending DTMF tones
    pub fn dtmf_xml(&self, digits: &str) -> String {
        format!(
            r#"<Response>
    <DTMF>{}</DTMF>
    <Wait length="60"/>
</Response>"#,
            digits,
        )
    }

    /// Build Plivo XML for saying something
    pub fn speak_xml(&self, text: &str) -> String {
        format!(
            r#"<Response>
    <Speak voice="{}" language="en-US">{}</Speak>
    <Wait length="60"/>
</Response>"#,
            self.config.tts_voice,
            xml_escape(text),
        )
    }

    /// Get the answer URL — uses custom answer_url if set, otherwise builds from webhook_base_url
    fn answer_url(&self) -> String {
        if let Some(ref url) = self.config.answer_url {
            url.clone()
        } else {
            format!("{}/voice/answer", self.config.webhook_base_url)
        }
    }

    /// Get the hangup URL
    fn hangup_url(&self) -> String {
        format!("{}/voice/hangup", self.config.webhook_base_url)
    }
}

/// Escape special XML characters
pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[async_trait]
impl VoiceCallProvider for PlivoProvider {
    async fn initiate_call(&self, to: &str, greeting_text: &str) -> Result<String> {
        let url = format!("{}/{}/Call/", PLIVO_API_BASE, self.config.auth_id);

        let answer_url = self.answer_url();
        let hangup_url = self.hangup_url();

        // Store greeting text for the answer webhook to use
        // For now, we use a simple approach: append greeting as query param
        let answer_url_with_greeting = format!(
            "{}?greeting={}",
            answer_url,
            urlencoding::encode(greeting_text)
        );

        let body = serde_json::json!({
            "from": self.config.from_number,
            "to": to,
            "answer_url": answer_url_with_greeting,
            "answer_method": "POST",
            "hangup_url": hangup_url,
            "hangup_method": "POST",
        });

        info!(
            "Initiating Plivo call to {} from {}",
            to, self.config.from_number
        );

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.config.auth_id, Some(&self.config.auth_token))
            .json(&body)
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Plivo API error: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(zeus_core::Error::Tool(format!(
                "Plivo call failed ({}): {}",
                status, body
            )));
        }

        let body: serde_json::Value = resp.json().await.map_err(|e| {
            zeus_core::Error::Tool(format!("Failed to parse Plivo response: {}", e))
        })?;

        let call_uuid = body
            .get("request_uuid")
            .and_then(|v| v.as_str())
            .or_else(|| {
                // Plivo sometimes returns request_uuid as first element in an array
                body.get("request_uuid")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.as_str())
            })
            .ok_or_else(|| zeus_core::Error::Tool("No call UUID in Plivo response".to_string()))?
            .to_string();

        info!("Plivo call initiated: {}", call_uuid);
        Ok(call_uuid)
    }

    async fn hangup_call(&self, call_id: &str) -> Result<()> {
        let url = format!(
            "{}/{}/Call/{}/",
            PLIVO_API_BASE, self.config.auth_id, call_id
        );

        let resp = self
            .http
            .delete(&url)
            .basic_auth(&self.config.auth_id, Some(&self.config.auth_token))
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Plivo hangup error: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(zeus_core::Error::Tool(format!(
                "Plivo hangup failed: {}",
                body
            )));
        }

        info!("Plivo call {} hung up", call_id);
        Ok(())
    }

    async fn play_tts(&self, call_id: &str, text: &str) -> Result<()> {
        let url = format!(
            "{}/{}/Call/{}/Speak/",
            PLIVO_API_BASE, self.config.auth_id, call_id
        );

        let body = serde_json::json!({
            "text": text,
            "voice": self.config.tts_voice,
            "language": "en-US",
        });

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.config.auth_id, Some(&self.config.auth_token))
            .json(&body)
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Plivo TTS error: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(zeus_core::Error::Tool(format!(
                "Plivo TTS failed: {}",
                body
            )));
        }

        debug!("Playing TTS on Plivo call {}: {}", call_id, text);
        Ok(())
    }

    async fn get_call_state(&self, call_id: &str) -> Result<CallState> {
        let url = format!(
            "{}/{}/Call/{}/",
            PLIVO_API_BASE, self.config.auth_id, call_id
        );

        let resp = self
            .http
            .get(&url)
            .basic_auth(&self.config.auth_id, Some(&self.config.auth_token))
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Plivo status error: {}", e)))?;

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Failed to parse status: {}", e)))?;

        let status = body
            .get("call_status")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");

        Ok(CallState::from_plivo_status(status))
    }

    async fn send_dtmf(&self, call_id: &str, digits: &str) -> Result<()> {
        let url = format!(
            "{}/{}/Call/{}/DTMF/",
            PLIVO_API_BASE, self.config.auth_id, call_id
        );

        let body = serde_json::json!({
            "digits": digits,
        });

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.config.auth_id, Some(&self.config.auth_token))
            .json(&body)
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Plivo DTMF error: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(zeus_core::Error::Tool(format!(
                "Plivo DTMF failed: {}",
                body
            )));
        }

        debug!("Sent DTMF digits '{}' on Plivo call {}", digits, call_id);
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "plivo"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> PlivoConfig {
        PlivoConfig {
            auth_id: "PLIVO_TEST_AUTH_ID".to_string(),
            auth_token: "test_auth_token".to_string(),
            from_number: "+15551234567".to_string(),
            webhook_base_url: "https://example.ngrok.io".to_string(),
            webhook_port: 8090,
            tts_voice: "WOMAN".to_string(),
            answer_url: None,
        }
    }

    // ---- PlivoConfig tests ----

    #[test]
    fn test_plivo_config_defaults() {
        let config = PlivoConfig::default();
        assert_eq!(config.webhook_port, 8090);
        assert_eq!(config.tts_voice, "WOMAN");
        assert!(config.auth_id.is_empty());
        assert!(config.auth_token.is_empty());
        assert!(config.from_number.is_empty());
        assert!(config.webhook_base_url.is_empty());
        assert!(config.answer_url.is_none());
    }

    #[test]
    fn test_plivo_config_serialization_roundtrip() {
        let config = PlivoConfig {
            auth_id: "PLIVO_AUTH_123".to_string(),
            auth_token: "secret_token".to_string(),
            from_number: "+15551234567".to_string(),
            webhook_base_url: "https://example.ngrok.io".to_string(),
            webhook_port: 9090,
            tts_voice: "MAN".to_string(),
            answer_url: Some("https://custom.example.com/answer".to_string()),
        };
        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        let deserialized: PlivoConfig =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(deserialized.auth_id, "PLIVO_AUTH_123");
        assert_eq!(deserialized.auth_token, "secret_token");
        assert_eq!(deserialized.from_number, "+15551234567");
        assert_eq!(deserialized.webhook_base_url, "https://example.ngrok.io");
        assert_eq!(deserialized.webhook_port, 9090);
        assert_eq!(deserialized.tts_voice, "MAN");
        assert_eq!(
            deserialized.answer_url,
            Some("https://custom.example.com/answer".to_string())
        );
    }

    #[test]
    fn test_plivo_config_deserialize_with_defaults() {
        let json = r#"{
            "auth_id": "PLIVO123",
            "auth_token": "tok",
            "from_number": "+1555",
            "webhook_base_url": "https://example.com"
        }"#;
        let config: PlivoConfig = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(config.auth_id, "PLIVO123");
        assert_eq!(config.webhook_port, 8090);
        assert_eq!(config.tts_voice, "WOMAN");
        assert!(config.answer_url.is_none());
    }

    #[test]
    fn test_plivo_config_from_env_defaults() {
        temp_env::with_vars(
            [
                ("PLIVO_AUTH_ID", None::<&str>),
                ("PLIVO_AUTH_TOKEN", None),
                ("PLIVO_PHONE_NUMBER", None),
            ],
            || {
                let config = PlivoConfig::from_env();
                assert!(config.auth_id.is_empty());
                assert!(config.auth_token.is_empty());
                assert!(config.from_number.is_empty());
                assert_eq!(config.webhook_port, 8090);
                assert_eq!(config.tts_voice, "WOMAN");
            },
        );
    }

    #[test]
    fn test_plivo_config_with_env_overrides() {
        temp_env::with_vars(
            [
                ("PLIVO_AUTH_ID", Some("ENV_AUTH_ID")),
                ("PLIVO_AUTH_TOKEN", Some("env_token_123")),
                ("PLIVO_PHONE_NUMBER", Some("+15559999999")),
            ],
            || {
                let config = PlivoConfig::default().with_env_overrides();
                assert_eq!(config.auth_id, "ENV_AUTH_ID");
                assert_eq!(config.auth_token, "env_token_123");
                assert_eq!(config.from_number, "+15559999999");
            },
        );
    }

    #[test]
    fn test_plivo_config_env_partial_override() {
        temp_env::with_vars(
            [
                ("PLIVO_AUTH_ID", None),
                ("PLIVO_AUTH_TOKEN", None),
                ("PLIVO_PHONE_NUMBER", Some("+15558888888")),
            ],
            || {
                let config = PlivoConfig {
                    auth_id: "AUTH_FROM_CONFIG".to_string(),
                    auth_token: "token_from_config".to_string(),
                    from_number: "+15550000000".to_string(),
                    webhook_base_url: "https://example.com".to_string(),
                    webhook_port: 9090,
                    tts_voice: "MAN".to_string(),
                    answer_url: None,
                }
                .with_env_overrides();

                // auth_id and auth_token should remain from config (env vars not set)
                assert_eq!(config.auth_id, "AUTH_FROM_CONFIG");
                assert_eq!(config.auth_token, "token_from_config");
                // from_number should be overridden by env var
                assert_eq!(config.from_number, "+15558888888");
                // Other fields should be unchanged
                assert_eq!(config.webhook_base_url, "https://example.com");
                assert_eq!(config.webhook_port, 9090);
                assert_eq!(config.tts_voice, "MAN");
            },
        );
    }

    // ---- Plivo status -> CallState mapping tests ----

    #[test]
    fn test_from_plivo_status_ringing() {
        assert_eq!(CallState::from_plivo_status("ringing"), CallState::Ringing);
    }

    #[test]
    fn test_from_plivo_status_in_progress() {
        assert_eq!(
            CallState::from_plivo_status("in-progress"),
            CallState::Active
        );
    }

    #[test]
    fn test_from_plivo_status_answered() {
        assert_eq!(
            CallState::from_plivo_status("answered"),
            CallState::Answered
        );
    }

    #[test]
    fn test_from_plivo_status_completed() {
        assert_eq!(
            CallState::from_plivo_status("completed"),
            CallState::Completed
        );
    }

    #[test]
    fn test_from_plivo_status_busy() {
        assert_eq!(CallState::from_plivo_status("busy"), CallState::NoAnswer);
    }

    #[test]
    fn test_from_plivo_status_failed() {
        assert_eq!(
            CallState::from_plivo_status("failed"),
            CallState::Failed("Plivo reported failure".to_string())
        );
    }

    #[test]
    fn test_from_plivo_status_timeout() {
        assert_eq!(CallState::from_plivo_status("timeout"), CallState::NoAnswer);
    }

    #[test]
    fn test_from_plivo_status_no_answer() {
        assert_eq!(
            CallState::from_plivo_status("no-answer"),
            CallState::NoAnswer
        );
    }

    #[test]
    fn test_from_plivo_status_cancel() {
        assert_eq!(CallState::from_plivo_status("cancel"), CallState::Cancelled);
    }

    #[test]
    fn test_from_plivo_status_unknown() {
        assert_eq!(
            CallState::from_plivo_status("something-else"),
            CallState::Unknown
        );
    }

    // ---- PlivoProvider tests ----

    #[test]
    fn test_provider_name() {
        let provider = PlivoProvider::new(test_config());
        assert_eq!(provider.provider_name(), "plivo");
    }

    #[test]
    fn test_xml_escape_basic() {
        assert_eq!(xml_escape("hello"), "hello");
    }

    #[test]
    fn test_xml_escape_special_chars() {
        assert_eq!(xml_escape("a & b"), "a &amp; b");
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(xml_escape(r#"he said "hi""#), "he said &quot;hi&quot;");
        assert_eq!(xml_escape("it's"), "it&apos;s");
    }

    #[test]
    fn test_xml_escape_all_special_chars() {
        assert_eq!(
            xml_escape(r#"<a & "b" 'c'>"#),
            "&lt;a &amp; &quot;b&quot; &apos;c&apos;&gt;"
        );
    }

    #[test]
    fn test_greeting_xml_structure() {
        let provider = PlivoProvider::new(test_config());
        let xml = provider.greeting_xml("Hello, this is Zeus calling.");

        assert!(xml.contains("<Response>"));
        assert!(xml.contains("</Response>"));
        assert!(xml.contains("<Speak"));
        assert!(xml.contains("Hello, this is Zeus calling."));
        assert!(xml.contains("<Wait"));
        assert!(xml.contains(r#"voice="WOMAN""#));
        assert!(xml.contains(r#"language="en-US""#));
    }

    #[test]
    fn test_greeting_xml_escapes_text() {
        let provider = PlivoProvider::new(test_config());
        let xml = provider.greeting_xml("Hello & goodbye <friend>");

        assert!(xml.contains("Hello &amp; goodbye &lt;friend&gt;"));
    }

    #[test]
    fn test_speak_xml_structure() {
        let provider = PlivoProvider::new(test_config());
        let xml = provider.speak_xml("Please hold on.");

        assert!(xml.contains("<Response>"));
        assert!(xml.contains("</Response>"));
        assert!(xml.contains("<Speak"));
        assert!(xml.contains("Please hold on."));
        assert!(xml.contains("<Wait"));
        assert!(xml.contains(r#"voice="WOMAN""#));
        assert!(xml.contains(r#"language="en-US""#));
    }

    #[test]
    fn test_speak_xml_escapes_text() {
        let provider = PlivoProvider::new(test_config());
        let xml = provider.speak_xml("A & B's \"deal\"");

        assert!(xml.contains("A &amp; B&apos;s &quot;deal&quot;"));
    }

    #[test]
    fn test_speak_xml_with_custom_voice() {
        let mut config = test_config();
        config.tts_voice = "MAN".to_string();
        let provider = PlivoProvider::new(config);
        let xml = provider.speak_xml("Hi");

        assert!(xml.contains(r#"voice="MAN""#));
    }

    #[test]
    fn test_dtmf_xml_structure() {
        let provider = PlivoProvider::new(test_config());
        let xml = provider.dtmf_xml("1234");

        assert!(xml.contains("<Response>"));
        assert!(xml.contains("</Response>"));
        assert!(xml.contains("<DTMF>1234</DTMF>"));
        assert!(xml.contains("<Wait"));
    }

    #[test]
    fn test_dtmf_xml_valid_digits() {
        let provider = PlivoProvider::new(test_config());

        // Test with standard phone digits
        let xml = provider.dtmf_xml("0123456789*#");
        assert!(xml.contains("<DTMF>0123456789*#</DTMF>"));

        // Test single digit
        let xml = provider.dtmf_xml("5");
        assert!(xml.contains("<DTMF>5</DTMF>"));

        // Test empty string
        let xml = provider.dtmf_xml("");
        assert!(xml.contains("<DTMF></DTMF>"));
    }

    #[test]
    fn test_dtmf_xml_special_chars() {
        let provider = PlivoProvider::new(test_config());

        // Test with 'w' (wait/pause character in DTMF)
        let xml = provider.dtmf_xml("ww1234");
        assert!(xml.contains("<DTMF>ww1234</DTMF>"));

        // Test with 'W' (longer wait)
        let xml = provider.dtmf_xml("W1234");
        assert!(xml.contains("<DTMF>W1234</DTMF>"));

        // Test mixed waits and digits
        let xml = provider.dtmf_xml("1w2w3w4");
        assert!(xml.contains("<DTMF>1w2w3w4</DTMF>"));
    }

    #[test]
    fn test_answer_url_default() {
        let provider = PlivoProvider::new(test_config());
        assert_eq!(
            provider.answer_url(),
            "https://example.ngrok.io/voice/answer"
        );
    }

    #[test]
    fn test_answer_url_custom() {
        let mut config = test_config();
        config.answer_url = Some("https://custom.example.com/my-answer".to_string());
        let provider = PlivoProvider::new(config);
        assert_eq!(
            provider.answer_url(),
            "https://custom.example.com/my-answer"
        );
    }

    #[test]
    fn test_hangup_url() {
        let provider = PlivoProvider::new(test_config());
        assert_eq!(
            provider.hangup_url(),
            "https://example.ngrok.io/voice/hangup"
        );
    }
}
