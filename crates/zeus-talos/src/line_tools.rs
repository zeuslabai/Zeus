//! LINE Messaging API tools
//!
//! Provides tools for interacting with the LINE Messaging API.
//! Each tool accepts an optional `token` parameter, falling back to the
//! `LINE_CHANNEL_ACCESS_TOKEN` environment variable.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

const LINE_API: &str = "https://api.line.me/v2";

/// Get channel access token from args or environment
fn get_token(args: &Value) -> Result<String> {
    if let Some(token) = args.get("token").and_then(|v| v.as_str()) {
        return Ok(token.to_string());
    }
    std::env::var("LINE_CHANNEL_ACCESS_TOKEN").map_err(|_| {
        Error::Tool(
            "Missing 'token' parameter and LINE_CHANNEL_ACCESS_TOKEN env var not set".to_string(),
        )
    })
}

/// Make a LINE API request
async fn line_api(
    token: &str,
    method: &str,
    endpoint: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("{}{}", LINE_API, endpoint);
    let client = reqwest::Client::new();

    let mut req = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
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
        .map_err(|e| Error::Tool(format!("LINE API request failed: {}", e)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read response: {}", e)))?;

    if !status.is_success() {
        return Err(Error::Tool(format!("LINE API error {}: {}", status, text)));
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
// 1. LineSendMessageTool
// ---------------------------------------------------------------------------

/// Send a text message via LINE push message API
pub struct LineSendMessageTool;

#[async_trait]
impl TalosTool for LineSendMessageTool {
    fn name(&self) -> &'static str {
        "line_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a text message to a LINE user or group"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("to", "string", "LINE user ID or group ID to send to", true)
            .with_param("text", "string", "Message text content", true)
            .with_param(
                "token",
                "string",
                "Channel access token (or set LINE_CHANNEL_ACCESS_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let to = args
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'to'".to_string()))?;
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'text'".to_string()))?;

        let body = json!({
            "to": to,
            "messages": [
                {
                    "type": "text",
                    "text": text,
                }
            ]
        });

        line_api(&token, "POST", "/bot/message/push", Some(&body)).await?;
        Ok(format!("Message sent to {}", to))
    }
}

// ---------------------------------------------------------------------------
// 2. LineGetProfileTool
// ---------------------------------------------------------------------------

/// Get a LINE user's profile
pub struct LineGetProfileTool;

#[async_trait]
impl TalosTool for LineGetProfileTool {
    fn name(&self) -> &'static str {
        "line_get_profile"
    }
    fn description(&self) -> &'static str {
        "Get a LINE user's profile information"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("user_id", "string", "LINE user ID", true)
            .with_param(
                "token",
                "string",
                "Channel access token (or set LINE_CHANNEL_ACCESS_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let user_id = args
            .get("user_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'user_id'".to_string()))?;

        let endpoint = format!("/bot/profile/{}", user_id);
        let result = line_api(&token, "GET", &endpoint, None).await?;

        let display_name = result
            .get("displayName")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let status_message = result.get("statusMessage").and_then(|v| v.as_str());
        let picture_url = result.get("pictureUrl").and_then(|v| v.as_str());
        let language = result.get("language").and_then(|v| v.as_str());

        let mut output = format!("Profile: {}\n  User ID: {}", display_name, user_id);
        if let Some(status) = status_message.filter(|s| !s.is_empty()) {
            output.push_str(&format!("\n  Status: {}", status));
        }
        if let Some(pic) = picture_url {
            output.push_str(&format!("\n  Picture: {}", pic));
        }
        if let Some(lang) = language {
            output.push_str(&format!("\n  Language: {}", lang));
        }
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 3. LineGetGroupInfoTool
// ---------------------------------------------------------------------------

/// Get information about a LINE group
pub struct LineGetGroupInfoTool;

#[async_trait]
impl TalosTool for LineGetGroupInfoTool {
    fn name(&self) -> &'static str {
        "line_get_group_info"
    }
    fn description(&self) -> &'static str {
        "Get summary information about a LINE group"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("group_id", "string", "LINE group ID", true)
            .with_param(
                "token",
                "string",
                "Channel access token (or set LINE_CHANNEL_ACCESS_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let group_id = args
            .get("group_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'group_id'".to_string()))?;

        let endpoint = format!("/bot/group/{}/summary", group_id);
        let result = line_api(&token, "GET", &endpoint, None).await?;

        let group_name = result
            .get("groupName")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let picture_url = result.get("pictureUrl").and_then(|v| v.as_str());
        let member_count = result.get("memberCount").and_then(|v| v.as_i64());

        let mut output = format!("Group: {}\n  Group ID: {}", group_name, group_id);
        if let Some(count) = member_count {
            output.push_str(&format!("\n  Members: {}", count));
        }
        if let Some(pic) = picture_url {
            output.push_str(&format!("\n  Picture: {}", pic));
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
        let tool = LineSendMessageTool;
        assert_eq!(tool.name(), "line_send_message");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"to"));
        assert!(names.contains(&"text"));
    }

    #[test]
    fn test_get_profile_schema() {
        let tool = LineGetProfileTool;
        assert_eq!(tool.name(), "line_get_profile");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"user_id"));
    }

    #[test]
    fn test_get_group_info_schema() {
        let tool = LineGetGroupInfoTool;
        assert_eq!(tool.name(), "line_get_group_info");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"group_id"));
    }

    #[test]
    fn test_get_token_from_args() {
        let args = json!({"token": "line-test-token"});
        let token = get_token(&args).expect("should succeed");
        assert_eq!(token, "line-test-token");
    }
}
