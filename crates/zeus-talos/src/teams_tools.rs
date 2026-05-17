//! Microsoft Teams tools via Microsoft Graph API
//!
//! Requires TEAMS_ACCESS_TOKEN or pass token per-call.
//! Uses Microsoft Graph API v1.0.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

const GRAPH_API: &str = "https://graph.microsoft.com/v1.0";

fn get_token(args: &Value) -> Result<String> {
    args.get("token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("TEAMS_ACCESS_TOKEN").ok())
        .ok_or_else(|| Error::Tool("Missing 'token' and TEAMS_ACCESS_TOKEN not set".to_string()))
}

async fn graph_api(
    token: &str,
    method: &str,
    endpoint: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("{}{}", GRAPH_API, endpoint);
    let client = reqwest::Client::new();
    let mut req = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
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
        .map_err(|e| Error::Tool(format!("Request failed: {}", e)))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(e.to_string()))?;
    if !status.is_success() {
        return Err(Error::Tool(format!("Graph API error {}: {}", status, text)));
    }
    if text.is_empty() {
        return Ok(json!({"ok": true}));
    }
    serde_json::from_str(&text).map_err(|e| Error::Tool(format!("Invalid JSON: {}", e)))
}

pub struct TeamsSendMessageTool;
#[async_trait]
impl TalosTool for TeamsSendMessageTool {
    fn name(&self) -> &'static str {
        "teams_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a message to a Microsoft Teams channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("team_id", "string", "Team ID", true)
            .with_param("channel_id", "string", "Channel ID", true)
            .with_param(
                "content",
                "string",
                "Message content (HTML supported)",
                true,
            )
            .with_param(
                "token",
                "string",
                "Access token (or set TEAMS_ACCESS_TOKEN)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let team_id = args
            .get("team_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'team_id'".to_string()))?;
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'content'".to_string()))?;
        let body = json!({"body": {"content": content}});
        let endpoint = format!("/teams/{}/channels/{}/messages", team_id, channel_id);
        let result = graph_api(&token, "POST", &endpoint, Some(&body)).await?;
        let id = result
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        Ok(format!("Message sent (id: {})", id))
    }
}

pub struct TeamsGetMessagesTool;
#[async_trait]
impl TalosTool for TeamsGetMessagesTool {
    fn name(&self) -> &'static str {
        "teams_get_messages"
    }
    fn description(&self) -> &'static str {
        "Get recent messages from a Teams channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("team_id", "string", "Team ID", true)
            .with_param("channel_id", "string", "Channel ID", true)
            .with_param("top", "integer", "Max messages (default 10)", false)
            .with_param(
                "token",
                "string",
                "Access token (or set TEAMS_ACCESS_TOKEN)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let team_id = args
            .get("team_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'team_id'".to_string()))?;
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let top = args.get("top").and_then(|v| v.as_u64()).unwrap_or(10);
        let endpoint = format!(
            "/teams/{}/channels/{}/messages?$top={}",
            team_id, channel_id, top
        );
        let result = graph_api(&token, "GET", &endpoint, None).await?;
        let msgs = result
            .get("value")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);
        if msgs.is_empty() {
            return Ok("No messages.".to_string());
        }
        let mut out = format!("{} message(s):\n", msgs.len());
        for m in msgs {
            let from = m
                .get("from")
                .and_then(|f| f.get("user"))
                .and_then(|u| u.get("displayName"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");
            let body = m
                .get("body")
                .and_then(|b| b.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            out.push_str(&format!("{}: {}\n", from, &body[..zeus_core::floor_char_boundary(&body, 200)]));
        }
        Ok(out)
    }
}

pub struct TeamsListChannelsTool;
#[async_trait]
impl TalosTool for TeamsListChannelsTool {
    fn name(&self) -> &'static str {
        "teams_list_channels"
    }
    fn description(&self) -> &'static str {
        "List channels in a Microsoft Teams team"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("team_id", "string", "Team ID", true)
            .with_param(
                "token",
                "string",
                "Access token (or set TEAMS_ACCESS_TOKEN)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let team_id = args
            .get("team_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'team_id'".to_string()))?;
        let result =
            graph_api(&token, "GET", &format!("/teams/{}/channels", team_id), None).await?;
        let chs = result
            .get("value")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);
        let mut out = format!("{} channel(s):\n", chs.len());
        for ch in chs {
            let name = ch
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let id = ch.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            out.push_str(&format!("  {} ({})\n", name, id));
        }
        Ok(out)
    }
}

pub struct TeamsListTeamsTool;
#[async_trait]
impl TalosTool for TeamsListTeamsTool {
    fn name(&self) -> &'static str {
        "teams_list_teams"
    }
    fn description(&self) -> &'static str {
        "List joined Microsoft Teams teams"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "token",
            "string",
            "Access token (or set TEAMS_ACCESS_TOKEN)",
            false,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let result = graph_api(&token, "GET", "/me/joinedTeams", None).await?;
        let teams = result
            .get("value")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);
        let mut out = format!("{} team(s):\n", teams.len());
        for t in teams {
            let name = t.get("displayName").and_then(|v| v.as_str()).unwrap_or("?");
            let id = t.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            out.push_str(&format!("  {} ({})\n", name, id));
        }
        Ok(out)
    }
}

pub struct TeamsSendChatMessageTool;
#[async_trait]
impl TalosTool for TeamsSendChatMessageTool {
    fn name(&self) -> &'static str {
        "teams_send_chat"
    }
    fn description(&self) -> &'static str {
        "Send a direct/group chat message in Teams"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("chat_id", "string", "Chat ID", true)
            .with_param("content", "string", "Message content", true)
            .with_param(
                "token",
                "string",
                "Access token (or set TEAMS_ACCESS_TOKEN)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let chat_id = args
            .get("chat_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'chat_id'".to_string()))?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'content'".to_string()))?;
        let body = json!({"body": {"content": content}});
        let result = graph_api(
            &token,
            "POST",
            &format!("/chats/{}/messages", chat_id),
            Some(&body),
        )
        .await?;
        let id = result
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        Ok(format!("Chat message sent (id: {})", id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_send_message_schema() {
        let t = TeamsSendMessageTool;
        assert_eq!(t.name(), "teams_send_message");
    }
    #[test]
    fn test_get_messages_schema() {
        let t = TeamsGetMessagesTool;
        assert_eq!(t.name(), "teams_get_messages");
    }
    #[test]
    fn test_list_channels_schema() {
        let t = TeamsListChannelsTool;
        assert_eq!(t.name(), "teams_list_channels");
    }
    #[test]
    fn test_list_teams_schema() {
        let t = TeamsListTeamsTool;
        assert_eq!(t.name(), "teams_list_teams");
    }
    #[test]
    fn test_send_chat_schema() {
        let t = TeamsSendChatMessageTool;
        assert_eq!(t.name(), "teams_send_chat");
    }
}
