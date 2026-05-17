//! Telegram Bot API tools
//!
//! Provides tools for interacting with the Telegram Bot API via curl.
//! Each tool accepts an optional `token` parameter, falling back to the
//! `TELEGRAM_BOT_TOKEN` environment variable.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::process::Command;
use zeus_core::{Error, Result, ToolSchema};

/// Call Telegram Bot API via curl
async fn telegram_api(token: &str, method: &str, params: &Value) -> Result<Value> {
    let url = format!("https://api.telegram.org/bot{}/{}", token, method);

    let mut cmd = Command::new("curl");
    cmd.arg("-s")
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-d")
        .arg(params.to_string())
        .arg(&url);

    let output = cmd
        .output()
        .await
        .map_err(|e| Error::Tool(format!("Failed to call Telegram API: {}", e)))?;

    if !output.status.success() {
        return Err(Error::Tool(format!(
            "curl error: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let response: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| Error::Tool(format!("Invalid JSON response: {}", e)))?;

    if response.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        Ok(response["result"].clone())
    } else {
        let desc = response
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        Err(Error::Tool(format!("Telegram API error: {}", desc)))
    }
}

/// Check if a value is a local file path (exists on disk)
fn is_local_file(value: &str) -> bool {
    !value.starts_with("http://")
        && !value.starts_with("https://")
        && std::path::Path::new(value).is_file()
}

/// Call Telegram Bot API via curl multipart form-data (for local file uploads)
async fn telegram_api_upload(
    token: &str,
    method: &str,
    file_field: &str,
    file_path: &str,
    extra_params: &[(&str, &str)],
) -> Result<Value> {
    let url = format!("https://api.telegram.org/bot{}/{}", token, method);

    let mut cmd = Command::new("curl");
    cmd.arg("-s");

    // Add file as multipart field
    cmd.arg("-F").arg(format!("{}=@{}", file_field, file_path));

    // Add other params as form fields
    for (key, value) in extra_params {
        cmd.arg("-F").arg(format!("{}={}", key, value));
    }

    cmd.arg(&url);

    let output = cmd
        .output()
        .await
        .map_err(|e| Error::Tool(format!("Failed to call Telegram API: {}", e)))?;

    if !output.status.success() {
        return Err(Error::Tool(format!(
            "curl error: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let response: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| Error::Tool(format!("Invalid JSON response: {}", e)))?;

    if response.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        Ok(response["result"].clone())
    } else {
        let desc = response
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        Err(Error::Tool(format!("Telegram API error: {}", desc)))
    }
}

/// Get bot token from args or environment
fn get_token(args: &Value) -> Result<String> {
    if let Some(token) = args.get("token").and_then(|v| v.as_str()) {
        return Ok(token.to_string());
    }
    std::env::var("TELEGRAM_BOT_TOKEN").map_err(|_| {
        Error::Tool("Missing 'token' parameter and TELEGRAM_BOT_TOKEN env var not set".to_string())
    })
}

// ---------------------------------------------------------------------------
// 1. TelegramSendMessageTool
// ---------------------------------------------------------------------------

/// Send a text message via Telegram Bot API
pub struct TelegramSendMessageTool;

#[async_trait]
impl TalosTool for TelegramSendMessageTool {
    fn name(&self) -> &'static str {
        "telegram_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a text message via Telegram"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("chat_id", "string", "Chat ID to send the message to", true)
            .with_param("text", "string", "Message text", true)
            .with_param(
                "token",
                "string",
                "Bot token (or set TELEGRAM_BOT_TOKEN env var)",
                false,
            )
            .with_param(
                "parse_mode",
                "string",
                "Parse mode: HTML or MarkdownV2",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;

        let chat_id = args
            .get("chat_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'chat_id' parameter".to_string()))?;

        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'text' parameter".to_string()))?;

        let mut params = json!({
            "chat_id": chat_id,
            "text": text,
        });

        if let Some(parse_mode) = args.get("parse_mode").and_then(|v| v.as_str()) {
            params["parse_mode"] = json!(parse_mode);
        }

        let result = telegram_api(&token, "sendMessage", &params).await?;

        let message_id = result
            .get("message_id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        Ok(format!(
            "Message sent successfully (message_id: {})",
            message_id
        ))
    }
}

// ---------------------------------------------------------------------------
// 2. TelegramGetUpdatesTool
// ---------------------------------------------------------------------------

/// Get recent updates/messages from Telegram Bot API
pub struct TelegramGetUpdatesTool;

#[async_trait]
impl TalosTool for TelegramGetUpdatesTool {
    fn name(&self) -> &'static str {
        "telegram_get_updates"
    }
    fn description(&self) -> &'static str {
        "Get recent messages and updates from Telegram"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "token",
                "string",
                "Bot token (or set TELEGRAM_BOT_TOKEN env var)",
                false,
            )
            .with_param(
                "limit",
                "integer",
                "Maximum number of updates (default 10)",
                false,
            )
            .with_param("offset", "integer", "Update offset for pagination", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);

        let mut params = json!({ "limit": limit });

        if let Some(offset) = args.get("offset").and_then(|v| v.as_i64()) {
            params["offset"] = json!(offset);
        }

        let result = telegram_api(&token, "getUpdates", &params).await?;

        let updates = result.as_array().map(|arr| arr.as_slice()).unwrap_or(&[]);

        if updates.is_empty() {
            return Ok("No new updates.".to_string());
        }

        let mut output = format!("Found {} update(s):\n", updates.len());

        for update in updates {
            let update_id = update
                .get("update_id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            if let Some(msg) = update.get("message") {
                let from = msg
                    .get("from")
                    .and_then(|f| f.get("first_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown");
                let chat_id = msg
                    .get("chat")
                    .and_then(|c| c.get("id"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let text = msg
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("[non-text message]");

                output.push_str(&format!(
                    "\n[{}] From: {} (chat: {})\n  {}\n",
                    update_id, from, chat_id, text
                ));
            } else if let Some(cb) = update.get("callback_query") {
                let from = cb
                    .get("from")
                    .and_then(|f| f.get("first_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown");
                let data = cb
                    .get("data")
                    .and_then(|v| v.as_str())
                    .unwrap_or("[no data]");

                output.push_str(&format!(
                    "\n[{}] Callback from: {}\n  Data: {}\n",
                    update_id, from, data
                ));
            } else {
                output.push_str(&format!("\n[{}] Other update type\n", update_id));
            }
        }

        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 3. TelegramSendPhotoTool
// ---------------------------------------------------------------------------

/// Send a photo via Telegram Bot API
pub struct TelegramSendPhotoTool;

#[async_trait]
impl TalosTool for TelegramSendPhotoTool {
    fn name(&self) -> &'static str {
        "telegram_send_photo"
    }
    fn description(&self) -> &'static str {
        "Send a photo via Telegram (local file path, URL, or file_id)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("chat_id", "string", "Chat ID to send the photo to", true)
            .with_param("photo", "string", "Local file path, URL, or file_id", true)
            .with_param("caption", "string", "Photo caption", false)
            .with_param(
                "token",
                "string",
                "Bot token (or set TELEGRAM_BOT_TOKEN env var)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;

        let chat_id = args
            .get("chat_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'chat_id' parameter".to_string()))?;

        let photo = args
            .get("photo")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'photo' parameter".to_string()))?;

        let result = if is_local_file(photo) {
            let mut extra = vec![("chat_id", chat_id)];
            let caption_val;
            if let Some(c) = args.get("caption").and_then(|v| v.as_str()) {
                caption_val = c.to_string();
                extra.push(("caption", &caption_val));
            }
            telegram_api_upload(&token, "sendPhoto", "photo", photo, &extra).await?
        } else {
            let mut params = json!({ "chat_id": chat_id, "photo": photo });
            if let Some(caption) = args.get("caption").and_then(|v| v.as_str()) {
                params["caption"] = json!(caption);
            }
            telegram_api(&token, "sendPhoto", &params).await?
        };

        let message_id = result
            .get("message_id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        Ok(format!(
            "Photo sent successfully (message_id: {})",
            message_id
        ))
    }
}

// ---------------------------------------------------------------------------
// 4. TelegramSendButtonsTool
// ---------------------------------------------------------------------------

/// Send a message with inline keyboard buttons via Telegram Bot API
pub struct TelegramSendButtonsTool;

#[async_trait]
impl TalosTool for TelegramSendButtonsTool {
    fn name(&self) -> &'static str {
        "telegram_send_buttons"
    }
    fn description(&self) -> &'static str {
        "Send a message with inline keyboard buttons via Telegram"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("chat_id", "string", "Chat ID to send the message to", true)
            .with_param("text", "string", "Message text", true)
            .with_param("buttons", "string", "JSON array of button rows, e.g. [[{\"text\":\"Click\",\"callback_data\":\"clicked\"}]]", true)
            .with_param("token", "string", "Bot token (or set TELEGRAM_BOT_TOKEN env var)", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;

        let chat_id = args
            .get("chat_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'chat_id' parameter".to_string()))?;

        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'text' parameter".to_string()))?;

        let buttons_str = args
            .get("buttons")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'buttons' parameter".to_string()))?;

        let inline_keyboard: Value = serde_json::from_str(buttons_str)
            .map_err(|e| Error::Tool(format!("Invalid buttons JSON: {}", e)))?;

        let params = json!({
            "chat_id": chat_id,
            "text": text,
            "reply_markup": {
                "inline_keyboard": inline_keyboard,
            },
        });

        let result = telegram_api(&token, "sendMessage", &params).await?;

        let message_id = result
            .get("message_id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        Ok(format!(
            "Message with buttons sent successfully (message_id: {})",
            message_id
        ))
    }
}

// ---------------------------------------------------------------------------
// 5. TelegramGetChatInfoTool
// ---------------------------------------------------------------------------

/// Get chat details via Telegram Bot API
pub struct TelegramGetChatInfoTool;

#[async_trait]
impl TalosTool for TelegramGetChatInfoTool {
    fn name(&self) -> &'static str {
        "telegram_get_chat_info"
    }
    fn description(&self) -> &'static str {
        "Get details about a Telegram chat"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("chat_id", "string", "Chat ID to get info for", true)
            .with_param(
                "token",
                "string",
                "Bot token (or set TELEGRAM_BOT_TOKEN env var)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;

        let chat_id = args
            .get("chat_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'chat_id' parameter".to_string()))?;

        let params = json!({ "chat_id": chat_id });

        let result = telegram_api(&token, "getChat", &params).await?;

        let chat_type = result
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let title = result.get("title").and_then(|v| v.as_str());
        let first_name = result.get("first_name").and_then(|v| v.as_str());
        let username = result.get("username").and_then(|v| v.as_str());
        let description = result.get("description").and_then(|v| v.as_str());
        let member_count = result.get("member_count").and_then(|v| v.as_i64());

        let id = result.get("id").and_then(|v| v.as_i64()).unwrap_or(0);

        let mut output = format!("Chat Info:\n  ID: {}\n  Type: {}", id, chat_type);

        if let Some(t) = title {
            output.push_str(&format!("\n  Title: {}", t));
        }
        if let Some(name) = first_name {
            output.push_str(&format!("\n  Name: {}", name));
        }
        if let Some(user) = username {
            output.push_str(&format!("\n  Username: @{}", user));
        }
        if let Some(desc) = description {
            output.push_str(&format!("\n  Description: {}", desc));
        }
        if let Some(count) = member_count {
            output.push_str(&format!("\n  Members: {}", count));
        }

        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 6. TelegramSendDocumentTool
// ---------------------------------------------------------------------------

/// Send a document/file via Telegram Bot API
pub struct TelegramSendDocumentTool;

#[async_trait]
impl TalosTool for TelegramSendDocumentTool {
    fn name(&self) -> &'static str {
        "telegram_send_document"
    }
    fn description(&self) -> &'static str {
        "Send a document/file via Telegram (local file path, URL, or file_id)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("chat_id", "string", "Chat ID to send the document to", true)
            .with_param(
                "document",
                "string",
                "Local file path, URL, or file_id",
                true,
            )
            .with_param("caption", "string", "Document caption", false)
            .with_param(
                "token",
                "string",
                "Bot token (or set TELEGRAM_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let chat_id = args
            .get("chat_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'chat_id'".to_string()))?;
        let document = args
            .get("document")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'document'".to_string()))?;

        let result = if is_local_file(document) {
            let mut extra = vec![("chat_id", chat_id)];
            let caption_val;
            if let Some(c) = args.get("caption").and_then(|v| v.as_str()) {
                caption_val = c.to_string();
                extra.push(("caption", &caption_val));
            }
            telegram_api_upload(&token, "sendDocument", "document", document, &extra).await?
        } else {
            let mut params = json!({ "chat_id": chat_id, "document": document });
            if let Some(caption) = args.get("caption").and_then(|v| v.as_str()) {
                params["caption"] = json!(caption);
            }
            telegram_api(&token, "sendDocument", &params).await?
        };

        let message_id = result
            .get("message_id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        Ok(format!("Document sent (message_id: {})", message_id))
    }
}

// ---------------------------------------------------------------------------
// 7. TelegramSendVoiceTool
// ---------------------------------------------------------------------------

/// Send a voice message via Telegram Bot API
pub struct TelegramSendVoiceTool;

#[async_trait]
impl TalosTool for TelegramSendVoiceTool {
    fn name(&self) -> &'static str {
        "telegram_send_voice"
    }
    fn description(&self) -> &'static str {
        "Send a voice message via Telegram (local file path, URL, or file_id — OGG/OPUS format)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("chat_id", "string", "Chat ID to send the voice to", true)
            .with_param(
                "voice",
                "string",
                "Local file path, URL, or file_id (OGG/OPUS)",
                true,
            )
            .with_param("caption", "string", "Voice message caption", false)
            .with_param("duration", "integer", "Duration in seconds", false)
            .with_param(
                "token",
                "string",
                "Bot token (or set TELEGRAM_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let chat_id = args
            .get("chat_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'chat_id'".to_string()))?;
        let voice = args
            .get("voice")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'voice'".to_string()))?;

        let result = if is_local_file(voice) {
            let mut extra = vec![("chat_id", chat_id)];
            let caption_val;
            if let Some(c) = args.get("caption").and_then(|v| v.as_str()) {
                caption_val = c.to_string();
                extra.push(("caption", &caption_val));
            }
            let duration_val;
            if let Some(d) = args.get("duration").and_then(|v| v.as_i64()) {
                duration_val = d.to_string();
                extra.push(("duration", &duration_val));
            }
            telegram_api_upload(&token, "sendVoice", "voice", voice, &extra).await?
        } else {
            let mut params = json!({ "chat_id": chat_id, "voice": voice });
            if let Some(caption) = args.get("caption").and_then(|v| v.as_str()) {
                params["caption"] = json!(caption);
            }
            if let Some(duration) = args.get("duration").and_then(|v| v.as_i64()) {
                params["duration"] = json!(duration);
            }
            telegram_api(&token, "sendVoice", &params).await?
        };

        let message_id = result
            .get("message_id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        Ok(format!("Voice sent (message_id: {})", message_id))
    }
}

// ---------------------------------------------------------------------------
// 8. TelegramGetMessagesTool
// ---------------------------------------------------------------------------

/// Get messages from a specific chat (using getUpdates with chat filter)
pub struct TelegramGetMessagesTool;

#[async_trait]
impl TalosTool for TelegramGetMessagesTool {
    fn name(&self) -> &'static str {
        "telegram_get_messages"
    }
    fn description(&self) -> &'static str {
        "Get recent messages from a specific Telegram chat"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("chat_id", "string", "Chat ID to filter messages from", true)
            .with_param(
                "limit",
                "integer",
                "Max messages to return (default 10)",
                false,
            )
            .with_param(
                "token",
                "string",
                "Bot token (or set TELEGRAM_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let target_chat = args
            .get("chat_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'chat_id'".to_string()))?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);

        // Fetch up to 100 updates and filter by chat_id
        let params = json!({ "limit": 100 });
        let result = telegram_api(&token, "getUpdates", &params).await?;
        let updates = result.as_array().map(|arr| arr.as_slice()).unwrap_or(&[]);

        let mut messages = Vec::new();
        for update in updates {
            if let Some(msg) = update.get("message") {
                let chat_id = msg
                    .get("chat")
                    .and_then(|c| c.get("id"))
                    .and_then(|v| v.as_i64())
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                if chat_id == target_chat {
                    let from = msg
                        .get("from")
                        .and_then(|f| f.get("first_name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown");
                    let text = msg
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("[non-text]");
                    let date = msg.get("date").and_then(|v| v.as_i64()).unwrap_or(0);
                    messages.push(format!("[{}] {}: {}", date, from, text));
                    if messages.len() >= limit as usize {
                        break;
                    }
                }
            }
        }

        if messages.is_empty() {
            Ok(format!("No messages found for chat {}", target_chat))
        } else {
            Ok(format!(
                "Messages from chat {}:\n{}",
                target_chat,
                messages.join("\n")
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// 9. TelegramCallTool (voice call via Telegram — uses sendVoice with note)
// ---------------------------------------------------------------------------

/// Initiate a voice-note "call" via Telegram (sends a voice message)
pub struct TelegramCallTool;

#[async_trait]
impl TalosTool for TelegramCallTool {
    fn name(&self) -> &'static str {
        "telegram_call"
    }
    fn description(&self) -> &'static str {
        "Send a voice note to a Telegram chat (simulated call)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("chat_id", "string", "Chat ID to call", true)
            .with_param(
                "voice_url",
                "string",
                "URL of OGG/OPUS voice file to send",
                true,
            )
            .with_param(
                "message",
                "string",
                "Text message to include with the call",
                false,
            )
            .with_param(
                "token",
                "string",
                "Bot token (or set TELEGRAM_BOT_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_token(&args)?;
        let chat_id = args
            .get("chat_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'chat_id'".to_string()))?;
        let voice_url = args
            .get("voice_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'voice_url'".to_string()))?;

        // Send optional text message first
        if let Some(msg) = args.get("message").and_then(|v| v.as_str()) {
            let text_params = json!({ "chat_id": chat_id, "text": msg });
            telegram_api(&token, "sendMessage", &text_params).await?;
        }

        // Send voice note
        let voice_params = json!({ "chat_id": chat_id, "voice": voice_url });
        let result = telegram_api(&token, "sendVoice", &voice_params).await?;
        let message_id = result
            .get("message_id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        Ok(format!(
            "Call sent to chat {} (voice message_id: {})",
            chat_id, message_id
        ))
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
        let tool = TelegramSendMessageTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "telegram_send_message");

        let params = &schema.parameters;
        let required = params
            .get("required")
            .and_then(|v| v.as_array())
            .expect("should have required array");
        let required_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(required_names.contains(&"chat_id"));
        assert!(required_names.contains(&"text"));
        assert!(!required_names.contains(&"token"));
        assert!(!required_names.contains(&"parse_mode"));
    }

    #[test]
    fn test_get_updates_schema() {
        let tool = TelegramGetUpdatesTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "telegram_get_updates");

        let params = &schema.parameters;
        let required = params.get("required").and_then(|v| v.as_array());
        // All params are optional for get_updates
        let required_names: Vec<&str> = required
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert!(!required_names.contains(&"token"));
        assert!(!required_names.contains(&"limit"));
        assert!(!required_names.contains(&"offset"));
    }

    #[test]
    fn test_send_photo_schema() {
        let tool = TelegramSendPhotoTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "telegram_send_photo");

        let params = &schema.parameters;
        let required = params
            .get("required")
            .and_then(|v| v.as_array())
            .expect("should have required array");
        let required_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(required_names.contains(&"chat_id"));
        assert!(required_names.contains(&"photo"));
        assert!(!required_names.contains(&"token"));
    }

    #[test]
    fn test_send_buttons_schema() {
        let tool = TelegramSendButtonsTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "telegram_send_buttons");

        let params = &schema.parameters;
        let props = params
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("should have properties");
        assert!(
            props.contains_key("buttons"),
            "schema must have buttons param"
        );

        let required = params
            .get("required")
            .and_then(|v| v.as_array())
            .expect("should have required array");
        let required_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(required_names.contains(&"chat_id"));
        assert!(required_names.contains(&"text"));
        assert!(required_names.contains(&"buttons"));
        assert!(!required_names.contains(&"token"));
    }

    #[test]
    fn test_get_chat_info_schema() {
        let tool = TelegramGetChatInfoTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "telegram_get_chat_info");

        let params = &schema.parameters;
        let required = params
            .get("required")
            .and_then(|v| v.as_array())
            .expect("should have required array");
        let required_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(required_names.contains(&"chat_id"));
        assert!(!required_names.contains(&"token"));
    }

    #[test]
    fn test_get_token_from_args() {
        let args = json!({ "token": "test-bot-token-123" });
        let token = get_token(&args).expect("operation should succeed");
        assert_eq!(token, "test-bot-token-123");
    }

    #[test]
    fn test_get_token_from_env_and_missing() {
        // Combined into one test to avoid parallel env var race conditions.
        // SAFETY: test runner may run tests in parallel but this test only
        // touches TELEGRAM_BOT_TOKEN which no other test uses.

        // Part 1: env var set → should succeed
        unsafe {
            std::env::set_var("TELEGRAM_BOT_TOKEN", "env-token-456");
        }
        let args = json!({});
        let token = get_token(&args).expect("operation should succeed");
        assert_eq!(token, "env-token-456");

        // Part 2: env var removed → should fail
        unsafe {
            std::env::remove_var("TELEGRAM_BOT_TOKEN");
        }
        let args = json!({});
        let result = get_token(&args);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("TELEGRAM_BOT_TOKEN"),
            "error should mention env var: {}",
            err_msg
        );
    }
}
