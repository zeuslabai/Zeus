//! Google Chat API tools
//!
//! Provides tools for interacting with the Google Chat API.
//! Each tool accepts an optional `token` parameter, falling back to the
//! `GOOGLE_CHAT_TOKEN` environment variable (service account or OAuth Bearer token).

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

const GOOGLE_CHAT_API: &str = "https://chat.googleapis.com/v1";

/// Get Google Chat Bearer token from args or environment
fn get_token(args: &Value) -> Result<String> {
    if let Some(token) = args.get("token").and_then(|v| v.as_str()) {
        return Ok(token.to_string());
    }
    std::env::var("GOOGLE_CHAT_TOKEN").map_err(|_| {
        Error::Tool("Missing 'token' parameter and GOOGLE_CHAT_TOKEN env var not set".to_string())
    })
}

/// Make a Google Chat API request
async fn googlechat_api(
    token: &str,
    method: &str,
    endpoint: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("{}{}", GOOGLE_CHAT_API, endpoint);
    let client = reqwest::Client::new();

    let mut req = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        "PATCH" => client.patch(&url),
        _ => return Err(Error::Tool(format!("Unsupported method: {}", method))),
    };

    req = req
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json");

    if let Some(b) = body {
        req = req.json(b);
    }

    let response = req
        .send()
        .await
        .map_err(|e| Error::Tool(format!("Google Chat API request failed: {}", e)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read response: {}", e)))?;

    if !status.is_success() {
        return Err(Error::Tool(format!(
            "Google Chat API error {}: {}",
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
// 1. GoogleChatSendMessageTool
// ---------------------------------------------------------------------------

/// Send a text message to a Google Chat space
pub struct GoogleChatSendMessageTool;

#[async_trait]
impl TalosTool for GoogleChatSendMessageTool {
    fn name(&self) -> &'static str {
        "googlechat_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a text message to a Google Chat space"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "space",
                "string",
                "Google Chat space name (e.g. spaces/AAAA1234)",
                true,
            )
            .with_param("text", "string", "Message text content", true)
            .with_param(
                "token",
                "string",
                "Bearer token (or set GOOGLE_CHAT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let space = args
            .get("space")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'space'".to_string()))?;
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'text'".to_string()))?;

        let body = json!({ "text": text });
        let endpoint = format!("/{}/messages", space);
        let result = googlechat_api(&token, "POST", &endpoint, Some(&body)).await?;

        let name = result
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        Ok(format!("Message sent (name: {})", name))
    }
}

// ---------------------------------------------------------------------------
// 2. GoogleChatGetMessagesTool
// ---------------------------------------------------------------------------

/// Get recent messages from a Google Chat space
pub struct GoogleChatGetMessagesTool;

#[async_trait]
impl TalosTool for GoogleChatGetMessagesTool {
    fn name(&self) -> &'static str {
        "googlechat_get_messages"
    }
    fn description(&self) -> &'static str {
        "Get recent messages from a Google Chat space"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "space",
                "string",
                "Google Chat space name (e.g. spaces/AAAA1234)",
                true,
            )
            .with_param(
                "limit",
                "integer",
                "Max messages to return (1-100, default 10)",
                false,
            )
            .with_param(
                "token",
                "string",
                "Bearer token (or set GOOGLE_CHAT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let space = args
            .get("space")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'space'".to_string()))?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(100);

        let endpoint = format!("/{}/messages?pageSize={}", space, limit);
        let result = googlechat_api(&token, "GET", &endpoint, None).await?;

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
            let name = msg.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let sender_name = msg
                .get("sender")
                .and_then(|s| s.get("displayName"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");
            let text = msg
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("[no text]");
            let create_time = msg
                .get("createTime")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            output.push_str(&format!(
                "[{}] {} ({}): {}\n",
                name, sender_name, create_time, text
            ));
        }
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 3. GoogleChatListSpacesTool
// ---------------------------------------------------------------------------

/// List Google Chat spaces accessible to the authenticated user
pub struct GoogleChatListSpacesTool;

#[async_trait]
impl TalosTool for GoogleChatListSpacesTool {
    fn name(&self) -> &'static str {
        "googlechat_list_spaces"
    }
    fn description(&self) -> &'static str {
        "List available Google Chat spaces"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "token",
            "string",
            "Bearer token (or set GOOGLE_CHAT_TOKEN env var)",
            false,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;

        let result = googlechat_api(&token, "GET", "/spaces", None).await?;

        let spaces = result
            .get("spaces")
            .and_then(|v| v.as_array())
            .map(|arr| arr.as_slice())
            .unwrap_or(&[]);

        if spaces.is_empty() {
            return Ok("No spaces found.".to_string());
        }

        let mut output = format!("{} space(s):\n", spaces.len());
        for space in spaces {
            let name = space.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let display_name = space
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or("unnamed");
            let space_type = space.get("type").and_then(|v| v.as_str()).unwrap_or("?");
            output.push_str(&format!(
                "  {} ({}) — type: {}\n",
                display_name, name, space_type
            ));
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
        let tool = GoogleChatSendMessageTool;
        assert_eq!(tool.name(), "googlechat_send_message");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"space"));
        assert!(names.contains(&"text"));
    }

    #[test]
    fn test_get_messages_schema() {
        let tool = GoogleChatGetMessagesTool;
        assert_eq!(tool.name(), "googlechat_get_messages");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"space"));
    }

    #[test]
    fn test_list_spaces_schema() {
        let tool = GoogleChatListSpacesTool;
        assert_eq!(tool.name(), "googlechat_list_spaces");
    }

    #[test]
    fn test_get_token_from_args() {
        let args = json!({"token": "ya29.test-token"});
        let token = get_token(&args).expect("should succeed");
        assert_eq!(token, "ya29.test-token");
    }
}
