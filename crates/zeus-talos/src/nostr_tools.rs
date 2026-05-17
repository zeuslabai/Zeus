//! Nostr Protocol tools
//!
//! Provides tools for interacting with the Nostr network via CLI (`nostril`)
//! or by constructing NIP-01 event JSON structures.
//! Each tool accepts optional configuration parameters, falling back to
//! `NOSTR_RELAY_URL` and `NOSTR_PRIVATE_KEY` environment variables.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

const DEFAULT_RELAY: &str = "wss://relay.damus.io";

/// Validate a relay URL — must start with ws:// or wss:// and contain no shell metacharacters.
fn validate_relay_url(url: &str) -> std::result::Result<(), String> {
    if !url.starts_with("ws://") && !url.starts_with("wss://") {
        return Err(format!(
            "Invalid relay URL scheme (must be ws:// or wss://): {}",
            url
        ));
    }
    // Reject URLs containing shell metacharacters beyond what single-quote escaping handles
    let dangerous = ['`', '$', '(', ')', '!', '\n', '\r', '\0'];
    if url.chars().any(|c| dangerous.contains(&c)) {
        return Err(format!("Relay URL contains forbidden characters: {}", url));
    }
    Ok(())
}

/// Get relay URL from args or environment
fn get_relay(args: &Value) -> String {
    if let Some(relay) = args.get("relay_url").and_then(|v| v.as_str()) {
        return relay.to_string();
    }
    std::env::var("NOSTR_RELAY_URL").unwrap_or_else(|_| DEFAULT_RELAY.to_string())
}

/// Get private key (hex) from args or environment
fn get_private_key(args: &Value) -> Result<String> {
    if let Some(key) = args.get("private_key").and_then(|v| v.as_str()) {
        return Ok(key.to_string());
    }
    std::env::var("NOSTR_PRIVATE_KEY").map_err(|_| {
        Error::Tool(
            "Missing 'private_key' parameter and NOSTR_PRIVATE_KEY env var not set".to_string(),
        )
    })
}

/// Check if the `nostril` CLI tool is available
async fn nostril_available() -> bool {
    tokio::process::Command::new("which")
        .arg("nostril")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// 1. NostrPublishNoteTool
// ---------------------------------------------------------------------------

/// Publish a kind:1 text note to the Nostr network
pub struct NostrPublishNoteTool;

#[async_trait]
impl TalosTool for NostrPublishNoteTool {
    fn name(&self) -> &'static str {
        "nostr_publish_note"
    }
    fn description(&self) -> &'static str {
        "Publish a kind:1 text note to the Nostr network"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("content", "string", "Text content for the note", true)
            .with_param(
                "private_key",
                "string",
                "Nostr private key in hex (or set NOSTR_PRIVATE_KEY env var)",
                false,
            )
            .with_param(
                "relay_url",
                "string",
                "Relay URL (or set NOSTR_RELAY_URL, default: wss://relay.damus.io)",
                false,
            )
            .with_param(
                "tags",
                "string",
                "Comma-separated tags to include (e.g. 'p:pubkey,e:eventid')",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'content'".to_string()))?;
        let relay = get_relay(&args);
        validate_relay_url(&relay).map_err(Error::Tool)?;

        if nostril_available().await {
            // Use nostril CLI for signing and publishing
            let secret = get_private_key(&args)?;
            let mut cmd = tokio::process::Command::new("nostril");
            cmd.arg("--content").arg(content).arg("--sec").arg(&secret);

            // Add tags if provided
            if let Some(tags_str) = args.get("tags").and_then(|v| v.as_str()) {
                for tag in tags_str.split(',') {
                    let parts: Vec<&str> = tag.trim().splitn(2, ':').collect();
                    if parts.len() == 2 {
                        cmd.arg(format!("--tag-{}", parts[0])).arg(parts[1]);
                    }
                }
            }

            let output = cmd
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to execute nostril: {}", e)))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(Error::Tool(format!("nostril failed: {}", stderr)));
            }

            let event_json = String::from_utf8_lossy(&output.stdout).trim().to_string();

            // Attempt to send to relay via nostril pipe or websocat
            // Sanitize both event JSON and relay URL to prevent shell injection
            let send_result = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "echo '{}' | websocat -1 '{}'",
                    event_json.replace('\'', "'\\''"),
                    relay.replace('\'', "'\\''")
                ))
                .output()
                .await;

            match send_result {
                Ok(out) if out.status.success() => {
                    // Try to extract event ID from the JSON
                    let event_id = serde_json::from_str::<Value>(&event_json)
                        .ok()
                        .and_then(|v| v.get("id").and_then(|id| id.as_str().map(String::from)))
                        .unwrap_or_else(|| "generated".to_string());
                    Ok(format!(
                        "Note published to {} (event id: {})",
                        relay, event_id
                    ))
                }
                _ => {
                    // Could not send but event was created
                    Ok(format!(
                        "Event created (relay send may have failed):\n{}",
                        event_json
                    ))
                }
            }
        } else {
            // No nostril available -- construct unsigned NIP-01 event JSON
            let created_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let mut tags: Vec<Value> = Vec::new();
            if let Some(tags_str) = args.get("tags").and_then(|v| v.as_str()) {
                for tag in tags_str.split(',') {
                    let parts: Vec<&str> = tag.trim().splitn(2, ':').collect();
                    if parts.len() == 2 {
                        tags.push(json!([parts[0], parts[1]]));
                    }
                }
            }

            let pubkey = if let Ok(key) = get_private_key(&args) {
                // Use first 64 chars as a placeholder pubkey representation
                if key.len() >= 64 {
                    key[..64].to_string()
                } else {
                    key
                }
            } else {
                "unsigned".to_string()
            };

            let event = json!({
                "id": "<to-be-computed>",
                "pubkey": pubkey,
                "created_at": created_at,
                "kind": 1,
                "tags": tags,
                "content": content,
                "sig": "<to-be-signed>"
            });

            Ok(format!(
                "Unsigned NIP-01 event created (nostril not found, install for signing):\n{}",
                serde_json::to_string_pretty(&event).unwrap_or_default()
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// 2. NostrGetEventsTool
// ---------------------------------------------------------------------------

/// Query events from a Nostr relay
pub struct NostrGetEventsTool;

#[async_trait]
impl TalosTool for NostrGetEventsTool {
    fn name(&self) -> &'static str {
        "nostr_get_events"
    }
    fn description(&self) -> &'static str {
        "Query events from a Nostr relay using REQ filters"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "kinds",
                "string",
                "Comma-separated event kinds to filter (default: '1' for text notes)",
                false,
            )
            .with_param(
                "limit",
                "integer",
                "Max events to return (default 10)",
                false,
            )
            .with_param(
                "authors",
                "string",
                "Comma-separated pubkey hex strings to filter by",
                false,
            )
            .with_param(
                "relay_url",
                "string",
                "Relay URL (or set NOSTR_RELAY_URL, default: wss://relay.damus.io)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let relay = get_relay(&args);
        validate_relay_url(&relay).map_err(Error::Tool)?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);

        let kinds: Vec<u64> = args
            .get("kinds")
            .and_then(|v| v.as_str())
            .unwrap_or("1")
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        let mut filter = json!({
            "kinds": kinds,
            "limit": limit
        });

        if let Some(authors_str) = args.get("authors").and_then(|v| v.as_str()) {
            let authors: Vec<&str> = authors_str.split(',').map(|s| s.trim()).collect();
            filter["authors"] = json!(authors);
        }

        // Build NIP-01 REQ message
        let req_msg = json!(["REQ", "zeus-query", filter]);

        if nostril_available().await {
            // Try using websocat to send REQ and receive events
            let req_json = serde_json::to_string(&req_msg)
                .map_err(|e| Error::Tool(format!("Failed to serialize REQ: {}", e)))?;

            // Sanitize relay URL to prevent shell injection
            let output = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "echo '{}' | websocat -t -1 '{}' 2>/dev/null | head -{}",
                    req_json.replace('\'', "'\\''"),
                    relay.replace('\'', "'\\''"),
                    limit + 1 // +1 for EOSE
                ))
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to query relay: {}", e)))?;

            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if stdout.is_empty() {
                    return Ok("No events returned from relay.".to_string());
                }

                let mut events = Vec::new();
                for line in stdout.lines() {
                    if let Ok(msg) = serde_json::from_str::<Value>(line)
                        && let Some(arr) = msg.as_array()
                        && arr.len() >= 3
                        && arr[0].as_str() == Some("EVENT")
                    {
                        events.push(arr[2].clone());
                    }
                }

                if events.is_empty() {
                    return Ok(format!(
                        "Query sent to {} but no EVENT messages received.",
                        relay
                    ));
                }

                let mut output_str = format!("{} event(s) from {}:\n", events.len(), relay);
                for event in &events {
                    let id = event.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let pubkey = event.get("pubkey").and_then(|v| v.as_str()).unwrap_or("?");
                    let content = event
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("[no content]");
                    let kind = event.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
                    let created = event
                        .get("created_at")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    output_str.push_str(&format!(
                        "[{}] kind:{} by {}...  @{}\n  {}\n",
                        &id[..id.len().min(8)],
                        kind,
                        &pubkey[..pubkey.len().min(12)],
                        created,
                        content
                    ));
                }
                Ok(output_str)
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(Error::Tool(format!("Relay query failed: {}", stderr)))
            }
        } else {
            // No websocat/nostril -- return the REQ JSON for manual use
            Ok(format!(
                "REQ message constructed (install websocat to query relays directly):\n{}",
                serde_json::to_string_pretty(&req_msg).unwrap_or_default()
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_publish_note_schema() {
        let tool = NostrPublishNoteTool;
        assert_eq!(tool.name(), "nostr_publish_note");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"content"));
    }

    #[test]
    fn test_get_events_schema() {
        let tool = NostrGetEventsTool;
        assert_eq!(tool.name(), "nostr_get_events");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        assert!(params.contains_key("properties"));
    }

    #[test]
    fn test_get_relay_from_args() {
        let args = json!({"relay_url": "wss://custom.relay.io"});
        let relay = get_relay(&args);
        assert_eq!(relay, "wss://custom.relay.io");
    }

    #[test]
    fn test_get_relay_default() {
        let args = json!({});
        let relay = get_relay(&args);
        // Will be env var or default
        assert!(!relay.is_empty());
    }

    #[test]
    fn test_validate_relay_url_valid() {
        assert!(validate_relay_url("wss://relay.damus.io").is_ok());
        assert!(validate_relay_url("ws://localhost:7777").is_ok());
        assert!(validate_relay_url("wss://relay.nostr.info/ws").is_ok());
    }

    #[test]
    fn test_validate_relay_url_bad_scheme() {
        assert!(validate_relay_url("http://example.com").is_err());
        assert!(validate_relay_url("ftp://relay.com").is_err());
        assert!(validate_relay_url("not-a-url").is_err());
    }

    #[test]
    fn test_validate_relay_url_shell_injection() {
        assert!(validate_relay_url("wss://relay.com'; rm -rf / #").is_ok()); // single quotes handled by escaping
        assert!(validate_relay_url("wss://relay.com`id`").is_err()); // backtick injection
        assert!(validate_relay_url("wss://relay.com$(whoami)").is_err()); // command substitution
        assert!(validate_relay_url("wss://relay.com\nid").is_err()); // newline injection
    }

    #[test]
    fn test_get_private_key_from_args() {
        let args = json!({"private_key": "deadbeef0123456789abcdef"});
        let key = get_private_key(&args).expect("should succeed");
        assert_eq!(key, "deadbeef0123456789abcdef");
    }
}
