//! Message Channels
//!
//! Simple channel implementations for sending messages:
//! - file: Write to a local notifications file
//! - webhook: POST to a URL
//! - console: Print to stdout (for testing)

use serde_json::Value;
use std::path::PathBuf;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tracing::debug;
use zeus_core::{Error, Result};

/// Channel type determined from channel name
#[derive(Debug, Clone)]
pub enum Channel {
    /// Write to ~/.zeus/notifications.md
    File(PathBuf),
    /// POST to a webhook URL
    Webhook(String),
    /// Print to console
    Console,
    /// Unknown channel type
    Unknown(String),
}

impl Channel {
    /// Parse channel specification
    /// Examples:
    /// - "file" -> writes to default notifications file
    /// - "file:/path/to/file.md" -> writes to specific file
    /// - "webhook:https://example.com/hook" -> POST to URL
    /// - "console" -> print to stdout
    pub fn parse(spec: &str) -> Self {
        if spec == "file" {
            let path = dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".zeus")
                .join("notifications.md");
            Channel::File(path)
        } else if let Some(path) = spec.strip_prefix("file:") {
            Channel::File(PathBuf::from(path))
        } else if let Some(url) = spec.strip_prefix("webhook:") {
            Channel::Webhook(url.to_string())
        } else if spec == "console" {
            Channel::Console
        } else {
            Channel::Unknown(spec.to_string())
        }
    }

    /// Send a message through this channel
    pub async fn send(&self, content: &str, target: Option<&str>) -> Result<String> {
        match self {
            Channel::File(path) => send_to_file(path, content, target).await,
            Channel::Webhook(url) => send_to_webhook(url, content, target).await,
            Channel::Console => {
                println!("[Zeus] {}", content);
                Ok("Message printed to console".to_string())
            }
            Channel::Unknown(name) => Err(Error::Tool(format!(
                "Unknown channel '{}'. Use 'file', 'file:/path', 'webhook:URL', or 'console'",
                name
            ))),
        }
    }
}

/// Send message to a file (append)
async fn send_to_file(path: &PathBuf, content: &str, target: Option<&str>) -> Result<String> {
    debug!("Sending to file: {:?}", path);

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| Error::Tool(format!("Failed to create directory: {}", e)))?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|e| Error::Tool(format!("Failed to open file: {}", e)))?;

    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    let header = if let Some(t) = target {
        format!("\n## {} (to: {})\n", timestamp, t)
    } else {
        format!("\n## {}\n", timestamp)
    };

    file.write_all(header.as_bytes())
        .await
        .map_err(|e| Error::Tool(format!("Failed to write header: {}", e)))?;
    file.write_all(content.as_bytes())
        .await
        .map_err(|e| Error::Tool(format!("Failed to write content: {}", e)))?;
    file.write_all(b"\n")
        .await
        .map_err(|e| Error::Tool(format!("Failed to write newline: {}", e)))?;

    Ok(format!("Message written to {}", path.display()))
}

/// Send message to a webhook (POST)
async fn send_to_webhook(url: &str, content: &str, target: Option<&str>) -> Result<String> {
    debug!("Sending to webhook: {}", url);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| Error::Tool(format!("Failed to create HTTP client: {}", e)))?;

    let mut payload = serde_json::json!({
        "content": content,
        "source": "zeus"
    });

    if let Some(t) = target {
        payload["target"] = Value::String(t.to_string());
    }

    let response = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| Error::Tool(format!("Webhook request failed: {}", e)))?;

    if response.status().is_success() {
        Ok(format!("Message sent to webhook: {}", url))
    } else {
        Err(Error::Tool(format!(
            "Webhook returned error: {} - {}",
            response.status(),
            response.text().await.unwrap_or_default()
        )))
    }
}

// ============================================================================
// Hermes ↔ ChannelManager bridge
// ============================================================================

use std::sync::Arc;
use zeus_channels::{ChannelManager, ChannelSource};

/// Bridges a `ChannelManager` adapter to Hermes' `NotificationSender` trait.
///
/// Each instance handles one channel type (e.g. "telegram") and sends
/// notifications through the corresponding `ChannelAdapter` registered
/// in the `ChannelManager`.
pub struct ChannelNotificationSender {
    /// Channel type this sender handles (e.g. "telegram", "discord")
    channel_type: String,
    /// Shared channel manager containing the real adapters
    manager: Arc<ChannelManager>,
}

impl ChannelNotificationSender {
    pub fn new(channel_type: impl Into<String>, manager: Arc<ChannelManager>) -> Self {
        Self {
            channel_type: channel_type.into(),
            manager,
        }
    }
}

#[async_trait::async_trait]
impl zeus_hermes::NotificationSender for ChannelNotificationSender {
    fn channel_type(&self) -> &str {
        &self.channel_type
    }

    async fn send(&self, message: &str, target: Option<&str>) -> Result<Option<String>> {
        // Build a ChannelSource. The target is the chat/channel ID.
        // If no target is given, use "notifications" as a placeholder user_id.
        let source = match target {
            Some(chat_id) => ChannelSource::with_chat(&self.channel_type, "zeus-hermes", chat_id),
            None => ChannelSource::new(&self.channel_type, "zeus-hermes"),
        };

        self.manager.send(&source, message).await?;
        Ok(None)
    }
}

/// Create `ChannelNotificationSender` instances for all connected channel adapters
/// in the given `ChannelManager`, and register them with Hermes.
pub fn register_channel_senders(hermes: &mut zeus_hermes::Hermes, manager: &Arc<ChannelManager>) {
    for channel_type in manager.connected_channels() {
        let sender = ChannelNotificationSender::new(channel_type, manager.clone());
        hermes.register_sender(Arc::new(sender));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_parse() {
        assert!(matches!(Channel::parse("file"), Channel::File(_)));
        assert!(matches!(
            Channel::parse("file:/tmp/test.md"),
            Channel::File(_)
        ));
        assert!(matches!(
            Channel::parse("webhook:https://example.com"),
            Channel::Webhook(_)
        ));
        assert!(matches!(Channel::parse("console"), Channel::Console));
        assert!(matches!(Channel::parse("unknown"), Channel::Unknown(_)));
    }

    #[tokio::test]
    async fn test_send_to_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.md");

        send_to_file(&path, "Test message", Some("user"))
            .await
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Test message"));
        assert!(content.contains("to: user"));
    }

    // ================================================================
    // Additional channel tests
    // ================================================================

    #[test]
    fn test_channel_parse_file_default_path() {
        if let Channel::File(path) = Channel::parse("file") {
            assert!(path.to_string_lossy().contains(".zeus"));
            assert!(path.to_string_lossy().contains("notifications.md"));
        } else {
            panic!("Expected Channel::File");
        }
    }

    #[test]
    fn test_channel_parse_file_custom_path() {
        if let Channel::File(path) = Channel::parse("file:/custom/path/notes.md") {
            assert_eq!(path, PathBuf::from("/custom/path/notes.md"));
        } else {
            panic!("Expected Channel::File");
        }
    }

    #[test]
    fn test_channel_parse_webhook_url() {
        if let Channel::Webhook(url) = Channel::parse("webhook:https://hooks.example.com/abc") {
            assert_eq!(url, "https://hooks.example.com/abc");
        } else {
            panic!("Expected Channel::Webhook");
        }
    }

    #[test]
    fn test_channel_parse_unknown_preserved() {
        if let Channel::Unknown(name) = Channel::parse("telegram") {
            assert_eq!(name, "telegram");
        } else {
            panic!("Expected Channel::Unknown");
        }
    }

    #[tokio::test]
    async fn test_send_to_file_without_target() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("no_target.md");

        send_to_file(&path, "Message without target", None)
            .await
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Message without target"));
        assert!(!content.contains("to:"));
    }

    #[tokio::test]
    async fn test_send_to_file_appends() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("append.md");

        send_to_file(&path, "First message", None).await.unwrap();
        send_to_file(&path, "Second message", None).await.unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("First message"));
        assert!(content.contains("Second message"));
    }

    #[tokio::test]
    async fn test_send_to_file_creates_parent_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("dir").join("msgs.md");

        send_to_file(&path, "Nested message", None).await.unwrap();

        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Nested message"));
    }

    #[tokio::test]
    async fn test_console_channel_send() {
        let channel = Channel::Console;
        let result = channel.send("Hello console", None).await.unwrap();
        assert!(result.contains("console"));
    }

    #[tokio::test]
    async fn test_unknown_channel_send_error() {
        let channel = Channel::Unknown("bad_channel".to_string());
        let result = channel.send("test", None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("bad_channel"));
    }

    #[test]
    fn test_channel_debug() {
        let channel = Channel::Console;
        let debug_str = format!("{:?}", channel);
        assert!(debug_str.contains("Console"));

        let file_channel = Channel::File(PathBuf::from("/tmp/test.md"));
        let debug_str = format!("{:?}", file_channel);
        assert!(debug_str.contains("File"));

        let webhook = Channel::Webhook("https://example.com".to_string());
        let debug_str = format!("{:?}", webhook);
        assert!(debug_str.contains("Webhook"));
    }

    // ================================================================
    // New tests
    // ================================================================

    #[test]
    fn test_channel_parse_console() {
        let channel = Channel::parse("console");
        assert!(matches!(channel, Channel::Console));
        // Verify it doesn't match as Unknown
        assert!(!matches!(channel, Channel::Unknown(_)));
    }

    #[test]
    fn test_channel_parse_empty_string() {
        let channel = Channel::parse("");
        // Empty string doesn't match "file", "console", or any prefix
        assert!(matches!(channel, Channel::Unknown(ref s) if s.is_empty()));
    }

    #[test]
    fn test_channel_parse_file_with_spaces() {
        let channel = Channel::parse("file:/path/with spaces/notes.md");
        if let Channel::File(path) = channel {
            assert_eq!(path, PathBuf::from("/path/with spaces/notes.md"));
        } else {
            panic!("Expected Channel::File");
        }
    }

    #[test]
    fn test_channel_parse_webhook_with_path() {
        let channel = Channel::parse("webhook:https://hooks.example.com/api/v1/notify?key=abc");
        if let Channel::Webhook(url) = channel {
            assert_eq!(url, "https://hooks.example.com/api/v1/notify?key=abc");
            assert!(url.contains("/api/v1/notify"));
            assert!(url.contains("key=abc"));
        } else {
            panic!("Expected Channel::Webhook");
        }
    }

    #[test]
    fn test_channel_debug_all_types() {
        let variants: Vec<Channel> = vec![
            Channel::Console,
            Channel::File(PathBuf::from("/tmp/debug.md")),
            Channel::Webhook("https://example.com/hook".to_string()),
            Channel::Unknown("mystery".to_string()),
        ];

        let expected_strs = vec!["Console", "File", "Webhook", "Unknown"];

        for (variant, expected) in variants.iter().zip(expected_strs.iter()) {
            let debug_str = format!("{:?}", variant);
            assert!(
                debug_str.contains(expected),
                "Debug of {:?} should contain '{}'",
                variant,
                expected
            );
        }
    }

    #[tokio::test]
    async fn test_send_to_file_unicode() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("unicode.md");

        let unicode_content =
            "\u{1F680} Rocket launch! \u{2764}\u{FE0F} \u{4F60}\u{597D}\u{4E16}\u{754C}";
        send_to_file(&path, unicode_content, None).await.unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("\u{1F680}"));
        assert!(content.contains("\u{4F60}\u{597D}"));
        assert!(content.contains("Rocket launch!"));
    }
}
