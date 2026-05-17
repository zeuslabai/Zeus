//! iMessage channel adapter
//!
//! Provides iMessage communication via AppleScript on macOS.

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};

#[cfg(target_os = "macos")]
/// Sanitize a string for safe interpolation into AppleScript double-quoted strings.
///
/// Escapes backslashes, double quotes, and control characters that could
/// break out of an AppleScript string context.
fn sanitize_applescript(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}
/// Validate that a recipient looks like a legitimate phone number or Apple ID.
///
/// Accepts:
/// - Phone numbers: optional `+`, then digits (with optional spaces, dashes, parens)
/// - Apple IDs: email-like format (contains `@` with text on both sides)
///
/// Rejects everything else to prevent sending messages to arbitrary handles.
///
/// Only called from `#[cfg(target_os = "macos")]` methods — iMessage is macOS-only.
#[cfg(target_os = "macos")]
fn validate_recipient(recipient: &str) -> Result<()> {
    let trimmed = recipient.trim();

    if trimmed.is_empty() {
        return Err(Error::Channel("Recipient cannot be empty".into()));
    }

    // Check if it looks like a phone number: starts with + or digit, rest is digits/spaces/dashes/parens
    let is_phone = {
        let stripped: String = trimmed
            .chars()
            .filter(|c| !matches!(c, ' ' | '-' | '(' | ')' | '.'))
            .collect();
        if let Some(rest) = stripped.strip_prefix('+') {
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
        } else {
            !stripped.is_empty() && stripped.chars().all(|c| c.is_ascii_digit())
        }
    };

    // Check if it looks like an email/Apple ID
    let is_email = {
        let parts: Vec<&str> = trimmed.split('@').collect();
        parts.len() == 2 && !parts[0].is_empty() && parts[1].contains('.')
    };

    if !is_phone && !is_email {
        return Err(Error::Channel(format!(
            "Invalid recipient format: expected phone number or Apple ID email, got '{}'",
            &trimmed[..zeus_core::floor_char_boundary(trimmed, 30)]
        )));
    }

    Ok(())
}

use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "macos")]
use std::time::Duration;
#[cfg(target_os = "macos")]
use crate::policy::ChannelPolicy;
use tokio::sync::{Mutex, Notify, mpsc};
use zeus_core::{Error, Result};

/// iMessage channel adapter
pub struct IMessageAdapter {
    connected: Arc<AtomicBool>,
    config: IMessageConfig,
    shutdown: Arc<Notify>,
    /// Track seen message IDs to avoid duplicates during polling (macOS only)
    #[allow(dead_code)]
    seen_ids: Arc<Mutex<HashSet<String>>>,
}

impl IMessageAdapter {
    /// Create a new iMessage adapter
    pub async fn new(config: IMessageConfig) -> Result<Self> {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = &config;
            Err(Error::Config("iMessage is only available on macOS".into()))
        }

        #[cfg(target_os = "macos")]
        {
            tracing::info!("iMessage adapter created");
            Ok(Self {
                connected: Arc::new(AtomicBool::new(false)),
                config,
                shutdown: Arc::new(Notify::new()),
                seen_ids: Arc::new(Mutex::new(HashSet::new())),
            })
        }
    }

    /// Send an iMessage via AppleScript
    #[cfg(target_os = "macos")]
    pub async fn send_imessage(&self, recipient: &str, message: &str) -> Result<()> {
        validate_recipient(recipient)?;

        // Reject null bytes that could truncate strings in C-based tools
        if message.contains('\0') || recipient.contains('\0') {
            return Err(Error::Channel(
                "Message or recipient contains null bytes".into(),
            ));
        }

        // Cap message length to prevent abuse
        if message.len() > 10_000 {
            return Err(Error::Channel(
                "Message too long (max 10,000 characters)".into(),
            ));
        }

        let script = format!(
            r#"
tell application "Messages"
    set targetService to 1st account whose service type = iMessage
    set targetBuddy to participant "{}" of targetService
    send "{}" to targetBuddy
end tell
return "Message sent"
"#,
            sanitize_applescript(recipient),
            sanitize_applescript(message)
        );

        let output = tokio::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .await
            .map_err(|e| Error::Channel(format!("Failed to run osascript: {}", e)))?;

        if output.status.success() {
            tracing::info!(recipient = %recipient, "iMessage sent");
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(Error::Channel(format!(
                "Failed to send iMessage: {}",
                stderr
            )))
        }
    }

    /// Read recent messages from a contact
    #[cfg(target_os = "macos")]
    pub async fn read_messages(&self, contact: &str, limit: usize) -> Result<Vec<IMessageMessage>> {
        validate_recipient(contact)?;

        if contact.contains('\0') {
            return Err(Error::Channel("Contact contains null bytes".into()));
        }

        // Cap limit to prevent excessive AppleScript runtime
        let limit = limit.min(100);

        let script = format!(
            r#"
tell application "Messages"
    set output to ""
    set theChats to (every chat whose participants contains participant "{}")
    repeat with aChat in theChats
        set msgCount to 0
        repeat with aMessage in (messages of aChat)
            if msgCount >= {} then exit repeat
            set msgText to text of aMessage
            set msgDate to date of aMessage
            try
                set msgSender to sender of aMessage
                set isFromMe to (msgSender is missing value)
            on error
                set msgSender to ""
                set isFromMe to true
            end try
            set output to output & msgText & "|" & isFromMe & "|" & (msgDate as string) & linefeed
            set msgCount to msgCount + 1
        end repeat
    end repeat
    return output
end tell
"#,
            sanitize_applescript(contact),
            limit
        );

        let output = tokio::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .await
            .map_err(|e| Error::Channel(format!("Failed to run osascript: {}", e)))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let messages: Vec<IMessageMessage> = stdout
                .lines()
                .filter(|l| !l.is_empty())
                .filter_map(|line| {
                    let parts: Vec<&str> = line.split('|').collect();
                    if parts.len() >= 3 {
                        Some(IMessageMessage {
                            text: parts[0].to_string(),
                            is_from_me: parts[1] == "true",
                            timestamp: parts[2].to_string(),
                        })
                    } else {
                        None
                    }
                })
                .collect();

            Ok(messages)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(Error::Channel(format!(
                "Failed to read messages: {}",
                stderr
            )))
        }
    }

    /// Poll for new incoming iMessages via AppleScript.
    /// Returns messages not previously seen (tracked by content+timestamp hash).
    #[cfg(target_os = "macos")]
    async fn poll_new_messages(
        seen_ids: &Arc<Mutex<HashSet<String>>>,
    ) -> Result<Vec<(String, String, String)>> {
        let script = r#"
tell application "Messages"
    set output to ""
    set chatCount to 0
    repeat with aChat in chats
        if chatCount >= 10 then exit repeat
        set msgCount to 0
        repeat with aMessage in (messages of aChat)
            if msgCount >= 5 then exit repeat
            set msgText to text of aMessage
            set msgDate to date of aMessage
            try
                set msgSender to handle of sender of aMessage
                set isFromMe to (msgSender is missing value)
            on error
                set msgSender to "me"
                set isFromMe to true
            end try
            if not isFromMe then
                set output to output & msgSender & "|" & msgText & "|" & (msgDate as string) & linefeed
            end if
            set msgCount to msgCount + 1
        end repeat
        set chatCount to chatCount + 1
    end repeat
    return output
end tell
"#;

        let output = tokio::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .await
            .map_err(|e| Error::Channel(format!("Failed to poll iMessages: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Channel(format!("iMessage poll failed: {}", stderr)));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut new_messages = Vec::new();
        let mut seen = seen_ids.lock().await;

        for line in stdout.lines().filter(|l| !l.is_empty()) {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 3 {
                let sender = parts[0].to_string();
                let text = parts[1].to_string();
                let timestamp = parts[2].to_string();
                let dedup_key = format!("{}|{}|{}", sender, text, timestamp);
                if seen.insert(dedup_key) {
                    new_messages.push((sender, text, timestamp));
                }
            }
        }

        // Cap seen set to prevent unbounded growth
        if seen.len() > 5000 {
            seen.clear();
        }

        Ok(new_messages)
    }

    /// List recent conversations
    #[cfg(target_os = "macos")]
    pub async fn list_conversations(&self, limit: usize) -> Result<Vec<IMessageConversation>> {
        // Cap limit to prevent excessive AppleScript runtime
        let limit = limit.min(100);

        let script = format!(
            r#"
tell application "Messages"
    set output to ""
    set chatCount to 0
    repeat with aChat in chats
        if chatCount >= {} then exit repeat
        set participantList to ""
        repeat with p in participants of aChat
            set participantList to participantList & (handle of p) & ","
        end repeat
        set lastMsg to ""
        try
            set lastMsg to text of last message of aChat
        end try
        set output to output & participantList & "|" & lastMsg & linefeed
        set chatCount to chatCount + 1
    end repeat
    return output
end tell
"#,
            limit
        );

        let output = tokio::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .await
            .map_err(|e| Error::Channel(format!("Failed to run osascript: {}", e)))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let conversations: Vec<IMessageConversation> = stdout
                .lines()
                .filter(|l| !l.is_empty())
                .filter_map(|line| {
                    let parts: Vec<&str> = line.split('|').collect();
                    if parts.len() >= 2 {
                        let participants: Vec<String> = parts[0]
                            .split(',')
                            .filter(|p| !p.is_empty())
                            .map(|s| s.to_string())
                            .collect();
                        Some(IMessageConversation {
                            participants,
                            last_message: parts[1].to_string(),
                        })
                    } else {
                        None
                    }
                })
                .collect();

            Ok(conversations)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(Error::Channel(format!(
                "Failed to list conversations: {}",
                stderr
            )))
        }
    }
}

#[async_trait]
impl ChannelAdapter for IMessageAdapter {
    fn channel_type(&self) -> &'static str {
        "imessage"
    }

    fn receive_mode(&self) -> ReceiveMode {
        if self.config.poll_for_messages {
            ReceiveMode::Polling {
                interval_secs: self.config.poll_interval_secs,
            }
        } else {
            ReceiveMode::None
        }
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        self.connected.store(true, Ordering::SeqCst);

        #[cfg(target_os = "macos")]
        {
            // If polling is enabled, start background polling task
            if self.config.poll_for_messages {
                let shutdown = self.shutdown.clone();
                let connected = self.connected.clone();
                let poll_interval = Duration::from_secs(self.config.poll_interval_secs);
                let seen_ids = self.seen_ids.clone();
                let policy = ChannelPolicy::new(self.config.policy.clone().unwrap_or_default());

                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = tokio::time::sleep(poll_interval) => {
                                if !connected.load(Ordering::SeqCst) {
                                    break;
                                }
                                match Self::poll_new_messages(&seen_ids).await {
                                    Ok(messages) => {
                                        for (sender, text, _timestamp) in messages {
                                            // Layer 1: policy — all iMessages are DMs
                                            if policy.check_dm(&sender).is_denied() {
                                                tracing::debug!(from = %sender, "iMessage denied by policy");
                                                continue;
                                            }
                                            let source = ChannelSource::new("imessage", &sender);
                                            // All iMessages are DMs — from_address IS the addressing signal
                                            let is_addressed = true;
                                            let msg = ChannelMessage::new(source, text).with_addressed(is_addressed);
                                            if tx.send(msg).await.is_err() {
                                                tracing::warn!("iMessage channel receiver dropped");
                                                return;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!("iMessage poll error: {}", e);
                                    }
                                }
                            }
                            _ = shutdown.notified() => {
                                tracing::info!("iMessage polling shutdown");
                                break;
                            }
                        }
                    }
                });
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = tx;
        }

        tracing::info!("iMessage adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.shutdown.notify_one();
        tracing::info!("iMessage adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "imessage" {
            return Err(Error::channel("Invalid channel source for iMessage"));
        }

        #[cfg(target_os = "macos")]
        {
            // user_id is the phone number or Apple ID handle
            self.send_imessage(&to.user_id, content).await
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (to, content);
            Err(Error::channel("iMessage is only available on macOS"))
        }
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn supports_native_identity(&self) -> bool {
        false
    }
}

/// iMessage configuration
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct IMessageConfig {
    /// Whether to poll for incoming messages
    #[serde(default)]
    pub poll_for_messages: bool,
    /// Poll interval in seconds
    #[serde(default = "default_imessage_poll_interval")]
    pub poll_interval_secs: u64,
    /// Access policy (DM access control)
    #[serde(default)]
    pub policy: Option<zeus_core::ChannelPolicyConfig>,
}

fn default_imessage_poll_interval() -> u64 {
    30
}

impl Default for IMessageConfig {
    fn default() -> Self {
        Self {
            poll_for_messages: false,
            poll_interval_secs: default_imessage_poll_interval(),
            policy: None,
        }
    }
}

/// A message from iMessage
#[derive(Debug, Clone)]
pub struct IMessageMessage {
    /// Message text
    pub text: String,
    /// Whether the message was sent by the user
    pub is_from_me: bool,
    /// Timestamp string
    pub timestamp: String,
}

/// An iMessage conversation
#[derive(Debug, Clone)]
pub struct IMessageConversation {
    /// Participant handles
    pub participants: Vec<String>,
    /// Last message preview
    pub last_message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentSendIdentity;

    #[cfg(target_os = "macos")]
    #[test]
    fn test_validate_recipient_phone() {
        assert!(validate_recipient("+14155551234").is_ok());
        assert!(validate_recipient("+44 20 7946 0958").is_ok());
        assert!(validate_recipient("(415) 555-1234").is_ok());
        assert!(validate_recipient("5551234567").is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_validate_recipient_email() {
        assert!(validate_recipient("user@icloud.com").is_ok());
        assert!(validate_recipient("test@example.co.uk").is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_validate_recipient_rejects_invalid() {
        assert!(validate_recipient("").is_err());
        assert!(validate_recipient("   ").is_err());
        assert!(validate_recipient("not-a-number-or-email").is_err());
        assert!(validate_recipient("../../../etc/passwd").is_err());
        assert!(validate_recipient("user@").is_err());
    }

    #[test]
    fn test_imessage_config_defaults() {
        let config = IMessageConfig::default();
        assert!(!config.poll_for_messages);
        assert_eq!(config.poll_interval_secs, 30);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn test_imessage_adapter_lifecycle() {
        let config = IMessageConfig::default();
        let adapter = IMessageAdapter::new(config)
            .await
            .expect("IMessageAdapter::new should succeed");
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "imessage");
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn test_imessage_adapter_start_stop() {
        let config = IMessageConfig::default();
        let adapter = IMessageAdapter::new(config)
            .await
            .expect("IMessageAdapter::new should succeed");
        let (tx, _rx) = mpsc::channel(100);

        adapter
            .start(tx)
            .await
            .expect("async operation should succeed");
        assert!(adapter.is_connected());

        adapter
            .stop()
            .await
            .expect("async operation should succeed");
        assert!(!adapter.is_connected());
    }

    // ── S33 Track D: Tier 2 identity tests ──────────────────────────────────

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn test_imessage_supports_native_identity_false() {
        let config = IMessageConfig::default();
        let adapter = IMessageAdapter::new(config)
            .await
            .expect("IMessageAdapter::new should succeed");
        assert!(!adapter.supports_native_identity());
    }

    #[test]
    fn test_imessage_send_as_text_prefix_format() {
        let identity = AgentSendIdentity::new("zeus_agent");
        let prefixed = identity.apply_prefix("Hello from iMessage");
        assert_eq!(prefixed, "[zeus_agent] Hello from iMessage");
    }
}
