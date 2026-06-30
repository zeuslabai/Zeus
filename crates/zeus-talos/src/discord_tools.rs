//! Discord Bot API tools
//!
//! Provides tools for interacting with the Discord Bot HTTP API.
//! Each tool accepts an optional `token` parameter, falling back to the
//! `DISCORD_BOT_TOKEN` environment variable.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

const DISCORD_API: &str = "https://discord.com/api/v10";

/// Get bot token from args, config.toml, or environment (in that order).
/// Config.toml is the SSoT for secrets — env var kept as legacy fallback.
fn get_token(args: &Value) -> Result<String> {
    // 1. Explicit parameter
    if let Some(token) = args.get("token").and_then(|v| v.as_str()) {
        if !token.is_empty() {
            return Ok(token.to_string());
        }
    }
    // 2. Config.toml → env var (via zeus_core helper)
    zeus_core::resolve_discord_token().ok_or_else(|| {
        Error::Tool("No Discord bot token found in config.toml or DISCORD_BOT_TOKEN env var".to_string())
    })
}

/// Make a Discord API request
async fn discord_api(
    token: &str,
    method: &str,
    endpoint: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("{}{}", DISCORD_API, endpoint);
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
        .header("Authorization", format!("Bot {}", token))
        .header("Content-Type", "application/json");

    if let Some(b) = body {
        req = req.json(b);
    }

    let response = req
        .send()
        .await
        .map_err(|e| Error::Tool(format!("Discord API request failed: {}", e)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read response: {}", e)))?;

    if !status.is_success() {
        return Err(Error::Tool(format!(
            "Discord API error {}: {}",
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
// 1. DiscordSendMessageTool
// ---------------------------------------------------------------------------

/// Send a text message to a Discord channel
pub struct DiscordSendMessageTool;

#[async_trait]
impl TalosTool for DiscordSendMessageTool {
    fn name(&self) -> &'static str {
        "discord_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a text message to a Discord channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "channel_id",
                "string",
                "Discord channel ID to send the message to",
                true,
            )
            .with_param("content", "string", "Message text content", true)
            .with_param(
                "token",
                "string",
                "Bot token (or set DISCORD_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'content'".to_string()))?;

        let body = json!({ "content": content });
        let result = discord_api(
            &token,
            "POST",
            &format!("/channels/{}/messages", channel_id),
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
// 2. DiscordSendEmbedTool
// ---------------------------------------------------------------------------

/// Send a rich embed message to a Discord channel
pub struct DiscordSendEmbedTool;

#[async_trait]
impl TalosTool for DiscordSendEmbedTool {
    fn name(&self) -> &'static str {
        "discord_send_embed"
    }
    fn description(&self) -> &'static str {
        "Send a rich embed message to a Discord channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel_id", "string", "Discord channel ID", true)
            .with_param("title", "string", "Embed title", true)
            .with_param("description", "string", "Embed description", false)
            .with_param(
                "color",
                "integer",
                "Embed color as decimal (e.g. 5814783 for blue)",
                false,
            )
            .with_param("url", "string", "Embed URL", false)
            .with_param(
                "token",
                "string",
                "Bot token (or set DISCORD_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'title'".to_string()))?;

        let mut embed = json!({ "title": title });
        if let Some(desc) = args.get("description").and_then(|v| v.as_str()) {
            embed["description"] = json!(desc);
        }
        if let Some(color) = args.get("color").and_then(|v| v.as_i64()) {
            embed["color"] = json!(color);
        }
        if let Some(url) = args.get("url").and_then(|v| v.as_str()) {
            embed["url"] = json!(url);
        }

        let body = json!({ "embeds": [embed] });
        let result = discord_api(
            &token,
            "POST",
            &format!("/channels/{}/messages", channel_id),
            Some(&body),
        )
        .await?;

        let msg_id = result
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        Ok(format!("Embed sent (id: {})", msg_id))
    }
}

// ---------------------------------------------------------------------------
// 3. DiscordGetMessagesTool
// ---------------------------------------------------------------------------

/// Get recent messages from a Discord channel
pub struct DiscordGetMessagesTool;

#[async_trait]
impl TalosTool for DiscordGetMessagesTool {
    fn name(&self) -> &'static str {
        "discord_get_messages"
    }
    fn description(&self) -> &'static str {
        "Get recent messages from a Discord channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel_id", "string", "Discord channel ID", true)
            .with_param(
                "limit",
                "integer",
                "Max messages to return (1-100, default 10)",
                false,
            )
            .with_param(
                "token",
                "string",
                "Bot token (or set DISCORD_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .min(100);

        let endpoint = format!("/channels/{}/messages?limit={}", channel_id, limit);
        let result = discord_api(&token, "GET", &endpoint, None).await?;

        let messages = result.as_array().map(|arr| arr.as_slice()).unwrap_or(&[]);
        if messages.is_empty() {
            return Ok("No messages found.".to_string());
        }

        let mut output = format!("{} message(s):\n", messages.len());
        for msg in messages {
            let author = msg
                .get("author")
                .and_then(|a| a.get("username"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");
            let content = msg
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("[no text]");
            let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            output.push_str(&format!("[{}] {}: {}\n", id, author, content));
        }
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 4. DiscordGetChannelInfoTool
// ---------------------------------------------------------------------------

/// Get information about a Discord channel
pub struct DiscordGetChannelInfoTool;

#[async_trait]
impl TalosTool for DiscordGetChannelInfoTool {
    fn name(&self) -> &'static str {
        "discord_get_channel_info"
    }
    fn description(&self) -> &'static str {
        "Get details about a Discord channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel_id", "string", "Discord channel ID", true)
            .with_param(
                "token",
                "string",
                "Bot token (or set DISCORD_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;

        let result = discord_api(&token, "GET", &format!("/channels/{}", channel_id), None).await?;

        let name = result
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let ch_type = result.get("type").and_then(|v| v.as_i64()).unwrap_or(-1);
        let topic = result.get("topic").and_then(|v| v.as_str());
        let guild_id = result.get("guild_id").and_then(|v| v.as_str());

        let type_name = match ch_type {
            0 => "text",
            2 => "voice",
            4 => "category",
            5 => "announcement",
            10..=12 => "thread",
            13 => "stage",
            15 => "forum",
            _ => "unknown",
        };

        let mut output = format!(
            "Channel: #{}\n  Type: {}\n  ID: {}",
            name, type_name, channel_id
        );
        if let Some(g) = guild_id {
            output.push_str(&format!("\n  Guild: {}", g));
        }
        if let Some(t) = topic {
            output.push_str(&format!("\n  Topic: {}", t));
        }
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 5. DiscordSendFileTool
// ---------------------------------------------------------------------------

/// Send a file/attachment to a Discord channel
pub struct DiscordSendFileTool;

#[async_trait]
impl TalosTool for DiscordSendFileTool {
    fn name(&self) -> &'static str {
        "discord_send_file"
    }
    fn description(&self) -> &'static str {
        "Send a file attachment to a Discord channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel_id", "string", "Discord channel ID", true)
            .with_param("file_path", "string", "Local file path to upload", true)
            .with_param("content", "string", "Optional message text", false)
            .with_param(
                "token",
                "string",
                "Bot token (or set DISCORD_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'file_path'".to_string()))?;
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");

        let file_bytes = std::fs::read(file_path)
            .map_err(|e| Error::Tool(format!("Failed to read file: {}", e)))?;
        let file_name = std::path::Path::new(file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");

        let url = format!("{}/channels/{}/messages", DISCORD_API, channel_id);
        let client = reqwest::Client::new();

        let mut form = reqwest::multipart::Form::new().part(
            "files[0]",
            reqwest::multipart::Part::bytes(file_bytes).file_name(file_name.to_string()),
        );

        if !content.is_empty() {
            form = form.text("payload_json", json!({"content": content}).to_string());
        }

        let response = client
            .post(&url)
            .header("Authorization", format!("Bot {}", token))
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::Tool(format!("Request failed: {}", e)))?;

        if response.status().is_success() {
            Ok(format!(
                "File '{}' sent to channel {}",
                file_name, channel_id
            ))
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(Error::Tool(format!("Discord API error: {}", text)))
        }
    }
}

// ---------------------------------------------------------------------------
// 6. DiscordCreateThreadTool
// ---------------------------------------------------------------------------

/// Create a thread in a Discord channel
pub struct DiscordCreateThreadTool;

#[async_trait]
impl TalosTool for DiscordCreateThreadTool {
    fn name(&self) -> &'static str {
        "discord_create_thread"
    }
    fn description(&self) -> &'static str {
        "Create a new thread in a Discord channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "channel_id",
                "string",
                "Discord channel ID to create the thread in",
                true,
            )
            .with_param("name", "string", "Thread name", true)
            .with_param("message", "string", "Initial message in the thread", false)
            .with_param(
                "token",
                "string",
                "Bot token (or set DISCORD_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'name'".to_string()))?;

        let mut body = json!({
            "name": name,
            "type": 11,  // PUBLIC_THREAD
        });
        if let Some(msg) = args.get("message").and_then(|v| v.as_str()) {
            body["message"] = json!({"content": msg});
        }

        let endpoint = format!("/channels/{}/threads", channel_id);
        let result = discord_api(&token, "POST", &endpoint, Some(&body)).await?;

        let thread_id = result
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        Ok(format!("Thread '{}' created (id: {})", name, thread_id))
    }
}

// ---------------------------------------------------------------------------
// 7. DiscordAddReactionTool
// ---------------------------------------------------------------------------

/// Add a reaction to a Discord message
pub struct DiscordAddReactionTool;

#[async_trait]
impl TalosTool for DiscordAddReactionTool {
    fn name(&self) -> &'static str {
        "discord_add_reaction"
    }
    fn description(&self) -> &'static str {
        "Add a reaction emoji to a Discord message"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("channel_id", "string", "Discord channel ID", true)
            .with_param("message_id", "string", "Message ID to react to", true)
            .with_param(
                "emoji",
                "string",
                "Emoji to react with (e.g. '👍' or 'custom:123456')",
                true,
            )
            .with_param(
                "token",
                "string",
                "Bot token (or set DISCORD_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let message_id = args
            .get("message_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'message_id'".to_string()))?;
        let emoji = args
            .get("emoji")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'emoji'".to_string()))?;

        let encoded_emoji = urlencoding::encode(emoji);
        let endpoint = format!(
            "/channels/{}/messages/{}/reactions/{}/@me",
            channel_id, message_id, encoded_emoji
        );
        discord_api(&token, "PUT", &endpoint, None).await?;
        Ok(format!(
            "Reaction {} added to message {}",
            emoji, message_id
        ))
    }
}

// ---------------------------------------------------------------------------
// 8. DiscordDeleteMessageTool
// ---------------------------------------------------------------------------

/// Delete a message from a Discord channel
pub struct DiscordDeleteMessageTool;

#[async_trait]
impl TalosTool for DiscordDeleteMessageTool {
    fn name(&self) -> &'static str {
        "discord_delete_message"
    }
    fn description(&self) -> &'static str {
        "Delete a message from a Discord channel"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "channel_id",
                "string",
                "Discord channel ID containing the message",
                true,
            )
            .with_param("message_id", "string", "ID of the message to delete", true)
            .with_param(
                "token",
                "string",
                "Bot token (or set DISCORD_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let channel_id = args
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'channel_id'".to_string()))?;
        let message_id = args
            .get("message_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'message_id'".to_string()))?;

        discord_api(
            &token,
            "DELETE",
            &format!("/channels/{}/messages/{}", channel_id, message_id),
            None,
        )
        .await?;

        Ok(format!("Message {} deleted", message_id))
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
        let tool = DiscordSendMessageTool;
        assert_eq!(tool.name(), "discord_send_message");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"channel_id"));
        assert!(names.contains(&"content"));
    }

    #[test]
    fn test_send_embed_schema() {
        let tool = DiscordSendEmbedTool;
        assert_eq!(tool.name(), "discord_send_embed");
    }

    #[test]
    fn test_get_messages_schema() {
        let tool = DiscordGetMessagesTool;
        assert_eq!(tool.name(), "discord_get_messages");
    }

    #[test]
    fn test_get_channel_info_schema() {
        let tool = DiscordGetChannelInfoTool;
        assert_eq!(tool.name(), "discord_get_channel_info");
    }

    #[test]
    fn test_send_file_schema() {
        let tool = DiscordSendFileTool;
        assert_eq!(tool.name(), "discord_send_file");
    }

    #[test]
    fn test_create_thread_schema() {
        let tool = DiscordCreateThreadTool;
        assert_eq!(tool.name(), "discord_create_thread");
    }

    #[test]
    fn test_add_reaction_schema() {
        let tool = DiscordAddReactionTool;
        assert_eq!(tool.name(), "discord_add_reaction");
    }

    #[test]
    fn test_get_token_from_args() {
        let args = json!({"token": "test-token"});
        let token = get_token(&args).expect("should succeed");
        assert_eq!(token, "test-token");
    }

    #[test]
    fn test_delete_message_schema() {
        let tool = DiscordDeleteMessageTool;
        assert_eq!(tool.name(), "discord_delete_message");
    }
}
