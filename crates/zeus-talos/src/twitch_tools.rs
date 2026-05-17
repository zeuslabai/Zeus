//! Twitch Helix API tools
//!
//! Requires TWITCH_ACCESS_TOKEN + TWITCH_CLIENT_ID or pass per-call.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

const TWITCH_API: &str = "https://api.twitch.tv/helix";

fn get_auth(args: &Value) -> Result<(String, String)> {
    let token = args
        .get("token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("TWITCH_ACCESS_TOKEN").ok())
        .ok_or_else(|| Error::Tool("Missing token / TWITCH_ACCESS_TOKEN".to_string()))?;
    let client_id = args
        .get("client_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("TWITCH_CLIENT_ID").ok())
        .ok_or_else(|| Error::Tool("Missing client_id / TWITCH_CLIENT_ID".to_string()))?;
    Ok((token, client_id))
}

async fn twitch_api(
    token: &str,
    client_id: &str,
    method: &str,
    endpoint: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("{}{}", TWITCH_API, endpoint);
    let client = reqwest::Client::new();
    let mut req = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        _ => return Err(Error::Tool(format!("Unsupported: {}", method))),
    };
    req = req
        .header("Authorization", format!("Bearer {}", token))
        .header("Client-Id", client_id)
        .header("Content-Type", "application/json");
    if let Some(b) = body {
        req = req.json(b);
    }
    let resp = req.send().await.map_err(|e| Error::Tool(e.to_string()))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| Error::Tool(e.to_string()))?;
    if !status.is_success() {
        return Err(Error::Tool(format!("Twitch API {}: {}", status, text)));
    }
    if text.is_empty() {
        return Ok(json!({"ok": true}));
    }
    serde_json::from_str(&text).map_err(|e| Error::Tool(format!("JSON error: {}", e)))
}

pub struct TwitchSendMessageTool;
#[async_trait]
impl TalosTool for TwitchSendMessageTool {
    fn name(&self) -> &'static str {
        "twitch_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a chat message to a Twitch channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("broadcaster_id", "string", "Broadcaster user ID", true)
            .with_param("sender_id", "string", "Sender (bot) user ID", true)
            .with_param("message", "string", "Chat message text", true)
            .with_param(
                "token",
                "string",
                "OAuth token (or TWITCH_ACCESS_TOKEN)",
                false,
            )
            .with_param(
                "client_id",
                "string",
                "Client ID (or TWITCH_CLIENT_ID)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let (token, client_id) = get_auth(&args)?;
        let broadcaster_id = args
            .get("broadcaster_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'broadcaster_id'".to_string()))?;
        let sender_id = args
            .get("sender_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'sender_id'".to_string()))?;
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'message'".to_string()))?;
        let body =
            json!({"broadcaster_id": broadcaster_id, "sender_id": sender_id, "message": message});
        twitch_api(&token, &client_id, "POST", "/chat/messages", Some(&body)).await?;
        Ok("Chat message sent".to_string())
    }
}

pub struct TwitchGetChannelInfoTool;
#[async_trait]
impl TalosTool for TwitchGetChannelInfoTool {
    fn name(&self) -> &'static str {
        "twitch_get_channel_info"
    }
    fn description(&self) -> &'static str {
        "Get Twitch channel information"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("broadcaster_id", "string", "Broadcaster user ID", true)
            .with_param("token", "string", "OAuth token", false)
            .with_param("client_id", "string", "Client ID", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let (token, client_id) = get_auth(&args)?;
        let bid = args
            .get("broadcaster_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'broadcaster_id'".to_string()))?;
        let result = twitch_api(
            &token,
            &client_id,
            "GET",
            &format!("/channels?broadcaster_id={}", bid),
            None,
        )
        .await?;
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "No data".to_string()))
    }
}

pub struct TwitchGetStreamsTool;
#[async_trait]
impl TalosTool for TwitchGetStreamsTool {
    fn name(&self) -> &'static str {
        "twitch_get_streams"
    }
    fn description(&self) -> &'static str {
        "Get live streams (optionally filtered by user)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "user_login",
                "string",
                "Filter by username (optional)",
                false,
            )
            .with_param("game_id", "string", "Filter by game ID (optional)", false)
            .with_param("first", "integer", "Number of results (default 10)", false)
            .with_param("token", "string", "OAuth token", false)
            .with_param("client_id", "string", "Client ID", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let (token, client_id) = get_auth(&args)?;
        let mut params = vec![];
        if let Some(u) = args.get("user_login").and_then(|v| v.as_str()) {
            params.push(format!("user_login={}", u));
        }
        if let Some(g) = args.get("game_id").and_then(|v| v.as_str()) {
            params.push(format!("game_id={}", g));
        }
        let first = args.get("first").and_then(|v| v.as_u64()).unwrap_or(10);
        params.push(format!("first={}", first));
        let endpoint = format!("/streams?{}", params.join("&"));
        let result = twitch_api(&token, &client_id, "GET", &endpoint, None).await?;
        let streams = result
            .get("data")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);
        if streams.is_empty() {
            return Ok("No live streams found.".to_string());
        }
        let mut out = format!("{} stream(s):\n", streams.len());
        for s in streams {
            let name = s.get("user_name").and_then(|v| v.as_str()).unwrap_or("?");
            let title = s.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let viewers = s.get("viewer_count").and_then(|v| v.as_i64()).unwrap_or(0);
            out.push_str(&format!(
                "  {} ({} viewers): {}\n",
                name,
                viewers,
                &title[..zeus_core::floor_char_boundary(&title, 80)]
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_send_msg() {
        assert_eq!(TwitchSendMessageTool.name(), "twitch_send_message");
    }
    #[test]
    fn test_channel_info() {
        assert_eq!(TwitchGetChannelInfoTool.name(), "twitch_get_channel_info");
    }
    #[test]
    fn test_get_streams() {
        assert_eq!(TwitchGetStreamsTool.name(), "twitch_get_streams");
    }
}
