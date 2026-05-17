//! BlueBubbles API tools
//!
//! Provides tools for interacting with a BlueBubbles iMessage bridge server.
//! Each tool accepts optional `url` and `password` parameters, falling back to
//! `BLUEBUBBLES_URL` and `BLUEBUBBLES_PASSWORD` environment variables.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

/// Get BlueBubbles server URL from args or environment
fn get_url(args: &Value) -> Result<String> {
    if let Some(url) = args.get("url").and_then(|v| v.as_str()) {
        return Ok(url.trim_end_matches('/').to_string());
    }
    std::env::var("BLUEBUBBLES_URL")
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|_| {
            Error::Tool("Missing 'url' parameter and BLUEBUBBLES_URL env var not set".to_string())
        })
}

/// Get BlueBubbles password from args or environment
fn get_password(args: &Value) -> Result<String> {
    if let Some(pwd) = args.get("password").and_then(|v| v.as_str()) {
        return Ok(pwd.to_string());
    }
    std::env::var("BLUEBUBBLES_PASSWORD").map_err(|_| {
        Error::Tool(
            "Missing 'password' parameter and BLUEBUBBLES_PASSWORD env var not set".to_string(),
        )
    })
}

/// Make a BlueBubbles API request (password appended as query param)
async fn bluebubbles_api(
    base_url: &str,
    password: &str,
    method: &str,
    endpoint: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let separator = if endpoint.contains('?') { "&" } else { "?" };
    let url = format!("{}{}{}password={}", base_url, endpoint, separator, password);
    let client = reqwest::Client::new();

    let mut req = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        _ => return Err(Error::Tool(format!("Unsupported method: {}", method))),
    };

    req = req.header("Content-Type", "application/json");

    if let Some(b) = body {
        req = req.json(b);
    }

    let response = req
        .send()
        .await
        .map_err(|e| Error::Tool(format!("BlueBubbles API request failed: {}", e)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read response: {}", e)))?;

    if !status.is_success() {
        return Err(Error::Tool(format!(
            "BlueBubbles API error {}: {}",
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
// 1. BlueBubblesSendMessageTool
// ---------------------------------------------------------------------------

/// Send an iMessage via BlueBubbles
pub struct BlueBubblesSendMessageTool;

#[async_trait]
impl TalosTool for BlueBubblesSendMessageTool {
    fn name(&self) -> &'static str {
        "bluebubbles_send_message"
    }
    fn description(&self) -> &'static str {
        "Send an iMessage via BlueBubbles"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "chatGuid",
                "string",
                "Chat GUID (e.g. iMessage;-;+1234567890)",
                true,
            )
            .with_param("message", "string", "Message text to send", true)
            .with_param(
                "method",
                "string",
                "Send method (default: private-api)",
                false,
            )
            .with_param(
                "url",
                "string",
                "BlueBubbles server URL (or set BLUEBUBBLES_URL env var)",
                false,
            )
            .with_param(
                "password",
                "string",
                "Server password (or set BLUEBUBBLES_PASSWORD env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_url(&args)?;
        let password = get_password(&args)?;
        let chat_guid = args
            .get("chatGuid")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'chatGuid'".to_string()))?;
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'message'".to_string()))?;
        let method = args
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("private-api");

        let body = json!({
            "chatGuid": chat_guid,
            "message": message,
            "method": method
        });
        let result = bluebubbles_api(
            &base_url,
            &password,
            "POST",
            "/api/v1/message/text",
            Some(&body),
        )
        .await?;

        let status = result.get("status").and_then(|v| v.as_i64()).unwrap_or(0);
        let msg = result
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("sent");
        Ok(format!(
            "Message sent (status: {}, message: {})",
            status, msg
        ))
    }
}

// ---------------------------------------------------------------------------
// 2. BlueBubblesGetMessagesTool
// ---------------------------------------------------------------------------

/// Query recent messages from BlueBubbles
pub struct BlueBubblesGetMessagesTool;

#[async_trait]
impl TalosTool for BlueBubblesGetMessagesTool {
    fn name(&self) -> &'static str {
        "bluebubbles_get_messages"
    }
    fn description(&self) -> &'static str {
        "Query recent messages from BlueBubbles"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "limit",
                "integer",
                "Max messages to return (default 25)",
                false,
            )
            .with_param(
                "sort",
                "string",
                "Sort order: ASC or DESC (default DESC)",
                false,
            )
            .with_param(
                "url",
                "string",
                "BlueBubbles server URL (or set BLUEBUBBLES_URL env var)",
                false,
            )
            .with_param(
                "password",
                "string",
                "Server password (or set BLUEBUBBLES_PASSWORD env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_url(&args)?;
        let password = get_password(&args)?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(25);
        let sort = args.get("sort").and_then(|v| v.as_str()).unwrap_or("DESC");

        let body = json!({
            "limit": limit,
            "sort": sort,
            "with": ["chat"]
        });
        let result = bluebubbles_api(
            &base_url,
            &password,
            "POST",
            "/api/v1/message/query",
            Some(&body),
        )
        .await?;

        let messages = result
            .get("data")
            .and_then(|v| v.as_array())
            .map(|arr| arr.as_slice())
            .unwrap_or(&[]);

        if messages.is_empty() {
            return Ok("No messages found.".to_string());
        }

        let mut output = format!("{} message(s):\n", messages.len());
        for msg in messages {
            let handle = msg
                .get("handle")
                .and_then(|h| h.get("address"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let text = msg
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("[no text]");
            let guid = msg.get("guid").and_then(|v| v.as_str()).unwrap_or("?");
            let is_from_me = msg
                .get("isFromMe")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let direction = if is_from_me { ">>>" } else { "<<<" };
            output.push_str(&format!("[{}] {} {}: {}\n", guid, direction, handle, text));
        }
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 3. BlueBubblesListChatsTool
// ---------------------------------------------------------------------------

/// List iMessage chats from BlueBubbles
pub struct BlueBubblesListChatsTool;

#[async_trait]
impl TalosTool for BlueBubblesListChatsTool {
    fn name(&self) -> &'static str {
        "bluebubbles_list_chats"
    }
    fn description(&self) -> &'static str {
        "List iMessage chats from BlueBubbles"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "limit",
                "integer",
                "Max chats to return (default 25)",
                false,
            )
            .with_param(
                "url",
                "string",
                "BlueBubbles server URL (or set BLUEBUBBLES_URL env var)",
                false,
            )
            .with_param(
                "password",
                "string",
                "Server password (or set BLUEBUBBLES_PASSWORD env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_url(&args)?;
        let password = get_password(&args)?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(25);

        let endpoint = format!("/api/v1/chat?limit={}&offset=0", limit);
        let result = bluebubbles_api(&base_url, &password, "GET", &endpoint, None).await?;

        let chats = result
            .get("data")
            .and_then(|v| v.as_array())
            .map(|arr| arr.as_slice())
            .unwrap_or(&[]);

        if chats.is_empty() {
            return Ok("No chats found.".to_string());
        }

        let mut output = format!("{} chat(s):\n", chats.len());
        for chat in chats {
            let guid = chat.get("guid").and_then(|v| v.as_str()).unwrap_or("?");
            let display_name = chat
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let chat_id = chat
                .get("chatIdentifier")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let participants = chat
                .get("participants")
                .and_then(|v| v.as_array())
                .map(|arr| arr.len())
                .unwrap_or(0);

            if display_name.is_empty() {
                output.push_str(&format!(
                    "  {} ({}) — {} participants\n",
                    chat_id, guid, participants
                ));
            } else {
                output.push_str(&format!(
                    "  {} [{}] ({}) — {} participants\n",
                    display_name, chat_id, guid, participants
                ));
            }
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
        let tool = BlueBubblesSendMessageTool;
        assert_eq!(tool.name(), "bluebubbles_send_message");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"chatGuid"));
        assert!(names.contains(&"message"));
    }

    #[test]
    fn test_get_messages_schema() {
        let tool = BlueBubblesGetMessagesTool;
        assert_eq!(tool.name(), "bluebubbles_get_messages");
    }

    #[test]
    fn test_list_chats_schema() {
        let tool = BlueBubblesListChatsTool;
        assert_eq!(tool.name(), "bluebubbles_list_chats");
    }

    #[test]
    fn test_get_url_from_args() {
        let args = json!({"url": "http://localhost:1234"});
        let url = get_url(&args).expect("should succeed");
        assert_eq!(url, "http://localhost:1234");
    }

    #[test]
    fn test_get_url_strips_trailing_slash() {
        let args = json!({"url": "http://localhost:1234/"});
        let url = get_url(&args).expect("should succeed");
        assert_eq!(url, "http://localhost:1234");
    }

    #[test]
    fn test_get_password_from_args() {
        let args = json!({"password": "my-secret-pwd"});
        let pwd = get_password(&args).expect("should succeed");
        assert_eq!(pwd, "my-secret-pwd");
    }
}
