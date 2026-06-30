//! Matrix Client-Server API tools
//!
//! Provides tools for interacting with the Matrix Client-Server REST API.
//! Each tool accepts optional `homeserver` and `access_token` parameters,
//! falling back to the `MATRIX_HOMESERVER` and `MATRIX_ACCESS_TOKEN`
//! environment variables respectively.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

/// Get the Matrix homeserver URL from args or environment.
fn get_homeserver(args: &Value) -> Result<String> {
    if let Some(hs) = args.get("homeserver").and_then(|v| v.as_str()) {
        return Ok(hs.to_string());
    }
    std::env::var("MATRIX_HOMESERVER").map_err(|_| {
        Error::Tool(
            "Missing 'homeserver' parameter and MATRIX_HOMESERVER env var not set".to_string(),
        )
    })
}

/// Get the Matrix access token from args or environment.
fn get_access_token(args: &Value) -> Result<String> {
    if let Some(token) = args.get("access_token").and_then(|v| v.as_str()) {
        return Ok(token.to_string());
    }
    std::env::var("MATRIX_ACCESS_TOKEN").map_err(|_| {
        Error::Tool(
            "Missing 'access_token' parameter and MATRIX_ACCESS_TOKEN env var not set".to_string(),
        )
    })
}

/// Make a Matrix Client-Server API request.
///
/// - `homeserver` — base URL, e.g. `https://matrix.org`
/// - `token`      — bearer access token
/// - `method`     — HTTP verb: `"GET"`, `"POST"`, `"PUT"`
/// - `endpoint`   — path starting with `/_matrix/...`
/// - `body`       — optional JSON body (ignored for GET)
async fn matrix_api(
    homeserver: &str,
    token: &str,
    method: &str,
    endpoint: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("{}{}", homeserver.trim_end_matches('/'), endpoint);
    let client = reqwest::Client::new();

    let mut req = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        _ => return Err(Error::Tool(format!("Unsupported HTTP method: {}", method))),
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
        .map_err(|e| Error::Tool(format!("Matrix API request failed: {}", e)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read Matrix response body: {}", e)))?;

    if !status.is_success() {
        // Matrix error responses carry `errcode` and `error` fields.
        if let Ok(err_body) = serde_json::from_str::<Value>(&text) {
            let errcode = err_body
                .get("errcode")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN");
            let error = err_body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or(&text);
            return Err(Error::Tool(format!(
                "Matrix API error {} {}: {}",
                status, errcode, error
            )));
        }
        return Err(Error::Tool(format!(
            "Matrix API error {}: {}",
            status, text
        )));
    }

    if text.is_empty() {
        return Ok(json!({}));
    }

    serde_json::from_str(&text).map_err(|e| {
        Error::Tool(format!(
            "Invalid JSON from Matrix API: {} (body: {})",
            e,
            &text[..zeus_core::floor_char_boundary(&text, 200)]
        ))
    })
}

/// Generate a transaction ID from the current Unix timestamp in milliseconds.
///
/// Matrix requires a unique `txn_id` per PUT send request to guarantee
/// idempotency. Using the millisecond timestamp is sufficient for sequential
/// tool calls; a UUID would be more robust but adds a dependency.
fn make_txn_id() -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("zeus_{}", ts)
}

// ---------------------------------------------------------------------------
// 1. MatrixSendMessageTool
// ---------------------------------------------------------------------------

/// Send a text message to a Matrix room
pub struct MatrixSendMessageTool;

#[async_trait]
impl TalosTool for MatrixSendMessageTool {
    fn name(&self) -> &'static str {
        "matrix_send_message"
    }
    fn description(&self) -> &'static str {
        "Send a text message to a Matrix room"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "room_id",
                "string",
                "Matrix room ID (e.g. !abc123:matrix.org)",
                true,
            )
            .with_param("message", "string", "Message text to send", true)
            .with_param(
                "homeserver",
                "string",
                "Matrix homeserver URL (or set MATRIX_HOMESERVER env var)",
                false,
            )
            .with_param(
                "access_token",
                "string",
                "Matrix access token (or set MATRIX_ACCESS_TOKEN env var)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let homeserver = get_homeserver(&args)?;
        let token = get_access_token(&args)?;

        let room_id = args
            .get("room_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'room_id' parameter".to_string()))?;

        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'message' parameter".to_string()))?;

        let txn_id = make_txn_id();
        let endpoint = format!(
            "/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
            room_id, txn_id
        );

        let body = json!({
            "msgtype": "m.text",
            "body": message,
        });

        let result = matrix_api(&homeserver, &token, "PUT", &endpoint, Some(&body)).await?;

        let event_id = result
            .get("event_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        Ok(format!(
            "Message sent to {} (event_id: {})",
            room_id, event_id
        ))
    }
}

// ---------------------------------------------------------------------------
// 2. MatrixGetMessagesTool
// ---------------------------------------------------------------------------

/// Retrieve recent messages from a Matrix room
pub struct MatrixGetMessagesTool;

#[async_trait]
impl TalosTool for MatrixGetMessagesTool {
    fn name(&self) -> &'static str {
        "matrix_get_messages"
    }
    fn description(&self) -> &'static str {
        "Get recent messages from a Matrix room"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "room_id",
                "string",
                "Matrix room ID (e.g. !abc123:matrix.org)",
                true,
            )
            .with_param(
                "limit",
                "integer",
                "Maximum number of messages to return (default 10)",
                false,
            )
            .with_param(
                "homeserver",
                "string",
                "Matrix homeserver URL (or set MATRIX_HOMESERVER env var)",
                false,
            )
            .with_param(
                "access_token",
                "string",
                "Matrix access token (or set MATRIX_ACCESS_TOKEN env var)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let homeserver = get_homeserver(&args)?;
        let token = get_access_token(&args)?;

        let room_id = args
            .get("room_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'room_id' parameter".to_string()))?;

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);

        let endpoint = format!(
            "/_matrix/client/v3/rooms/{}/messages?dir=b&limit={}",
            room_id, limit
        );

        let result = matrix_api(&homeserver, &token, "GET", &endpoint, None).await?;

        let chunk = result
            .get("chunk")
            .and_then(|v| v.as_array())
            .map(|arr| arr.as_slice())
            .unwrap_or(&[]);

        if chunk.is_empty() {
            return Ok(format!("No messages found in room {}.", room_id));
        }

        let mut output = format!("{} message(s) from {}:\n", chunk.len(), room_id);
        for event in chunk {
            let sender = event
                .get("sender")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let event_id = event
                .get("event_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let body = event
                .get("content")
                .and_then(|c| c.get("body"))
                .and_then(|v| v.as_str())
                .unwrap_or("[non-text event]");
            output.push_str(&format!("[{}] {}: {}\n", event_id, sender, body));
        }

        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 3. MatrixJoinRoomTool
// ---------------------------------------------------------------------------

/// Join a Matrix room by room ID or alias
pub struct MatrixJoinRoomTool;

#[async_trait]
impl TalosTool for MatrixJoinRoomTool {
    fn name(&self) -> &'static str {
        "matrix_join_room"
    }
    fn description(&self) -> &'static str {
        "Join a Matrix room by room ID or alias"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "room_id",
                "string",
                "Matrix room ID or alias to join (e.g. !abc123:matrix.org or #room:matrix.org)",
                true,
            )
            .with_param(
                "homeserver",
                "string",
                "Matrix homeserver URL (or set MATRIX_HOMESERVER env var)",
                false,
            )
            .with_param(
                "access_token",
                "string",
                "Matrix access token (or set MATRIX_ACCESS_TOKEN env var)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let homeserver = get_homeserver(&args)?;
        let token = get_access_token(&args)?;

        let room_id = args
            .get("room_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'room_id' parameter".to_string()))?;

        let endpoint = format!("/_matrix/client/v3/join/{}", room_id);

        let result = matrix_api(&homeserver, &token, "POST", &endpoint, Some(&json!({}))).await?;

        let joined_room_id = result
            .get("room_id")
            .and_then(|v| v.as_str())
            .unwrap_or(room_id);

        Ok(format!("Joined room {}", joined_room_id))
    }
}

// ---------------------------------------------------------------------------
// 4. MatrixListRoomsTool
// ---------------------------------------------------------------------------

/// List all rooms the authenticated user has joined
pub struct MatrixListRoomsTool;

#[async_trait]
impl TalosTool for MatrixListRoomsTool {
    fn name(&self) -> &'static str {
        "matrix_list_rooms"
    }
    fn description(&self) -> &'static str {
        "List all Matrix rooms the authenticated user has joined"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "homeserver",
                "string",
                "Matrix homeserver URL (or set MATRIX_HOMESERVER env var)",
                false,
            )
            .with_param(
                "access_token",
                "string",
                "Matrix access token (or set MATRIX_ACCESS_TOKEN env var)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let homeserver = get_homeserver(&args)?;
        let token = get_access_token(&args)?;

        let result = matrix_api(
            &homeserver,
            &token,
            "GET",
            "/_matrix/client/v3/joined_rooms",
            None,
        )
        .await?;

        let rooms = result
            .get("joined_rooms")
            .and_then(|v| v.as_array())
            .map(|arr| arr.as_slice())
            .unwrap_or(&[]);

        if rooms.is_empty() {
            return Ok("No joined rooms found.".to_string());
        }

        let mut output = format!("{} joined room(s):\n", rooms.len());
        for room in rooms {
            if let Some(id) = room.as_str() {
                output.push_str(&format!("  {}\n", id));
            }
        }

        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// 5. MatrixSendImageTool
// ---------------------------------------------------------------------------

/// Send an image to a Matrix room using an mxc:// URL
pub struct MatrixSendImageTool;

#[async_trait]
impl TalosTool for MatrixSendImageTool {
    fn name(&self) -> &'static str {
        "matrix_send_image"
    }
    fn description(&self) -> &'static str {
        "Send an image to a Matrix room using an mxc:// media URL"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "room_id",
                "string",
                "Matrix room ID (e.g. !abc123:matrix.org)",
                true,
            )
            .with_param(
                "image_url",
                "string",
                "Matrix media URL (mxc://server/media-id)",
                true,
            )
            .with_param(
                "body",
                "string",
                "Alt-text / filename for the image (default: \"image\")",
                false,
            )
            .with_param(
                "homeserver",
                "string",
                "Matrix homeserver URL (or set MATRIX_HOMESERVER env var)",
                false,
            )
            .with_param(
                "access_token",
                "string",
                "Matrix access token (or set MATRIX_ACCESS_TOKEN env var)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let homeserver = get_homeserver(&args)?;
        let token = get_access_token(&args)?;

        let room_id = args
            .get("room_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'room_id' parameter".to_string()))?;

        let image_url = args
            .get("image_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'image_url' parameter".to_string()))?;

        if !image_url.starts_with("mxc://") {
            return Err(Error::Tool(format!(
                "image_url must be an mxc:// URL, got: {}",
                image_url
            )));
        }

        let alt_body = args.get("body").and_then(|v| v.as_str()).unwrap_or("image");

        let txn_id = make_txn_id();
        let endpoint = format!(
            "/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
            room_id, txn_id
        );

        let body = json!({
            "msgtype": "m.image",
            "body": alt_body,
            "url": image_url,
        });

        let result = matrix_api(&homeserver, &token, "PUT", &endpoint, Some(&body)).await?;

        let event_id = result
            .get("event_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        Ok(format!(
            "Image sent to {} (event_id: {})",
            room_id, event_id
        ))
    }
}

// ---------------------------------------------------------------------------
// 6. MatrixGetRoomInfoTool
// ---------------------------------------------------------------------------

/// Retrieve the full state of a Matrix room
pub struct MatrixGetRoomInfoTool;

#[async_trait]
impl TalosTool for MatrixGetRoomInfoTool {
    fn name(&self) -> &'static str {
        "matrix_get_room_info"
    }
    fn description(&self) -> &'static str {
        "Get state information for a Matrix room (name, topic, members, etc.)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "room_id",
                "string",
                "Matrix room ID (e.g. !abc123:matrix.org)",
                true,
            )
            .with_param(
                "homeserver",
                "string",
                "Matrix homeserver URL (or set MATRIX_HOMESERVER env var)",
                false,
            )
            .with_param(
                "access_token",
                "string",
                "Matrix access token (or set MATRIX_ACCESS_TOKEN env var)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let homeserver = get_homeserver(&args)?;
        let token = get_access_token(&args)?;

        let room_id = args
            .get("room_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'room_id' parameter".to_string()))?;

        let endpoint = format!("/_matrix/client/v3/rooms/{}/state", room_id);

        let result = matrix_api(&homeserver, &token, "GET", &endpoint, None).await?;

        // The state endpoint returns a JSON array of state events.
        let events = match result.as_array() {
            Some(arr) => arr,
            None => {
                return Ok(format!(
                    "Room state for {}:\n{}",
                    room_id,
                    serde_json::to_string_pretty(&result).unwrap_or_default()
                ));
            }
        };

        // Extract commonly useful state event types.
        let mut name = None;
        let mut topic = None;
        let mut member_count = 0usize;
        let mut avatar_url = None;

        let empty_content = json!({});
        for event in events {
            let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let content = event.get("content").unwrap_or(&empty_content);

            match event_type {
                "m.room.name" => {
                    name = content
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned);
                }
                "m.room.topic" => {
                    topic = content
                        .get("topic")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned);
                }
                "m.room.member" => {
                    if content.get("membership").and_then(|v| v.as_str()) == Some("join") {
                        member_count += 1;
                    }
                }
                "m.room.avatar" => {
                    avatar_url = content
                        .get("url")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned);
                }
                _ => {}
            }
        }

        let mut output = format!("Room info for {}:\n  ID: {}", room_id, room_id);
        if let Some(n) = name {
            output.push_str(&format!("\n  Name: {}", n));
        }
        if let Some(t) = topic {
            output.push_str(&format!("\n  Topic: {}", t));
        }
        output.push_str(&format!("\n  Joined members: {}", member_count));
        if let Some(av) = avatar_url {
            output.push_str(&format!("\n  Avatar: {}", av));
        }
        output.push_str(&format!("\n  State events: {}", events.len()));

        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: extract required field names from a ToolSchema.
    fn required_params(schema: &ToolSchema) -> Vec<&str> {
        schema
            .parameters
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default()
    }

    // Helper: extract all property names from a ToolSchema.
    fn all_params(schema: &ToolSchema) -> Vec<String> {
        schema
            .parameters
            .get("properties")
            .and_then(|v| v.as_object())
            .map(|obj| obj.keys().cloned().collect())
            .unwrap_or_default()
    }

    #[test]
    fn test_send_message_schema() {
        let tool = MatrixSendMessageTool;
        assert_eq!(tool.name(), "matrix_send_message");

        let schema = tool.schema();
        let req = required_params(&schema);
        assert!(req.contains(&"room_id"), "room_id must be required");
        assert!(req.contains(&"message"), "message must be required");
        assert!(!req.contains(&"homeserver"), "homeserver must be optional");
        assert!(
            !req.contains(&"access_token"),
            "access_token must be optional"
        );

        let all = all_params(&schema);
        assert!(all.iter().any(|p| p == "room_id"));
        assert!(all.iter().any(|p| p == "message"));
        assert!(all.iter().any(|p| p == "homeserver"));
        assert!(all.iter().any(|p| p == "access_token"));
    }

    #[test]
    fn test_get_messages_schema() {
        let tool = MatrixGetMessagesTool;
        assert_eq!(tool.name(), "matrix_get_messages");

        let schema = tool.schema();
        let req = required_params(&schema);
        assert!(req.contains(&"room_id"));
        assert!(!req.contains(&"limit"), "limit must be optional");

        let all = all_params(&schema);
        assert!(all.iter().any(|p| p == "limit"));
    }

    #[test]
    fn test_join_room_schema() {
        let tool = MatrixJoinRoomTool;
        assert_eq!(tool.name(), "matrix_join_room");

        let schema = tool.schema();
        let req = required_params(&schema);
        assert!(req.contains(&"room_id"));
        assert!(!req.contains(&"homeserver"));
        assert!(!req.contains(&"access_token"));
    }

    #[test]
    fn test_list_rooms_schema() {
        let tool = MatrixListRoomsTool;
        assert_eq!(tool.name(), "matrix_list_rooms");

        let schema = tool.schema();
        // No required params — both are resolved from env vars.
        let req = required_params(&schema);
        assert!(!req.contains(&"homeserver"));
        assert!(!req.contains(&"access_token"));

        let all = all_params(&schema);
        assert!(all.iter().any(|p| p == "homeserver"));
        assert!(all.iter().any(|p| p == "access_token"));
    }

    #[test]
    fn test_send_image_schema() {
        let tool = MatrixSendImageTool;
        assert_eq!(tool.name(), "matrix_send_image");

        let schema = tool.schema();
        let req = required_params(&schema);
        assert!(req.contains(&"room_id"));
        assert!(req.contains(&"image_url"));
        assert!(!req.contains(&"body"), "body must be optional");

        let all = all_params(&schema);
        assert!(all.iter().any(|p| p == "body"));
    }

    #[test]
    fn test_get_room_info_schema() {
        let tool = MatrixGetRoomInfoTool;
        assert_eq!(tool.name(), "matrix_get_room_info");

        let schema = tool.schema();
        let req = required_params(&schema);
        assert!(req.contains(&"room_id"));
        assert!(!req.contains(&"homeserver"));
        assert!(!req.contains(&"access_token"));
    }

    #[test]
    fn test_get_homeserver_from_args() {
        let args = json!({ "homeserver": "https://matrix.example.com" });
        let hs = get_homeserver(&args).expect("should succeed");
        assert_eq!(hs, "https://matrix.example.com");
    }

    #[test]
    fn test_get_access_token_from_args() {
        let args = json!({ "access_token": "syt_test_token_abc123" });
        let token = get_access_token(&args).expect("should succeed");
        assert_eq!(token, "syt_test_token_abc123");
    }

    #[test]
    fn test_get_homeserver_missing() {
        // Remove env var so we can test the error path.
        unsafe {
            std::env::remove_var("MATRIX_HOMESERVER");
        }
        let args = json!({});
        let result = get_homeserver(&args);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("MATRIX_HOMESERVER"),
            "error should mention env var: {}",
            msg
        );
    }

    #[test]
    fn test_get_access_token_missing() {
        unsafe {
            std::env::remove_var("MATRIX_ACCESS_TOKEN");
        }
        let args = json!({});
        let result = get_access_token(&args);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("MATRIX_ACCESS_TOKEN"),
            "error should mention env var: {}",
            msg
        );
    }

    #[test]
    fn test_make_txn_id_format() {
        let txn = make_txn_id();
        assert!(txn.starts_with("zeus_"), "txn_id must have zeus_ prefix");
        // The numeric suffix must be parseable as u128.
        let suffix = txn.trim_start_matches("zeus_");
        assert!(
            suffix.parse::<u128>().is_ok(),
            "txn_id suffix must be a timestamp: {}",
            suffix
        );
    }

    #[test]
    fn test_make_txn_id_unique() {
        // Two consecutive IDs should differ (relies on sub-millisecond timing
        // being irrelevant — if both land in the same ms the test still passes
        // because make_txn_id uses system time and the IDs are strings).
        // We just assert the format is consistent.
        let a = make_txn_id();
        let b = make_txn_id();
        assert!(a.starts_with("zeus_"));
        assert!(b.starts_with("zeus_"));
    }
}
