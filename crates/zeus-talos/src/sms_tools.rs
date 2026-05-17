//! Twilio SMS API tools
//!
//! Provides tools for sending and retrieving SMS messages via the Twilio API.
//! Each tool accepts optional `account_sid` / `auth_token` / `from` parameters,
//! falling back to `TWILIO_ACCOUNT_SID`, `TWILIO_AUTH_TOKEN`, and
//! `TWILIO_PHONE_NUMBER` environment variables respectively.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

const TWILIO_API: &str = "https://api.twilio.com/2010-04-01";

/// Get Twilio Account SID from args or environment
fn get_account_sid(args: &Value) -> Result<String> {
    if let Some(sid) = args.get("account_sid").and_then(|v| v.as_str()) {
        return Ok(sid.to_string());
    }
    std::env::var("TWILIO_ACCOUNT_SID").map_err(|_| {
        Error::Tool(
            "Missing 'account_sid' parameter and TWILIO_ACCOUNT_SID env var not set".to_string(),
        )
    })
}

/// Get Twilio Auth Token from args or environment
fn get_auth_token(args: &Value) -> Result<String> {
    if let Some(token) = args.get("auth_token").and_then(|v| v.as_str()) {
        return Ok(token.to_string());
    }
    std::env::var("TWILIO_AUTH_TOKEN").map_err(|_| {
        Error::Tool(
            "Missing 'auth_token' parameter and TWILIO_AUTH_TOKEN env var not set".to_string(),
        )
    })
}

/// Get the sender phone number from args or environment
fn get_from_number(args: &Value) -> Result<String> {
    if let Some(from) = args.get("from").and_then(|v| v.as_str()) {
        return Ok(from.to_string());
    }
    std::env::var("TWILIO_PHONE_NUMBER").map_err(|_| {
        Error::Tool("Missing 'from' parameter and TWILIO_PHONE_NUMBER env var not set".to_string())
    })
}

/// Make a Twilio API request (GET or form-encoded POST)
async fn twilio_api(
    account_sid: &str,
    auth_token: &str,
    method: &str,
    endpoint: &str,
    form: Option<&[(&str, &str)]>,
) -> Result<Value> {
    let url = format!("{}{}", TWILIO_API, endpoint);
    let client = reqwest::Client::new();

    let req = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        _ => return Err(Error::Tool(format!("Unsupported method: {}", method))),
    };

    let mut req = req.basic_auth(account_sid, Some(auth_token));

    if let Some(params) = form {
        req = req.form(params);
    }

    let response = req
        .send()
        .await
        .map_err(|e| Error::Tool(format!("Twilio API request failed: {}", e)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read response: {}", e)))?;

    if !status.is_success() {
        return Err(Error::Tool(format!(
            "Twilio API error {}: {}",
            status, text
        )));
    }

    if text.is_empty() {
        return Ok(json!({"ok": true}));
    }

    serde_json::from_str(&text).map_err(|e| {
        Error::Tool(format!(
            "Invalid JSON: {} (body: {})",
            e,
            &text[..zeus_core::floor_char_boundary(&text, 200)]
        ))
    })
}

// ---------------------------------------------------------------------------
// 1. SmsSendMessageTool
// ---------------------------------------------------------------------------

/// Send an SMS message via Twilio
pub struct SmsSendMessageTool;

#[async_trait]
impl TalosTool for SmsSendMessageTool {
    fn name(&self) -> &'static str {
        "sms_send_message"
    }
    fn description(&self) -> &'static str {
        "Send an SMS message via Twilio"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "to",
                "string",
                "Destination phone number in E.164 format (e.g. +15551234567)",
                true,
            )
            .with_param("body", "string", "SMS message text", true)
            .with_param(
                "from",
                "string",
                "Sender phone number (or set TWILIO_PHONE_NUMBER env var)",
                false,
            )
            .with_param(
                "account_sid",
                "string",
                "Twilio Account SID (or set TWILIO_ACCOUNT_SID env var)",
                false,
            )
            .with_param(
                "auth_token",
                "string",
                "Twilio Auth Token (or set TWILIO_AUTH_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let account_sid = get_account_sid(&args)?;
        let auth_token = get_auth_token(&args)?;
        let from = get_from_number(&args)?;
        let to = args
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'to'".to_string()))?;
        let body = args
            .get("body")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'body'".to_string()))?;

        let endpoint = format!("/Accounts/{}/Messages.json", account_sid);
        let form_params = [("To", to), ("From", from.as_str()), ("Body", body)];
        let result = twilio_api(
            &account_sid,
            &auth_token,
            "POST",
            &endpoint,
            Some(&form_params),
        )
        .await?;

        let sid = result
            .get("sid")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let status = result
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        Ok(format!("SMS sent (sid: {}, status: {})", sid, status))
    }
}

// ---------------------------------------------------------------------------
// 2. SmsGetMessagesTool
// ---------------------------------------------------------------------------

/// Get recent SMS messages from Twilio
pub struct SmsGetMessagesTool;

#[async_trait]
impl TalosTool for SmsGetMessagesTool {
    fn name(&self) -> &'static str {
        "sms_get_messages"
    }
    fn description(&self) -> &'static str {
        "Get recent SMS messages from Twilio"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "limit",
                "integer",
                "Max messages to return (1-100, default 10)",
                false,
            )
            .with_param(
                "account_sid",
                "string",
                "Twilio Account SID (or set TWILIO_ACCOUNT_SID env var)",
                false,
            )
            .with_param(
                "auth_token",
                "string",
                "Twilio Auth Token (or set TWILIO_AUTH_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let account_sid = get_account_sid(&args)?;
        let auth_token = get_auth_token(&args)?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(100);

        let endpoint = format!("/Accounts/{}/Messages.json?PageSize={}", account_sid, limit);
        let result = twilio_api(&account_sid, &auth_token, "GET", &endpoint, None).await?;

        let messages = result
            .get("messages")
            .and_then(|v| v.as_array())
            .map(|arr| arr.as_slice())
            .unwrap_or(&[]);

        if messages.is_empty() {
            return Ok("No messages found.".to_string());
        }

        let mut output = format!("{} message(s):\n", messages.len());
        for msg in messages {
            let sid = msg.get("sid").and_then(|v| v.as_str()).unwrap_or("?");
            let from = msg.get("from").and_then(|v| v.as_str()).unwrap_or("?");
            let to = msg.get("to").and_then(|v| v.as_str()).unwrap_or("?");
            let body = msg
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("[no text]");
            let status = msg.get("status").and_then(|v| v.as_str()).unwrap_or("?");
            output.push_str(&format!(
                "[{}] {} -> {}: {} ({})\n",
                sid, from, to, body, status
            ));
        }
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 3. SmsGetMessageTool
// ---------------------------------------------------------------------------

/// Get a single SMS message by SID from Twilio
pub struct SmsGetMessageTool;

#[async_trait]
impl TalosTool for SmsGetMessageTool {
    fn name(&self) -> &'static str {
        "sms_get_message"
    }
    fn description(&self) -> &'static str {
        "Get details of a single SMS message by SID"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "message_sid",
                "string",
                "Twilio Message SID (e.g. SMxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx)",
                true,
            )
            .with_param(
                "account_sid",
                "string",
                "Twilio Account SID (or set TWILIO_ACCOUNT_SID env var)",
                false,
            )
            .with_param(
                "auth_token",
                "string",
                "Twilio Auth Token (or set TWILIO_AUTH_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let account_sid = get_account_sid(&args)?;
        let auth_token = get_auth_token(&args)?;
        let message_sid = args
            .get("message_sid")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'message_sid'".to_string()))?;

        let endpoint = format!("/Accounts/{}/Messages/{}.json", account_sid, message_sid);
        let result = twilio_api(&account_sid, &auth_token, "GET", &endpoint, None).await?;

        let sid = result.get("sid").and_then(|v| v.as_str()).unwrap_or("?");
        let from = result.get("from").and_then(|v| v.as_str()).unwrap_or("?");
        let to = result.get("to").and_then(|v| v.as_str()).unwrap_or("?");
        let body = result
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("[no text]");
        let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        let direction = result
            .get("direction")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let date_sent = result
            .get("date_sent")
            .and_then(|v| v.as_str())
            .unwrap_or("?");

        let output = format!(
            "Message: {}\n  From: {}\n  To: {}\n  Body: {}\n  Status: {}\n  Direction: {}\n  Date sent: {}",
            sid, from, to, body, status, direction, date_sent
        );
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_message_schema() {
        let tool = SmsSendMessageTool;
        assert_eq!(tool.name(), "sms_send_message");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"to"));
        assert!(names.contains(&"body"));
    }

    #[test]
    fn test_get_messages_schema() {
        let tool = SmsGetMessagesTool;
        assert_eq!(tool.name(), "sms_get_messages");
    }

    #[test]
    fn test_get_message_schema() {
        let tool = SmsGetMessageTool;
        assert_eq!(tool.name(), "sms_get_message");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"message_sid"));
    }

    #[test]
    fn test_get_account_sid_from_args() {
        let args = json!({"account_sid": "AC1234567890"});
        let sid = get_account_sid(&args).expect("should succeed");
        assert_eq!(sid, "AC1234567890");
    }

    #[test]
    fn test_get_auth_token_from_args() {
        let args = json!({"auth_token": "test-token"});
        let token = get_auth_token(&args).expect("should succeed");
        assert_eq!(token, "test-token");
    }
}
