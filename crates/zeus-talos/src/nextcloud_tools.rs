//! Nextcloud Talk API tools
//!
//! Provides tools for interacting with the Nextcloud Talk (Spreed) API.
//! Each tool accepts optional authentication parameters, falling back to
//! `NEXTCLOUD_URL`, `NEXTCLOUD_USERNAME`, `NEXTCLOUD_PASSWORD`, or
//! `NEXTCLOUD_TOKEN` environment variables.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

/// Get Nextcloud base URL from args or environment
fn get_base_url(args: &Value) -> Result<String> {
    if let Some(url) = args.get("url").and_then(|v| v.as_str()) {
        return Ok(url.trim_end_matches('/').to_string());
    }
    std::env::var("NEXTCLOUD_URL")
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|_| {
            Error::Tool("Missing 'url' parameter and NEXTCLOUD_URL env var not set".to_string())
        })
}

/// Authentication credentials for Nextcloud
enum NextcloudAuth {
    Basic { username: String, password: String },
    Bearer { token: String },
}

/// Get auth credentials from args or environment
fn get_auth(args: &Value) -> Result<NextcloudAuth> {
    // Check for token-based auth first (app passwords)
    if let Some(token) = args.get("token").and_then(|v| v.as_str()) {
        return Ok(NextcloudAuth::Bearer {
            token: token.to_string(),
        });
    }
    if let Ok(token) = std::env::var("NEXTCLOUD_TOKEN") {
        return Ok(NextcloudAuth::Bearer { token });
    }

    // Fall back to basic auth
    let username = args
        .get("username")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("NEXTCLOUD_USERNAME").ok());
    let password = args
        .get("password")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("NEXTCLOUD_PASSWORD").ok());

    match (username, password) {
        (Some(u), Some(p)) => Ok(NextcloudAuth::Basic {
            username: u,
            password: p,
        }),
        _ => Err(Error::Tool(
            "Missing authentication: provide 'token' or 'username'+'password' params, \
             or set NEXTCLOUD_TOKEN or NEXTCLOUD_USERNAME+NEXTCLOUD_PASSWORD env vars"
                .to_string(),
        )),
    }
}

/// Make a Nextcloud OCS API request
async fn nextcloud_api(
    base_url: &str,
    auth: &NextcloudAuth,
    method: &str,
    endpoint: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("{}{}", base_url, endpoint);
    let client = reqwest::Client::new();

    let mut req = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        _ => return Err(Error::Tool(format!("Unsupported method: {}", method))),
    };

    // OCS API required headers
    req = req
        .header("Accept", "application/json")
        .header("OCS-APIRequest", "true")
        .header("Content-Type", "application/json");

    // Apply authentication
    req = match auth {
        NextcloudAuth::Basic { username, password } => req.basic_auth(username, Some(password)),
        NextcloudAuth::Bearer { token } => req.bearer_auth(token),
    };

    if let Some(b) = body {
        req = req.json(b);
    }

    let response = req
        .send()
        .await
        .map_err(|e| Error::Tool(format!("Nextcloud API request failed: {}", e)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read response: {}", e)))?;

    if !status.is_success() {
        return Err(Error::Tool(format!(
            "Nextcloud API error {}: {}",
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
// 1. NextcloudSendMessageTool
// ---------------------------------------------------------------------------

/// Send a message to a Nextcloud Talk conversation
pub struct NextcloudSendMessageTool;

#[async_trait]
impl TalosTool for NextcloudSendMessageTool {
    fn name(&self) -> &'static str {
        "nextcloud_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a message to a Nextcloud Talk conversation"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "room_token",
                "string",
                "Nextcloud Talk room/conversation token",
                true,
            )
            .with_param("message", "string", "Message text to send", true)
            .with_param(
                "url",
                "string",
                "Nextcloud base URL (or set NEXTCLOUD_URL env var)",
                false,
            )
            .with_param(
                "username",
                "string",
                "Nextcloud username (or set NEXTCLOUD_USERNAME env var)",
                false,
            )
            .with_param(
                "password",
                "string",
                "Nextcloud password (or set NEXTCLOUD_PASSWORD env var)",
                false,
            )
            .with_param(
                "token",
                "string",
                "App password/token (or set NEXTCLOUD_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_base_url(&args)?;
        let auth = get_auth(&args)?;
        let room_token = args
            .get("room_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'room_token'".to_string()))?;
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'message'".to_string()))?;

        let endpoint = format!("/ocs/v2.php/apps/spreed/api/v1/chat/{}", room_token);
        let body = json!({ "message": message });
        let result = nextcloud_api(&base_url, &auth, "POST", &endpoint, Some(&body)).await?;

        let msg_id = result
            .get("ocs")
            .and_then(|o| o.get("data"))
            .and_then(|d| d.get("id"))
            .and_then(|v| v.as_i64())
            .map(|id| id.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        Ok(format!(
            "Message sent to room {} (id: {})",
            room_token, msg_id
        ))
    }
}

// ---------------------------------------------------------------------------
// 2. NextcloudGetMessagesTool
// ---------------------------------------------------------------------------

/// Get recent messages from a Nextcloud Talk conversation
pub struct NextcloudGetMessagesTool;

#[async_trait]
impl TalosTool for NextcloudGetMessagesTool {
    fn name(&self) -> &'static str {
        "nextcloud_get_messages"
    }
    fn description(&self) -> &'static str {
        "Get recent messages from a Nextcloud Talk conversation"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "room_token",
                "string",
                "Nextcloud Talk room/conversation token",
                true,
            )
            .with_param(
                "limit",
                "integer",
                "Max messages to return (default 20, max 200)",
                false,
            )
            .with_param(
                "url",
                "string",
                "Nextcloud base URL (or set NEXTCLOUD_URL env var)",
                false,
            )
            .with_param(
                "username",
                "string",
                "Nextcloud username (or set NEXTCLOUD_USERNAME env var)",
                false,
            )
            .with_param(
                "password",
                "string",
                "Nextcloud password (or set NEXTCLOUD_PASSWORD env var)",
                false,
            )
            .with_param(
                "token",
                "string",
                "App password/token (or set NEXTCLOUD_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_base_url(&args)?;
        let auth = get_auth(&args)?;
        let room_token = args
            .get("room_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'room_token'".to_string()))?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .min(200);

        let endpoint = format!(
            "/ocs/v2.php/apps/spreed/api/v1/chat/{}?limit={}&lookIntoFuture=0",
            room_token, limit
        );
        let result = nextcloud_api(&base_url, &auth, "GET", &endpoint, None).await?;

        let messages = result
            .get("ocs")
            .and_then(|o| o.get("data"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.as_slice())
            .unwrap_or(&[]);

        if messages.is_empty() {
            return Ok("No messages found.".to_string());
        }

        let mut output = format!("{} message(s):\n", messages.len());
        for msg in messages {
            let actor = msg
                .get("actorDisplayName")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");
            let text = msg
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("[no text]");
            let id = msg
                .get("id")
                .and_then(|v| v.as_i64())
                .map(|i| i.to_string())
                .unwrap_or_else(|| "?".to_string());
            let timestamp = msg.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
            output.push_str(&format!("[{}] {} ({}): {}\n", id, actor, timestamp, text));
        }
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 3. NextcloudListRoomsTool
// ---------------------------------------------------------------------------

/// List Nextcloud Talk rooms/conversations
pub struct NextcloudListRoomsTool;

#[async_trait]
impl TalosTool for NextcloudListRoomsTool {
    fn name(&self) -> &'static str {
        "nextcloud_list_rooms"
    }
    fn description(&self) -> &'static str {
        "List Nextcloud Talk rooms and conversations"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "url",
                "string",
                "Nextcloud base URL (or set NEXTCLOUD_URL env var)",
                false,
            )
            .with_param(
                "username",
                "string",
                "Nextcloud username (or set NEXTCLOUD_USERNAME env var)",
                false,
            )
            .with_param(
                "password",
                "string",
                "Nextcloud password (or set NEXTCLOUD_PASSWORD env var)",
                false,
            )
            .with_param(
                "token",
                "string",
                "App password/token (or set NEXTCLOUD_TOKEN env var)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_base_url(&args)?;
        let auth = get_auth(&args)?;

        let endpoint = "/ocs/v2.php/apps/spreed/api/v4/room";
        let result = nextcloud_api(&base_url, &auth, "GET", endpoint, None).await?;

        let rooms = result
            .get("ocs")
            .and_then(|o| o.get("data"))
            .and_then(|v| v.as_array())
            .map(|arr| arr.as_slice())
            .unwrap_or(&[]);

        if rooms.is_empty() {
            return Ok("No rooms found.".to_string());
        }

        let mut output = format!("{} room(s):\n", rooms.len());
        for room in rooms {
            let name = room
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let token = room.get("token").and_then(|v| v.as_str()).unwrap_or("?");
            let room_type = room.get("type").and_then(|v| v.as_i64()).unwrap_or(-1);
            let participants = room
                .get("participantCount")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let type_name = match room_type {
                1 => "one-to-one",
                2 => "group",
                3 => "public",
                4 => "changelog",
                5 => "former-one-to-one",
                6 => "note-to-self",
                _ => "unknown",
            };

            output.push_str(&format!(
                "  {} ({}) — {} — {} participant(s)\n",
                name, token, type_name, participants
            ));
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
        let tool = NextcloudSendMessageTool;
        assert_eq!(tool.name(), "nextcloud_send_message");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"room_token"));
        assert!(names.contains(&"message"));
    }

    #[test]
    fn test_get_messages_schema() {
        let tool = NextcloudGetMessagesTool;
        assert_eq!(tool.name(), "nextcloud_get_messages");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"room_token"));
    }

    #[test]
    fn test_list_rooms_schema() {
        let tool = NextcloudListRoomsTool;
        assert_eq!(tool.name(), "nextcloud_list_rooms");
    }

    #[test]
    fn test_get_base_url_from_args() {
        let args = json!({"url": "https://cloud.example.com/"});
        let url = get_base_url(&args).expect("should succeed");
        assert_eq!(url, "https://cloud.example.com");
    }

    #[test]
    fn test_get_auth_bearer_from_args() {
        let args = json!({"token": "app-password-123"});
        let auth = get_auth(&args).expect("should succeed");
        match auth {
            NextcloudAuth::Bearer { token } => assert_eq!(token, "app-password-123"),
            _ => panic!("Expected Bearer auth"),
        }
    }

    #[test]
    fn test_get_auth_basic_from_args() {
        let args = json!({"username": "admin", "password": "secret"});
        let auth = get_auth(&args).expect("should succeed");
        match auth {
            NextcloudAuth::Basic { username, password } => {
                assert_eq!(username, "admin");
                assert_eq!(password, "secret");
            }
            _ => panic!("Expected Basic auth"),
        }
    }
}
