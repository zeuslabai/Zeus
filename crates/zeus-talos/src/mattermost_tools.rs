//! Mattermost REST API tools
//!
//! Requires MATTERMOST_URL + MATTERMOST_TOKEN or pass per-call.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

fn get_url(args: &Value) -> Result<String> {
    args.get("url")
        .and_then(|v| v.as_str())
        .map(|s| s.trim_end_matches('/').to_string())
        .or_else(|| std::env::var("MATTERMOST_URL").ok())
        .ok_or_else(|| Error::Tool("Missing 'url' / MATTERMOST_URL".to_string()))
}

fn get_token(args: &Value) -> Result<String> {
    args.get("token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("MATTERMOST_TOKEN").ok())
        .ok_or_else(|| Error::Tool("Missing 'token' / MATTERMOST_TOKEN".to_string()))
}

async fn mm_api(
    base: &str,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("{}/api/v4{}", base, path);
    let client = reqwest::Client::new();
    let mut req = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        _ => return Err(Error::Tool(format!("Unsupported: {}", method))),
    };
    req = req
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json");
    if let Some(b) = body {
        req = req.json(b);
    }
    let resp = req.send().await.map_err(|e| Error::Tool(e.to_string()))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| Error::Tool(e.to_string()))?;
    if !status.is_success() {
        return Err(Error::Tool(format!("MM API {}: {}", status, text)));
    }
    if text.is_empty() {
        return Ok(json!({"ok": true}));
    }
    serde_json::from_str(&text).map_err(|e| Error::Tool(format!("JSON: {}", e)))
}

pub struct MattermostSendMessageTool;
#[async_trait]
impl TalosTool for MattermostSendMessageTool {
    fn name(&self) -> &'static str {
        "mattermost_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a message to a Mattermost channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel_id", "string", "Channel ID", true)
            .with_param(
                "message",
                "string",
                "Message text (Markdown supported)",
                true,
            )
            .with_param("url", "string", "Mattermost URL (or MATTERMOST_URL)", false)
            .with_param(
                "token",
                "string",
                "Access token (or MATTERMOST_TOKEN)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base = get_url(&args)?;
        let token = get_token(&args)?;
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'message'".to_string()))?;
        let body = json!({"channel_id": channel_id, "message": message});
        let result = mm_api(&base, &token, "POST", "/posts", Some(&body)).await?;
        let id = result
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        Ok(format!("Message sent (id: {})", id))
    }
}

pub struct MattermostGetMessagesTool;
#[async_trait]
impl TalosTool for MattermostGetMessagesTool {
    fn name(&self) -> &'static str {
        "mattermost_get_messages"
    }
    fn description(&self) -> &'static str {
        "Get recent messages from a Mattermost channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel_id", "string", "Channel ID", true)
            .with_param(
                "per_page",
                "integer",
                "Messages per page (default 10)",
                false,
            )
            .with_param("url", "string", "Mattermost URL", false)
            .with_param("token", "string", "Access token", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base = get_url(&args)?;
        let token = get_token(&args)?;
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let per_page = args.get("per_page").and_then(|v| v.as_u64()).unwrap_or(10);
        let path = format!("/channels/{}/posts?per_page={}", channel_id, per_page);
        let result = mm_api(&base, &token, "GET", &path, None).await?;
        Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "No data".to_string()))
    }
}

pub struct MattermostListChannelsTool;
#[async_trait]
impl TalosTool for MattermostListChannelsTool {
    fn name(&self) -> &'static str {
        "mattermost_list_channels"
    }
    fn description(&self) -> &'static str {
        "List channels in a Mattermost team"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("team_id", "string", "Team ID", true)
            .with_param("url", "string", "Mattermost URL", false)
            .with_param("token", "string", "Access token", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base = get_url(&args)?;
        let token = get_token(&args)?;
        let team_id = args
            .get("team_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'team_id'".to_string()))?;
        let result = mm_api(
            &base,
            &token,
            "GET",
            &format!("/teams/{}/channels", team_id),
            None,
        )
        .await?;
        let chs = result.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
        let mut out = format!("{} channel(s):\n", chs.len());
        for ch in chs {
            let name = ch
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let id = ch.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            out.push_str(&format!("  {} ({})\n", name, id));
        }
        Ok(out)
    }
}

pub struct MattermostReplyToThreadTool;
#[async_trait]
impl TalosTool for MattermostReplyToThreadTool {
    fn name(&self) -> &'static str {
        "mattermost_reply_to_thread"
    }
    fn description(&self) -> &'static str {
        "Reply to an existing Mattermost thread by specifying the root post ID"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel_id", "string", "Channel ID", true)
            .with_param("root_id", "string", "ID of the root post to reply to", true)
            .with_param("message", "string", "Reply message text (Markdown supported)", true)
            .with_param("url", "string", "Mattermost URL (or MATTERMOST_URL)", false)
            .with_param("token", "string", "Access token (or MATTERMOST_TOKEN)", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base = get_url(&args)?;
        let token = get_token(&args)?;
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let root_id = args
            .get("root_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'root_id'".to_string()))?;
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'message'".to_string()))?;
        let body = json!({
            "channel_id": channel_id,
            "message": message,
            "root_id": root_id
        });
        let result = mm_api(&base, &token, "POST", "/posts", Some(&body)).await?;
        let id = result
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        Ok(format!("Thread reply sent (id: {}, root: {})", id, root_id))
    }
}

pub struct MattermostSendSlashCommandTool;
#[async_trait]
impl TalosTool for MattermostSendSlashCommandTool {
    fn name(&self) -> &'static str {
        "mattermost_send_slash_command"
    }
    fn description(&self) -> &'static str {
        "Execute a slash command in a Mattermost channel (e.g. /away, /status, /remind)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel_id", "string", "Channel ID to execute the command in", true)
            .with_param("command", "string", "Slash command including leading slash (e.g. /away)", true)
            .with_param("url", "string", "Mattermost URL (or MATTERMOST_URL)", false)
            .with_param("token", "string", "Access token (or MATTERMOST_TOKEN)", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base = get_url(&args)?;
        let token = get_token(&args)?;
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'command'".to_string()))?;
        let body = json!({
            "channel_id": channel_id,
            "command": command
        });
        let result = mm_api(&base, &token, "POST", "/commands/execute", Some(&body)).await?;
        Ok(format!(
            "Command '{}' executed: {}",
            command,
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| "ok".to_string())
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_send() {
        assert_eq!(MattermostSendMessageTool.name(), "mattermost_send_message");
    }
    #[test]
    fn test_get() {
        assert_eq!(MattermostGetMessagesTool.name(), "mattermost_get_messages");
    }
    #[test]
    fn test_list() {
        assert_eq!(
            MattermostListChannelsTool.name(),
            "mattermost_list_channels"
        );
    }
    #[test]
    fn test_reply_to_thread() {
        assert_eq!(MattermostReplyToThreadTool.name(), "mattermost_reply_to_thread");
    }
    #[test]
    fn test_slash_command() {
        assert_eq!(MattermostSendSlashCommandTool.name(), "mattermost_send_slash_command");
    }
}
