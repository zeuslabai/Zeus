//! Feishu/Lark Open API tools
//!
//! Provides tools for interacting with the Feishu (Lark) Open API.
//! Each tool accepts an optional `access_token` parameter, falling back to the
//! `FEISHU_ACCESS_TOKEN` environment variable. If no token is available,
//! it will attempt to obtain one using `FEISHU_APP_ID` and `FEISHU_APP_SECRET`.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

const FEISHU_API: &str = "https://open.feishu.cn/open-apis";

/// Get app credentials from args or environment
fn get_app_credentials(args: &Value) -> Result<(String, String)> {
    let app_id = args
        .get("app_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("FEISHU_APP_ID").ok())
        .ok_or_else(|| {
            Error::Tool("Missing 'app_id' parameter and FEISHU_APP_ID env var not set".to_string())
        })?;
    let app_secret = args
        .get("app_secret")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("FEISHU_APP_SECRET").ok())
        .ok_or_else(|| {
            Error::Tool(
                "Missing 'app_secret' parameter and FEISHU_APP_SECRET env var not set".to_string(),
            )
        })?;
    Ok((app_id, app_secret))
}

/// Get tenant_access_token from args, environment, or by requesting one
async fn get_token(args: &Value) -> Result<String> {
    // 1. Check direct token parameter
    if let Some(token) = args.get("access_token").and_then(|v| v.as_str()) {
        return Ok(token.to_string());
    }
    // 2. Check environment variable
    if let Ok(token) = std::env::var("FEISHU_ACCESS_TOKEN") {
        return Ok(token);
    }
    // 3. Obtain token via app credentials
    let (app_id, app_secret) = get_app_credentials(args)?;
    let url = format!("{}/auth/v3/tenant_access_token/internal", FEISHU_API);
    let client = reqwest::Client::new();

    let body = json!({
        "app_id": app_id,
        "app_secret": app_secret,
    });

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Tool(format!("Feishu auth request failed: {}", e)))?;

    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read auth response: {}", e)))?;

    let result: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Tool(format!("Invalid auth JSON: {}", e)))?;

    if result.get("code").and_then(|v| v.as_i64()) != Some(0) {
        let msg = result
            .get("msg")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(Error::Tool(format!("Feishu auth error: {}", msg)));
    }

    result
        .get("tenant_access_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::Tool("No tenant_access_token in auth response".to_string()))
}

/// Make a Feishu API request
async fn feishu_api(
    token: &str,
    method: &str,
    endpoint: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("{}{}", FEISHU_API, endpoint);
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
        .header("Content-Type", "application/json; charset=utf-8");

    if let Some(b) = body {
        req = req.json(b);
    }

    let response = req
        .send()
        .await
        .map_err(|e| Error::Tool(format!("Feishu API request failed: {}", e)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read response: {}", e)))?;

    if !status.is_success() {
        return Err(Error::Tool(format!(
            "Feishu API error {}: {}",
            status, text
        )));
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

    if result.get("code").and_then(|v| v.as_i64()) != Some(0) {
        let msg = result
            .get("msg")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(Error::Tool(format!("Feishu API error: {}", msg)));
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// 1. FeishuSendMessageTool
// ---------------------------------------------------------------------------

/// Send a text message to a Feishu chat
pub struct FeishuSendMessageTool;

#[async_trait]
impl TalosTool for FeishuSendMessageTool {
    fn name(&self) -> &'static str {
        "feishu_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a text message to a Feishu/Lark chat"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("chat_id", "string", "Feishu chat ID (receive_id)", true)
            .with_param("text", "string", "Message text content", true)
            .with_param(
                "access_token",
                "string",
                "Tenant access token (or set FEISHU_ACCESS_TOKEN env var)",
                false,
            )
            .with_param(
                "app_id",
                "string",
                "App ID for token auth (or set FEISHU_APP_ID env var)",
                false,
            )
            .with_param(
                "app_secret",
                "string",
                "App secret for token auth (or set FEISHU_APP_SECRET env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args).await?;
        let chat_id = args
            .get("chat_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'chat_id'".to_string()))?;
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'text'".to_string()))?;

        let content = json!({"text": text}).to_string();
        let body = json!({
            "receive_id": chat_id,
            "msg_type": "text",
            "content": content,
        });

        let endpoint = "/im/v1/messages?receive_id_type=chat_id";
        let result = feishu_api(&token, "POST", endpoint, Some(&body)).await?;

        let msg_id = result
            .get("data")
            .and_then(|d| d.get("message_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        Ok(format!("Message sent (message_id: {})", msg_id))
    }
}

// ---------------------------------------------------------------------------
// 2. FeishuGetMessagesTool
// ---------------------------------------------------------------------------

/// Get recent messages from a Feishu chat
pub struct FeishuGetMessagesTool;

#[async_trait]
impl TalosTool for FeishuGetMessagesTool {
    fn name(&self) -> &'static str {
        "feishu_get_messages"
    }
    fn description(&self) -> &'static str {
        "Get recent messages from a Feishu/Lark chat"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("chat_id", "string", "Feishu chat ID (container_id)", true)
            .with_param(
                "limit",
                "integer",
                "Max messages to return (1-50, default 20)",
                false,
            )
            .with_param(
                "access_token",
                "string",
                "Tenant access token (or set FEISHU_ACCESS_TOKEN env var)",
                false,
            )
            .with_param(
                "app_id",
                "string",
                "App ID for token auth (or set FEISHU_APP_ID env var)",
                false,
            )
            .with_param(
                "app_secret",
                "string",
                "App secret for token auth (or set FEISHU_APP_SECRET env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args).await?;
        let chat_id = args
            .get("chat_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'chat_id'".to_string()))?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .min(50);

        let endpoint = format!(
            "/im/v1/messages?container_id={}&container_id_type=chat&page_size={}",
            chat_id, limit
        );
        let result = feishu_api(&token, "GET", &endpoint, None).await?;

        let items = result
            .get("data")
            .and_then(|d| d.get("items"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.as_slice())
            .unwrap_or(&[]);

        if items.is_empty() {
            return Ok("No messages found.".to_string());
        }

        let mut output = format!("{} message(s):\n", items.len());
        for msg in items {
            let msg_id = msg
                .get("message_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let sender_id = msg
                .get("sender")
                .and_then(|s| s.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let msg_type = msg
                .get("msg_type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let body_str = msg
                .get("body")
                .and_then(|b| b.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("[no content]");
            output.push_str(&format!(
                "[{}] {} ({}): {}\n",
                msg_id, sender_id, msg_type, body_str
            ));
        }
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 3. FeishuListChatsTool
// ---------------------------------------------------------------------------

/// List Feishu chats the bot is a member of
pub struct FeishuListChatsTool;

#[async_trait]
impl TalosTool for FeishuListChatsTool {
    fn name(&self) -> &'static str {
        "feishu_list_chats"
    }
    fn description(&self) -> &'static str {
        "List Feishu/Lark chats the bot belongs to"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "limit",
                "integer",
                "Max chats to return (1-100, default 20)",
                false,
            )
            .with_param(
                "access_token",
                "string",
                "Tenant access token (or set FEISHU_ACCESS_TOKEN env var)",
                false,
            )
            .with_param(
                "app_id",
                "string",
                "App ID for token auth (or set FEISHU_APP_ID env var)",
                false,
            )
            .with_param(
                "app_secret",
                "string",
                "App secret for token auth (or set FEISHU_APP_SECRET env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args).await?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .min(100);

        let endpoint = format!("/im/v1/chats?page_size={}", limit);
        let result = feishu_api(&token, "GET", &endpoint, None).await?;

        let items = result
            .get("data")
            .and_then(|d| d.get("items"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.as_slice())
            .unwrap_or(&[]);

        if items.is_empty() {
            return Ok("No chats found.".to_string());
        }

        let mut output = format!("{} chat(s):\n", items.len());
        for chat in items {
            let chat_id = chat.get("chat_id").and_then(|v| v.as_str()).unwrap_or("?");
            let name = chat
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unnamed");
            let owner_id = chat.get("owner_id").and_then(|v| v.as_str()).unwrap_or("?");
            output.push_str(&format!("  {} ({}) — owner: {}\n", name, chat_id, owner_id));
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
        let tool = FeishuSendMessageTool;
        assert_eq!(tool.name(), "feishu_send_message");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"chat_id"));
        assert!(names.contains(&"text"));
    }

    #[test]
    fn test_get_messages_schema() {
        let tool = FeishuGetMessagesTool;
        assert_eq!(tool.name(), "feishu_get_messages");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"chat_id"));
    }

    #[test]
    fn test_list_chats_schema() {
        let tool = FeishuListChatsTool;
        assert_eq!(tool.name(), "feishu_list_chats");
    }

    #[test]
    fn test_get_token_from_args() {
        let args = json!({"access_token": "t-test-token"});
        let rt = tokio::runtime::Runtime::new().unwrap();
        let token = rt.block_on(get_token(&args)).expect("should succeed");
        assert_eq!(token, "t-test-token");
    }
}
