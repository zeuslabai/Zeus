//! Zalo Official Account API tools
//!
//! Provides tools for interacting with the Zalo Official Account HTTP API.
//! Each tool accepts an optional `access_token` parameter, falling back to the
//! `ZALO_ACCESS_TOKEN` environment variable.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

const ZALO_API: &str = "https://openapi.zalo.me/v3.0";

/// Get access token from args or environment
fn get_access_token(args: &Value) -> Result<String> {
    if let Some(token) = args.get("access_token").and_then(|v| v.as_str()) {
        return Ok(token.to_string());
    }
    std::env::var("ZALO_ACCESS_TOKEN").map_err(|_| {
        Error::Tool(
            "Missing 'access_token' parameter and ZALO_ACCESS_TOKEN env var not set".to_string(),
        )
    })
}

/// Get OA ID from args or environment
fn get_oa_id(args: &Value) -> Result<Option<String>> {
    if let Some(oa_id) = args.get("oa_id").and_then(|v| v.as_str()) {
        return Ok(Some(oa_id.to_string()));
    }
    match std::env::var("ZALO_OA_ID") {
        Ok(val) => Ok(Some(val)),
        Err(_) => Ok(None),
    }
}

/// Make a Zalo OA API request
async fn zalo_api(
    access_token: &str,
    method: &str,
    endpoint: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("{}{}", ZALO_API, endpoint);
    let client = reqwest::Client::new();

    let mut req = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        _ => return Err(Error::Tool(format!("Unsupported method: {}", method))),
    };

    req = req
        .header("access_token", access_token)
        .header("Content-Type", "application/json");

    if let Some(b) = body {
        req = req.json(b);
    }

    let response = req
        .send()
        .await
        .map_err(|e| Error::Tool(format!("Zalo API request failed: {}", e)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read response: {}", e)))?;

    if !status.is_success() {
        return Err(Error::Tool(format!("Zalo API error {}: {}", status, text)));
    }

    if text.is_empty() {
        return Ok(json!({"ok": true}));
    }

    let result: Value = serde_json::from_str(&text).map_err(|e| {
        Error::Tool(format!(
            "Invalid JSON: {} (body: {})",
            e,
            &text[..zeus_core::floor_char_boundary(&text, 200)]
        ))
    })?;

    // Zalo API returns error code 0 for success
    if let Some(error) = result.get("error").and_then(|v| v.as_i64())
        && error != 0
    {
        let message = result
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(Error::Tool(format!(
            "Zalo API error {}: {}",
            error, message
        )));
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// 1. ZaloSendMessageTool
// ---------------------------------------------------------------------------

/// Send a text message to a Zalo user via Official Account
pub struct ZaloSendMessageTool;

#[async_trait]
impl TalosTool for ZaloSendMessageTool {
    fn name(&self) -> &'static str {
        "zalo_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a text message to a Zalo user via Official Account"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "user_id",
                "string",
                "Zalo user ID to send the message to",
                true,
            )
            .with_param("text", "string", "Message text content", true)
            .with_param(
                "access_token",
                "string",
                "OA access token (or set ZALO_ACCESS_TOKEN env var)",
                false,
            )
            .with_param(
                "oa_id",
                "string",
                "Official Account ID (or set ZALO_OA_ID env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let access_token = get_access_token(&args)?;
        let _oa_id = get_oa_id(&args)?;
        let user_id = args
            .get("user_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'user_id'".to_string()))?;
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'text'".to_string()))?;

        let body = json!({
            "recipient": { "user_id": user_id },
            "message": { "text": text }
        });
        let result = zalo_api(&access_token, "POST", "/oa/message/cs", Some(&body)).await?;

        let msg_id = result
            .get("data")
            .and_then(|d| d.get("message_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        Ok(format!(
            "Message sent to user {} (message_id: {})",
            user_id, msg_id
        ))
    }
}

// ---------------------------------------------------------------------------
// 2. ZaloGetProfileTool
// ---------------------------------------------------------------------------

/// Get a Zalo user's profile via Official Account
pub struct ZaloGetProfileTool;

#[async_trait]
impl TalosTool for ZaloGetProfileTool {
    fn name(&self) -> &'static str {
        "zalo_get_profile"
    }
    fn description(&self) -> &'static str {
        "Get a Zalo user's profile via Official Account"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("user_id", "string", "Zalo user ID to get profile for", true)
            .with_param(
                "access_token",
                "string",
                "OA access token (or set ZALO_ACCESS_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let access_token = get_access_token(&args)?;
        let user_id = args
            .get("user_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'user_id'".to_string()))?;

        let endpoint = format!("/oa/user/detail?user_id={}", user_id);
        let result = zalo_api(&access_token, "GET", &endpoint, None).await?;

        let data = result.get("data").unwrap_or(&result);
        let display_name = data
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let user_id_resp = data
            .get("user_id")
            .and_then(|v| v.as_str())
            .unwrap_or(user_id);
        let avatar = data.get("avatar").and_then(|v| v.as_str());
        let is_follower = data.get("user_is_follower").and_then(|v| v.as_bool());

        let mut output = format!("User: {}\n  ID: {}", display_name, user_id_resp);
        if let Some(av) = avatar {
            output.push_str(&format!("\n  Avatar: {}", av));
        }
        if let Some(f) = is_follower {
            output.push_str(&format!("\n  Follower: {}", f));
        }
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 3. ZaloGetFollowersTool
// ---------------------------------------------------------------------------

/// Get followers list of a Zalo Official Account
pub struct ZaloGetFollowersTool;

#[async_trait]
impl TalosTool for ZaloGetFollowersTool {
    fn name(&self) -> &'static str {
        "zalo_get_followers"
    }
    fn description(&self) -> &'static str {
        "Get followers list of a Zalo Official Account"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("offset", "integer", "Pagination offset (default 0)", false)
            .with_param(
                "count",
                "integer",
                "Number of followers to return (default 10, max 50)",
                false,
            )
            .with_param(
                "access_token",
                "string",
                "OA access token (or set ZALO_ACCESS_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let access_token = get_access_token(&args)?;
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0);
        let count = args
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(50);

        let endpoint = format!("/oa/getfollowers?offset={}&count={}", offset, count);
        let result = zalo_api(&access_token, "GET", &endpoint, None).await?;

        let data = result.get("data").unwrap_or(&result);
        let total = data.get("total").and_then(|v| v.as_i64()).unwrap_or(0);
        let followers = data
            .get("followers")
            .and_then(|v| v.as_array())
            .map(|arr| arr.as_slice())
            .unwrap_or(&[]);

        if followers.is_empty() {
            return Ok(format!("No followers found (total: {}).", total));
        }

        let mut output = format!("{} follower(s) (total: {}):\n", followers.len(), total);
        for f in followers {
            let user_id = f.get("user_id").and_then(|v| v.as_str()).unwrap_or("?");
            output.push_str(&format!("  - {}\n", user_id));
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
        let tool = ZaloSendMessageTool;
        assert_eq!(tool.name(), "zalo_send_message");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"user_id"));
        assert!(names.contains(&"text"));
    }

    #[test]
    fn test_get_profile_schema() {
        let tool = ZaloGetProfileTool;
        assert_eq!(tool.name(), "zalo_get_profile");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"user_id"));
    }

    #[test]
    fn test_get_followers_schema() {
        let tool = ZaloGetFollowersTool;
        assert_eq!(tool.name(), "zalo_get_followers");
    }

    #[test]
    fn test_get_access_token_from_args() {
        let args = json!({"access_token": "test-zalo-token"});
        let token = get_access_token(&args).expect("should succeed");
        assert_eq!(token, "test-zalo-token");
    }
}
