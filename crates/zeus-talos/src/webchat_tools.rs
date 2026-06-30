//! WebChat HTTP API tools
//!
//! Provides tools for interacting with a generic webhook-based chat server.
//! Each tool accepts optional `url` and `api_key` parameters, falling back to
//! `WEBCHAT_URL` and `WEBCHAT_API_KEY` environment variables.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

/// Get webchat base URL from args or environment
fn get_url(args: &Value) -> Result<String> {
    if let Some(url) = args.get("url").and_then(|v| v.as_str()) {
        return Ok(url.trim_end_matches('/').to_string());
    }
    std::env::var("WEBCHAT_URL")
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|_| {
            Error::Tool("Missing 'url' parameter and WEBCHAT_URL env var not set".to_string())
        })
}

/// Get optional API key from args or environment
fn get_api_key(args: &Value) -> Option<String> {
    if let Some(key) = args.get("api_key").and_then(|v| v.as_str()) {
        return Some(key.to_string());
    }
    std::env::var("WEBCHAT_API_KEY").ok()
}

/// Make a WebChat API request
async fn webchat_api(
    base_url: &str,
    api_key: Option<&str>,
    method: &str,
    endpoint: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("{}{}", base_url, endpoint);
    let client = reqwest::Client::new();

    let mut req = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        _ => return Err(Error::Tool(format!("Unsupported method: {}", method))),
    };

    req = req.header("Content-Type", "application/json");
    if let Some(key) = api_key {
        req = req.header("Authorization", format!("Bearer {}", key));
    }

    if let Some(b) = body {
        req = req.json(b);
    }

    let response = req
        .send()
        .await
        .map_err(|e| Error::Tool(format!("WebChat API request failed: {}", e)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read response: {}", e)))?;

    if !status.is_success() {
        return Err(Error::Tool(format!(
            "WebChat API error {}: {}",
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
// 1. WebchatSendMessageTool
// ---------------------------------------------------------------------------

/// Send a text message to a webchat channel
pub struct WebchatSendMessageTool;

#[async_trait]
impl TalosTool for WebchatSendMessageTool {
    fn name(&self) -> &'static str {
        "webchat_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a text message to a webchat channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "channel_id",
                "string",
                "Channel ID to send the message to",
                true,
            )
            .with_param("text", "string", "Message text content", true)
            .with_param("sender", "string", "Sender name or identifier", true)
            .with_param(
                "url",
                "string",
                "WebChat server base URL (or set WEBCHAT_URL env var)",
                false,
            )
            .with_param(
                "api_key",
                "string",
                "API key for auth (or set WEBCHAT_API_KEY env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_url(&args)?;
        let api_key = get_api_key(&args);
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'text'".to_string()))?;
        let sender = args
            .get("sender")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'sender'".to_string()))?;

        let body = json!({
            "channel_id": channel_id,
            "text": text,
            "sender": sender
        });
        let result = webchat_api(
            &base_url,
            api_key.as_deref(),
            "POST",
            "/api/messages",
            Some(&body),
        )
        .await?;

        let msg_id = result
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        Ok(format!("Message sent (id: {})", msg_id))
    }
}

// ---------------------------------------------------------------------------
// 2. WebchatGetMessagesTool
// ---------------------------------------------------------------------------

/// Get recent messages from a webchat channel
pub struct WebchatGetMessagesTool;

#[async_trait]
impl TalosTool for WebchatGetMessagesTool {
    fn name(&self) -> &'static str {
        "webchat_get_messages"
    }
    fn description(&self) -> &'static str {
        "Get recent messages from a webchat channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "channel_id",
                "string",
                "Channel ID to fetch messages from",
                true,
            )
            .with_param(
                "limit",
                "integer",
                "Max messages to return (default 20)",
                false,
            )
            .with_param(
                "url",
                "string",
                "WebChat server base URL (or set WEBCHAT_URL env var)",
                false,
            )
            .with_param(
                "api_key",
                "string",
                "API key for auth (or set WEBCHAT_API_KEY env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_url(&args)?;
        let api_key = get_api_key(&args);
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);

        let endpoint = format!("/api/messages?channel_id={}&limit={}", channel_id, limit);
        let result = webchat_api(&base_url, api_key.as_deref(), "GET", &endpoint, None).await?;

        let messages = result.as_array().map(|arr| arr.as_slice()).unwrap_or(&[]);
        if messages.is_empty() {
            return Ok("No messages found.".to_string());
        }

        let mut output = format!("{} message(s):\n", messages.len());
        for msg in messages {
            let sender = msg
                .get("sender")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let text = msg
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("[no text]");
            let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            output.push_str(&format!("[{}] {}: {}\n", id, sender, text));
        }
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 3. WebchatListChannelsTool
// ---------------------------------------------------------------------------

/// List available webchat channels
pub struct WebchatListChannelsTool;

#[async_trait]
impl TalosTool for WebchatListChannelsTool {
    fn name(&self) -> &'static str {
        "webchat_list_channels"
    }
    fn description(&self) -> &'static str {
        "List available webchat channels"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "url",
                "string",
                "WebChat server base URL (or set WEBCHAT_URL env var)",
                false,
            )
            .with_param(
                "api_key",
                "string",
                "API key for auth (or set WEBCHAT_API_KEY env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_url(&args)?;
        let api_key = get_api_key(&args);

        let result =
            webchat_api(&base_url, api_key.as_deref(), "GET", "/api/channels", None).await?;

        let channels = result.as_array().map(|arr| arr.as_slice()).unwrap_or(&[]);
        if channels.is_empty() {
            return Ok("No channels found.".to_string());
        }

        let mut output = format!("{} channel(s):\n", channels.len());
        for ch in channels {
            let name = ch.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let id = ch.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let members = ch.get("members").and_then(|v| v.as_i64()).unwrap_or(0);
            output.push_str(&format!("  #{} ({}) — {} members\n", name, id, members));
        }
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
        let tool = WebchatSendMessageTool;
        assert_eq!(tool.name(), "webchat_send_message");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"channel_id"));
        assert!(names.contains(&"text"));
        assert!(names.contains(&"sender"));
    }

    #[test]
    fn test_get_messages_schema() {
        let tool = WebchatGetMessagesTool;
        assert_eq!(tool.name(), "webchat_get_messages");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"channel_id"));
    }

    #[test]
    fn test_list_channels_schema() {
        let tool = WebchatListChannelsTool;
        assert_eq!(tool.name(), "webchat_list_channels");
    }

    #[test]
    fn test_get_url_from_args() {
        let args = json!({"url": "http://localhost:8080"});
        let url = get_url(&args).expect("should succeed");
        assert_eq!(url, "http://localhost:8080");
    }

    #[test]
    fn test_get_url_strips_trailing_slash() {
        let args = json!({"url": "http://localhost:8080/"});
        let url = get_url(&args).expect("should succeed");
        assert_eq!(url, "http://localhost:8080");
    }

    #[test]
    fn test_get_api_key_from_args() {
        let args = json!({"api_key": "test-key-123"});
        let key = get_api_key(&args);
        assert_eq!(key, Some("test-key-123".to_string()));
    }

    #[test]
    fn test_get_api_key_none_when_missing() {
        let args = json!({});
        // Only None if env var also not set — clear it to be safe
        unsafe {
            std::env::remove_var("WEBCHAT_API_KEY");
        }
        let key = get_api_key(&args);
        assert_eq!(key, None);
    }
}
