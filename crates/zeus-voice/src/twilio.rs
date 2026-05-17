//! Twilio voice call provider

use async_trait::async_trait;
use tracing::{debug, info};
use zeus_core::Result;

use crate::VoiceConfig;
use crate::call::CallState;
use crate::provider::VoiceCallProvider;

/// Twilio REST API base URL
const TWILIO_API_BASE: &str = "https://api.twilio.com/2010-04-01";

/// Twilio voice call provider
pub struct TwilioProvider {
    config: VoiceConfig,
    http: reqwest::Client,
}

impl TwilioProvider {
    pub fn new(config: VoiceConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    /// Build the TwiML for initial call with greeting
    fn greeting_twiml(&self, text: &str) -> String {
        // TwiML: Say the greeting, then connect to a WebSocket stream for bidirectional audio
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Response>
    <Say voice="{}">{}</Say>
    <Connect>
        <Stream url="wss://{}/voice/media-stream" />
    </Connect>
</Response>"#,
            self.config.tts_voice,
            xml_escape(text),
            self.config
                .webhook_base_url
                .trim_start_matches("https://")
                .trim_start_matches("http://"),
        )
    }

    /// Build TwiML for sending DTMF tones
    fn dtmf_twiml(&self, digits: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Response>
    <Play digits="{}"/>
    <Pause length="60"/>
</Response>"#,
            digits,
        )
    }

    /// Build TwiML for saying something
    fn say_twiml(&self, text: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Response>
    <Say voice="{}">{}</Say>
    <Pause length="60"/>
</Response>"#,
            self.config.tts_voice,
            xml_escape(text),
        )
    }
}

/// Escape special XML characters
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[async_trait]
impl VoiceCallProvider for TwilioProvider {
    async fn initiate_call(&self, to: &str, greeting_text: &str) -> Result<String> {
        let url = format!(
            "{}/Accounts/{}/Calls.json",
            TWILIO_API_BASE, self.config.account_sid
        );

        let twiml = self.greeting_twiml(greeting_text);

        let params = [
            ("To", to),
            ("From", self.config.from_number.as_str()),
            ("Twiml", twiml.as_str()),
            (
                "StatusCallback",
                &format!("{}/voice/status", self.config.webhook_base_url),
            ),
            (
                "StatusCallbackEvent",
                "initiated ringing answered completed",
            ),
        ];

        info!("Initiating call to {} from {}", to, self.config.from_number);

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.config.account_sid, Some(&self.config.auth_token))
            .form(&params)
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Twilio API error: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(zeus_core::Error::Tool(format!(
                "Twilio call failed ({}): {}",
                status, body
            )));
        }

        let body: serde_json::Value = resp.json().await.map_err(|e| {
            zeus_core::Error::Tool(format!("Failed to parse Twilio response: {}", e))
        })?;

        let call_sid = body
            .get("sid")
            .and_then(|s| s.as_str())
            .ok_or_else(|| zeus_core::Error::Tool("No call SID in Twilio response".to_string()))?
            .to_string();

        info!("Call initiated: {}", call_sid);
        Ok(call_sid)
    }

    async fn hangup_call(&self, call_id: &str) -> Result<()> {
        let url = format!(
            "{}/Accounts/{}/Calls/{}.json",
            TWILIO_API_BASE, self.config.account_sid, call_id
        );

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.config.account_sid, Some(&self.config.auth_token))
            .form(&[("Status", "completed")])
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Twilio hangup error: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(zeus_core::Error::Tool(format!(
                "Twilio hangup failed: {}",
                body
            )));
        }

        info!("Call {} hung up", call_id);
        Ok(())
    }

    async fn play_tts(&self, call_id: &str, text: &str) -> Result<()> {
        let url = format!(
            "{}/Accounts/{}/Calls/{}.json",
            TWILIO_API_BASE, self.config.account_sid, call_id
        );

        let twiml = self.say_twiml(text);

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.config.account_sid, Some(&self.config.auth_token))
            .form(&[("Twiml", &twiml)])
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Twilio TTS error: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(zeus_core::Error::Tool(format!(
                "Twilio TTS failed: {}",
                body
            )));
        }

        debug!("Playing TTS on call {}: {}", call_id, text);
        Ok(())
    }

    async fn get_call_state(&self, call_id: &str) -> Result<CallState> {
        let url = format!(
            "{}/Accounts/{}/Calls/{}.json",
            TWILIO_API_BASE, self.config.account_sid, call_id
        );

        let resp = self
            .http
            .get(&url)
            .basic_auth(&self.config.account_sid, Some(&self.config.auth_token))
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Twilio status error: {}", e)))?;

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Failed to parse status: {}", e)))?;

        let status = body
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");

        Ok(CallState::from_twilio_status(status))
    }

    async fn send_dtmf(&self, call_id: &str, digits: &str) -> Result<()> {
        let url = format!(
            "{}/Accounts/{}/Calls/{}.json",
            TWILIO_API_BASE, self.config.account_sid, call_id
        );

        let twiml = self.dtmf_twiml(digits);

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.config.account_sid, Some(&self.config.auth_token))
            .form(&[("Twiml", &twiml)])
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Twilio DTMF error: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(zeus_core::Error::Tool(format!(
                "Twilio DTMF failed: {}",
                body
            )));
        }

        debug!("Sent DTMF digits '{}' on call {}", digits, call_id);
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "twilio"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> VoiceConfig {
        VoiceConfig {
            account_sid: "AC_TEST_SID".to_string(),
            auth_token: "test_auth_token".to_string(),
            from_number: "+15551234567".to_string(),
            webhook_base_url: "https://example.ngrok.io".to_string(),
            webhook_port: 8090,
            tts_voice: "Polly.Amy".to_string(),
            ..Default::default()
        }
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
    fn test_greeting_twiml_structure() {
        let provider = TwilioProvider::new(test_config());
        let twiml = provider.greeting_twiml("Hello, this is Zeus calling.");

        assert!(twiml.contains(r#"<?xml version="1.0" encoding="UTF-8"?>"#));
        assert!(twiml.contains("<Response>"));
        assert!(twiml.contains("</Response>"));
        assert!(twiml.contains("<Say"));
        assert!(twiml.contains("Hello, this is Zeus calling."));
        assert!(twiml.contains("<Connect>"));
        assert!(twiml.contains("<Stream"));
        assert!(twiml.contains("wss://example.ngrok.io/voice/media-stream"));
        assert!(twiml.contains(r#"voice="Polly.Amy""#));
    }

    #[test]
    fn test_greeting_twiml_escapes_text() {
        let provider = TwilioProvider::new(test_config());
        let twiml = provider.greeting_twiml("Hello & goodbye <friend>");

        assert!(twiml.contains("Hello &amp; goodbye &lt;friend&gt;"));
    }

    #[test]
    fn test_say_twiml_structure() {
        let provider = TwilioProvider::new(test_config());
        let twiml = provider.say_twiml("Please hold on.");

        assert!(twiml.contains(r#"<?xml version="1.0" encoding="UTF-8"?>"#));
        assert!(twiml.contains("<Response>"));
        assert!(twiml.contains("</Response>"));
        assert!(twiml.contains("<Say"));
        assert!(twiml.contains("Please hold on."));
        assert!(twiml.contains("<Pause"));
        assert!(twiml.contains(r#"voice="Polly.Amy""#));
    }

    #[test]
    fn test_say_twiml_escapes_text() {
        let provider = TwilioProvider::new(test_config());
        let twiml = provider.say_twiml("A & B's \"deal\"");

        assert!(twiml.contains("A &amp; B&apos;s &quot;deal&quot;"));
    }

    #[test]
    fn test_greeting_twiml_strips_https_prefix() {
        let mut config = test_config();
        config.webhook_base_url = "https://my-tunnel.ngrok.io".to_string();
        let provider = TwilioProvider::new(config);
        let twiml = provider.greeting_twiml("Hi");

        assert!(twiml.contains("wss://my-tunnel.ngrok.io/voice/media-stream"));
        assert!(!twiml.contains("wss://https://"));
    }

    #[test]
    fn test_greeting_twiml_strips_http_prefix() {
        let mut config = test_config();
        config.webhook_base_url = "http://localhost:8090".to_string();
        let provider = TwilioProvider::new(config);
        let twiml = provider.greeting_twiml("Hi");

        assert!(twiml.contains("wss://localhost:8090/voice/media-stream"));
    }

    #[test]
    fn test_provider_name() {
        let provider = TwilioProvider::new(test_config());
        assert_eq!(provider.provider_name(), "twilio");
    }

    #[test]
    fn test_dtmf_twiml_structure() {
        let provider = TwilioProvider::new(test_config());
        let twiml = provider.dtmf_twiml("1234");

        assert!(twiml.contains(r#"<?xml version="1.0" encoding="UTF-8"?>"#));
        assert!(twiml.contains("<Response>"));
        assert!(twiml.contains("</Response>"));
        assert!(twiml.contains(r#"<Play digits="1234"/>"#));
        assert!(twiml.contains("<Pause"));
    }

    #[test]
    fn test_dtmf_twiml_valid_digits() {
        let provider = TwilioProvider::new(test_config());

        // Test with standard phone digits
        let twiml = provider.dtmf_twiml("0123456789*#");
        assert!(twiml.contains(r#"<Play digits="0123456789*#"/>"#));

        // Test single digit
        let twiml = provider.dtmf_twiml("5");
        assert!(twiml.contains(r#"<Play digits="5"/>"#));

        // Test empty string
        let twiml = provider.dtmf_twiml("");
        assert!(twiml.contains(r#"<Play digits=""/>"#));
    }

    #[test]
    fn test_dtmf_twiml_special_chars() {
        let provider = TwilioProvider::new(test_config());

        // Test with 'w' (wait/pause character in Twilio DTMF)
        let twiml = provider.dtmf_twiml("ww1234");
        assert!(twiml.contains(r#"<Play digits="ww1234"/>"#));

        // Test with 'W' (longer wait)
        let twiml = provider.dtmf_twiml("W1234");
        assert!(twiml.contains(r#"<Play digits="W1234"/>"#));

        // Test mixed waits and digits
        let twiml = provider.dtmf_twiml("1w2w3w4");
        assert!(twiml.contains(r#"<Play digits="1w2w3w4"/>"#));
    }
}
