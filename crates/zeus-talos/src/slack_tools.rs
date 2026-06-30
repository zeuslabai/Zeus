//! Slack Web API tools
//!
//! Provides tools for interacting with the Slack Web API.
//! Each tool accepts an optional `token` parameter, falling back to the
//! `SLACK_BOT_TOKEN` environment variable.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

const SLACK_API: &str = "https://slack.com/api";

/// Get bot token from args or environment
fn get_token(args: &Value) -> Result<String> {
    if let Some(token) = args.get("token").and_then(|v| v.as_str()) {
        return Ok(token.to_string());
    }
    std::env::var("SLACK_BOT_TOKEN").map_err(|_| {
        Error::Tool("Missing 'token' parameter and SLACK_BOT_TOKEN env var not set".to_string())
    })
}

/// Make a Slack Web API request
async fn slack_api(token: &str, method: &str, body: &Value) -> Result<Value> {
    let url = format!("{}/{}", SLACK_API, method);
    let client = reqwest::Client::new();

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json; charset=utf-8")
        .json(body)
        .send()
        .await
        .map_err(|e| Error::Tool(format!("Slack API request failed: {}", e)))?;

    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read response: {}", e)))?;

    let result: Value =
        serde_json::from_str(&text).map_err(|e| Error::Tool(format!("Invalid JSON: {}", e)))?;

    if result.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        Ok(result)
    } else {
        let err = result
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown_error");
        Err(Error::Tool(format!("Slack API error: {}", err)))
    }
}

// ---------------------------------------------------------------------------
// 1. SlackSendMessageTool
// ---------------------------------------------------------------------------

/// Send a text message to a Slack channel
pub struct SlackSendMessageTool;

#[async_trait]
impl TalosTool for SlackSendMessageTool {
    fn name(&self) -> &'static str {
        "slack_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a text message to a Slack channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "channel",
                "string",
                "Slack channel ID (e.g. C01234567)",
                true,
            )
            .with_param("text", "string", "Message text", true)
            .with_param(
                "thread_ts",
                "string",
                "Thread timestamp to reply in (optional)",
                false,
            )
            .with_param(
                "token",
                "string",
                "Bot token (or set SLACK_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let channel = args
            .get("channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel'".to_string()))?;
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'text'".to_string()))?;

        let mut body = json!({ "channel": channel, "text": text });
        if let Some(ts) = args.get("thread_ts").and_then(|v| v.as_str()) {
            body["thread_ts"] = json!(ts);
        }

        let result = slack_api(&token, "chat.postMessage", &body).await?;
        let ts = result
            .get("ts")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        Ok(format!("Message sent (ts: {})", ts))
    }
}

// ---------------------------------------------------------------------------
// 2. SlackGetMessagesTool
// ---------------------------------------------------------------------------

/// Get recent messages from a Slack channel
pub struct SlackGetMessagesTool;

#[async_trait]
impl TalosTool for SlackGetMessagesTool {
    fn name(&self) -> &'static str {
        "slack_get_messages"
    }
    fn description(&self) -> &'static str {
        "Get recent messages from a Slack channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel", "string", "Slack channel ID", true)
            .with_param(
                "limit",
                "integer",
                "Max messages to return (1-100, default 10)",
                false,
            )
            .with_param(
                "token",
                "string",
                "Bot token (or set SLACK_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let channel = args
            .get("channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel'".to_string()))?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(100);

        let body = json!({ "channel": channel, "limit": limit });
        let result = slack_api(&token, "conversations.history", &body).await?;

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
            let user = msg
                .get("user")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let text = msg
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("[no text]");
            let ts = msg.get("ts").and_then(|v| v.as_str()).unwrap_or("?");
            output.push_str(&format!("[{}] {}: {}\n", ts, user, text));
        }
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 3. SlackGetChannelInfoTool
// ---------------------------------------------------------------------------

/// Get information about a Slack channel
pub struct SlackGetChannelInfoTool;

#[async_trait]
impl TalosTool for SlackGetChannelInfoTool {
    fn name(&self) -> &'static str {
        "slack_get_channel_info"
    }
    fn description(&self) -> &'static str {
        "Get details about a Slack channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel", "string", "Slack channel ID", true)
            .with_param(
                "token",
                "string",
                "Bot token (or set SLACK_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let channel = args
            .get("channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel'".to_string()))?;

        let body = json!({ "channel": channel });
        let result = slack_api(&token, "conversations.info", &body).await?;

        let ch = result.get("channel").unwrap_or(&result);
        let name = ch.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
        let topic = ch
            .get("topic")
            .and_then(|t| t.get("value"))
            .and_then(|v| v.as_str());
        let purpose = ch
            .get("purpose")
            .and_then(|p| p.get("value"))
            .and_then(|v| v.as_str());
        let members = ch.get("num_members").and_then(|v| v.as_i64());
        let is_private = ch
            .get("is_private")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut output = format!(
            "Channel: #{}\n  ID: {}\n  Private: {}",
            name, channel, is_private
        );
        if let Some(t) = topic.filter(|s| !s.is_empty()) {
            output.push_str(&format!("\n  Topic: {}", t));
        }
        if let Some(p) = purpose.filter(|s| !s.is_empty()) {
            output.push_str(&format!("\n  Purpose: {}", p));
        }
        if let Some(m) = members {
            output.push_str(&format!("\n  Members: {}", m));
        }
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 4. SlackSendFileTool
// ---------------------------------------------------------------------------

/// Upload and share a file in a Slack channel
pub struct SlackSendFileTool;

#[async_trait]
impl TalosTool for SlackSendFileTool {
    fn name(&self) -> &'static str {
        "slack_send_file"
    }
    fn description(&self) -> &'static str {
        "Upload and share a file in a Slack channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel", "string", "Slack channel ID", true)
            .with_param("file_path", "string", "Local file path to upload", true)
            .with_param("title", "string", "File title", false)
            .with_param(
                "initial_comment",
                "string",
                "Comment to include with the file",
                false,
            )
            .with_param(
                "token",
                "string",
                "Bot token (or set SLACK_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let channel = args
            .get("channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel'".to_string()))?;
        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'file_path'".to_string()))?;

        let file_bytes = std::fs::read(file_path)
            .map_err(|e| Error::Tool(format!("Failed to read file: {}", e)))?;
        let file_name = std::path::Path::new(file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");

        let url = format!("{}/files.upload", SLACK_API);
        let client = reqwest::Client::new();

        let mut form = reqwest::multipart::Form::new()
            .text("channels", channel.to_string())
            .part(
                "file",
                reqwest::multipart::Part::bytes(file_bytes).file_name(file_name.to_string()),
            );

        if let Some(title) = args.get("title").and_then(|v| v.as_str()) {
            form = form.text("title", title.to_string());
        }
        if let Some(comment) = args.get("initial_comment").and_then(|v| v.as_str()) {
            form = form.text("initial_comment", comment.to_string());
        }

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::Tool(format!("Request failed: {}", e)))?;

        let text = response.text().await.unwrap_or_default();
        let result: Value = serde_json::from_str(&text).unwrap_or(json!({}));

        if result.get("ok").and_then(|v| v.as_bool()) == Some(true) {
            Ok(format!(
                "File '{}' uploaded to channel {}",
                file_name, channel
            ))
        } else {
            let err = result
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            Err(Error::Tool(format!("Slack file upload error: {}", err)))
        }
    }
}

// ---------------------------------------------------------------------------
// 5. SlackListChannelsTool
// ---------------------------------------------------------------------------

/// List Slack channels
pub struct SlackListChannelsTool;

#[async_trait]
impl TalosTool for SlackListChannelsTool {
    fn name(&self) -> &'static str {
        "slack_list_channels"
    }
    fn description(&self) -> &'static str {
        "List available Slack channels"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "limit",
                "integer",
                "Max channels to return (default 100)",
                false,
            )
            .with_param(
                "token",
                "string",
                "Bot token (or set SLACK_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(100);

        let body = json!({ "limit": limit, "exclude_archived": true });
        let result = slack_api(&token, "conversations.list", &body).await?;

        let channels = result
            .get("channels")
            .and_then(|v| v.as_array())
            .map(|arr| arr.as_slice())
            .unwrap_or(&[]);

        if channels.is_empty() {
            return Ok("No channels found.".to_string());
        }

        let mut output = format!("{} channel(s):\n", channels.len());
        for ch in channels {
            let name = ch.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let id = ch.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let members = ch.get("num_members").and_then(|v| v.as_i64()).unwrap_or(0);
            output.push_str(&format!("  #{} ({}) — {} members\n", name, id, members));
        }
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 6. SlackSetTopicTool
// ---------------------------------------------------------------------------

/// Set a Slack channel's topic
pub struct SlackSetTopicTool;

#[async_trait]
impl TalosTool for SlackSetTopicTool {
    fn name(&self) -> &'static str {
        "slack_set_topic"
    }
    fn description(&self) -> &'static str {
        "Set the topic of a Slack channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel", "string", "Slack channel ID", true)
            .with_param("topic", "string", "New topic text", true)
            .with_param(
                "token",
                "string",
                "Bot token (or set SLACK_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let channel = args
            .get("channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel'".to_string()))?;
        let topic = args
            .get("topic")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'topic'".to_string()))?;

        let body = json!({ "channel": channel, "topic": topic });
        slack_api(&token, "conversations.setTopic", &body).await?;
        Ok(format!("Topic set for channel {}", channel))
    }
}

// ---------------------------------------------------------------------------
// 7. SlackAddReactionTool
// ---------------------------------------------------------------------------

/// Add a reaction emoji to a Slack message
pub struct SlackAddReactionTool;

#[async_trait]
impl TalosTool for SlackAddReactionTool {
    fn name(&self) -> &'static str {
        "slack_add_reaction"
    }
    fn description(&self) -> &'static str {
        "Add a reaction emoji to a Slack message"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel", "string", "Slack channel ID", true)
            .with_param("timestamp", "string", "Message timestamp to react to", true)
            .with_param(
                "name",
                "string",
                "Emoji name without colons (e.g. 'thumbsup')",
                true,
            )
            .with_param(
                "token",
                "string",
                "Bot token (or set SLACK_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let channel = args
            .get("channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel'".to_string()))?;
        let timestamp = args
            .get("timestamp")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'timestamp'".to_string()))?;
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'name'".to_string()))?;

        let body = json!({ "channel": channel, "timestamp": timestamp, "name": name });
        slack_api(&token, "reactions.add", &body).await?;
        Ok(format!("Reaction :{}: added", name))
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
        let tool = SlackSendMessageTool;
        assert_eq!(tool.name(), "slack_send_message");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"channel"));
        assert!(names.contains(&"text"));
    }

    #[test]
    fn test_get_messages_schema() {
        let tool = SlackGetMessagesTool;
        assert_eq!(tool.name(), "slack_get_messages");
    }

    #[test]
    fn test_get_channel_info_schema() {
        let tool = SlackGetChannelInfoTool;
        assert_eq!(tool.name(), "slack_get_channel_info");
    }

    #[test]
    fn test_send_file_schema() {
        let tool = SlackSendFileTool;
        assert_eq!(tool.name(), "slack_send_file");
    }

    #[test]
    fn test_list_channels_schema() {
        let tool = SlackListChannelsTool;
        assert_eq!(tool.name(), "slack_list_channels");
    }

    #[test]
    fn test_set_topic_schema() {
        let tool = SlackSetTopicTool;
        assert_eq!(tool.name(), "slack_set_topic");
    }

    #[test]
    fn test_add_reaction_schema() {
        let tool = SlackAddReactionTool;
        assert_eq!(tool.name(), "slack_add_reaction");
    }

    #[test]
    fn test_get_token_from_args() {
        let args = json!({"token": "xoxb-test"});
        let token = get_token(&args).expect("should succeed");
        assert_eq!(token, "xoxb-test");
    }
}
