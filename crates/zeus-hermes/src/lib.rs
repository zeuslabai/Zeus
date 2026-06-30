//! Zeus Hermes - Notification Router
//!
//! Multi-channel notification delivery with priority routing.
//!
//! Hermes routes notifications to configured channels (Telegram, Discord,
//! Slack, Email, etc.) via pluggable `NotificationSender` backends.
//! Register senders with `Hermes::register_sender()`, then call `notify()`
//! — Hermes resolves the channel name to the right sender and dispatches.

pub mod escalation;
pub use escalation::{
    AlertStatus, EscalationAction, EscalationChain, EscalationConfig, EscalationLevel,
    EscalationManager, EscalationStats,
};
pub mod routing;
pub use routing::{
    AdaptiveRouter, ChannelHealthSnapshot, ChannelScore, NotificationCategory, RouteChannel,
    RoutingConfig, RoutingDecision, RoutingStats, Urgency, UserPreferences,
};
pub mod formatter;
pub use formatter::{
    AlertSeverity, DigestItem, MessageSection, NotificationFormatter, OutputFormat, Template,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};
use zeus_core::Result;

// ============================================================================
// NotificationSender trait
// ============================================================================

/// A pluggable backend that can deliver notifications to a specific channel type.
///
/// Implement this trait to bridge Hermes to real messaging platforms.
/// The `channel_type()` return value is matched against channel names in
/// `Notification::channels` and `HermesConfig::default_channels`.
#[async_trait]
pub trait NotificationSender: Send + Sync {
    /// Channel type this sender handles (e.g. "telegram", "discord", "slack", "email").
    fn channel_type(&self) -> &str;

    /// Send a notification message to the configured target.
    ///
    /// `target` is the destination within the channel (chat ID, channel ID, email
    /// address, etc.). When `None`, the sender should use its own default target.
    async fn send(&self, message: &str, target: Option<&str>) -> Result<Option<String>>;
}

/// Per-channel routing target configuration.
///
/// Maps a channel name to an optional target ID (e.g. Telegram chat_id,
/// Discord channel_id). Stored in `HermesConfig::targets`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationTarget {
    /// Target ID within the channel (chat_id, channel_id, email address, etc.)
    pub target_id: Option<String>,
}

// ============================================================================
// Hermes
// ============================================================================

/// Notification router
pub struct Hermes {
    config: HermesConfig,
    stats: NotificationStats,
    /// Registered notification senders keyed by channel type
    senders: HashMap<String, Arc<dyn NotificationSender>>,
}

/// Hermes configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HermesConfig {
    /// Default channels for each priority level
    pub default_channels: HashMap<NotificationPriority, Vec<String>>,
    /// Whether to batch low priority notifications
    #[serde(default)]
    pub batch_low_priority: bool,
    /// Batch interval in seconds
    #[serde(default = "default_batch_interval")]
    pub batch_interval_secs: u64,
    /// Per-channel target configuration (channel_name → target)
    #[serde(default)]
    pub targets: HashMap<String, NotificationTarget>,
}

fn default_batch_interval() -> u64 {
    300
}

/// A notification to send
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    /// Message content
    pub message: String,
    /// Title (optional)
    pub title: Option<String>,
    /// Priority level
    #[serde(default)]
    pub priority: NotificationPriority,
    /// Target channels (empty = use defaults based on priority)
    #[serde(default)]
    pub channels: Vec<String>,
    /// Additional metadata
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Notification {
    /// Create a new notification
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            title: None,
            priority: NotificationPriority::Normal,
            channels: vec![],
            metadata: HashMap::new(),
        }
    }

    /// Set title
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set priority
    pub fn with_priority(mut self, priority: NotificationPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Add a target channel
    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channels.push(channel.into());
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

/// Notification priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationPriority {
    /// Low priority - can be batched
    Low = 0,
    /// Normal priority (default)
    #[default]
    Normal = 1,
    /// High priority - send immediately
    High = 2,
    /// Urgent - send to all channels
    Urgent = 3,
}

/// Delivery result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryResult {
    /// Whether delivery succeeded
    pub success: bool,
    /// Channel that was used
    pub channel: String,
    /// Message ID if available
    pub message_id: Option<String>,
    /// Error message if failed
    pub error: Option<String>,
}

/// Notification statistics
#[derive(Debug, Clone, Default)]
pub struct NotificationStats {
    /// Total notifications sent
    pub total_sent: u64,
    /// Successful deliveries
    pub successful: u64,
    /// Failed deliveries
    pub failed: u64,
    /// Notifications by priority
    pub by_priority: HashMap<NotificationPriority, u64>,
    /// Notifications by channel
    pub by_channel: HashMap<String, u64>,
}

impl Hermes {
    /// Create a new Hermes instance
    pub fn new(config: HermesConfig) -> Self {
        Self {
            config,
            stats: NotificationStats::default(),
            senders: HashMap::new(),
        }
    }

    /// Register a notification sender for a channel type.
    ///
    /// When `notify()` encounters this channel name, it dispatches to this sender
    /// instead of falling back to the built-in console/file handlers.
    pub fn register_sender(&mut self, sender: Arc<dyn NotificationSender>) {
        let channel = sender.channel_type().to_string();
        info!(channel = %channel, "Hermes: registered notification sender");
        self.senders.insert(channel, sender);
    }

    /// List registered sender channel types
    pub fn registered_channels(&self) -> Vec<&str> {
        self.senders.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for Hermes {
    fn default() -> Self {
        Self::new(HermesConfig::default())
    }
}

impl Hermes {
    /// Send a notification
    pub async fn notify(&mut self, notification: Notification) -> Result<Vec<DeliveryResult>> {
        let channels = if notification.channels.is_empty() {
            // Use default channels based on priority
            self.config
                .default_channels
                .get(&notification.priority)
                .cloned()
                .unwrap_or_else(|| vec!["console".to_string()])
        } else {
            notification.channels.clone()
        };

        let mut results = Vec::new();

        for channel in channels {
            let result = self.send_to_channel(&channel, &notification).await;

            // Update stats
            self.stats.total_sent += 1;
            if result.success {
                self.stats.successful += 1;
            } else {
                self.stats.failed += 1;
            }
            *self
                .stats
                .by_priority
                .entry(notification.priority)
                .or_insert(0) += 1;
            *self.stats.by_channel.entry(channel.clone()).or_insert(0) += 1;

            results.push(result);
        }

        Ok(results)
    }

    /// Send to a specific channel.
    ///
    /// Dispatch order:
    /// 1. Built-in channels ("console", "file") — always available
    /// 2. Registered `NotificationSender` — matched by channel type name
    /// 3. Fallback — returns error for unknown channels
    async fn send_to_channel(&self, channel: &str, notification: &Notification) -> DeliveryResult {
        // Format the message with optional title prefix
        let formatted = match &notification.title {
            Some(title) => format!(
                "[{}] {}: {}",
                notification.priority_str(),
                title,
                notification.message
            ),
            None => format!("[{}] {}", notification.priority_str(), notification.message),
        };

        match channel {
            "console" => {
                println!("{formatted}");
                DeliveryResult {
                    success: true,
                    channel: channel.to_string(),
                    message_id: None,
                    error: None,
                }
            }
            "file" => {
                let log_path = zeus_core::default_config_dir().join("notifications.log");
                let entry = format!("[{}] {}\n", chrono::Utc::now().to_rfc3339(), formatted);

                let result = async {
                    use tokio::io::AsyncWriteExt;
                    let mut f = tokio::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&log_path)
                        .await?;
                    f.write_all(entry.as_bytes()).await
                }
                .await;

                match result {
                    Ok(_) => DeliveryResult {
                        success: true,
                        channel: channel.to_string(),
                        message_id: None,
                        error: None,
                    },
                    Err(e) => DeliveryResult {
                        success: false,
                        channel: channel.to_string(),
                        message_id: None,
                        error: Some(e.to_string()),
                    },
                }
            }
            _ => {
                // Try registered senders
                if let Some(sender) = self.senders.get(channel) {
                    let target = self
                        .config
                        .targets
                        .get(channel)
                        .and_then(|t| t.target_id.as_deref());

                    match sender.send(&formatted, target).await {
                        Ok(msg_id) => {
                            info!(channel = %channel, "Notification delivered via sender");
                            DeliveryResult {
                                success: true,
                                channel: channel.to_string(),
                                message_id: msg_id,
                                error: None,
                            }
                        }
                        Err(e) => {
                            warn!(channel = %channel, error = %e, "Notification sender failed");
                            DeliveryResult {
                                success: false,
                                channel: channel.to_string(),
                                message_id: None,
                                error: Some(e.to_string()),
                            }
                        }
                    }
                } else {
                    warn!(channel = %channel, "No sender registered for channel");
                    DeliveryResult {
                        success: false,
                        channel: channel.to_string(),
                        message_id: None,
                        error: Some(format!("No sender registered for channel: {}", channel)),
                    }
                }
            }
        }
    }

    /// Broadcast to all configured channels
    pub async fn broadcast(&mut self, notification: Notification) -> Result<Vec<DeliveryResult>> {
        let all_channels: Vec<String> = self
            .config
            .default_channels
            .values()
            .flatten()
            .cloned()
            .collect();

        let mut notification = notification;
        notification.channels = all_channels;

        self.notify(notification).await
    }

    /// Get notification stats
    pub fn stats(&self) -> &NotificationStats {
        &self.stats
    }

    /// Reset stats
    pub fn reset_stats(&mut self) {
        self.stats = NotificationStats::default();
    }
}

impl Notification {
    fn priority_str(&self) -> &'static str {
        match self.priority {
            NotificationPriority::Low => "LOW",
            NotificationPriority::Normal => "NORMAL",
            NotificationPriority::High => "HIGH",
            NotificationPriority::Urgent => "URGENT",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Notification builder tests -----------------------------------------

    #[test]
    fn test_notification_new() {
        let n = Notification::new("hello");
        assert_eq!(n.message, "hello");
        assert!(n.title.is_none());
        assert_eq!(n.priority, NotificationPriority::Normal);
        assert!(n.channels.is_empty());
        assert!(n.metadata.is_empty());
    }

    #[test]
    fn test_notification_with_title() {
        let n = Notification::new("msg").with_title("My Title");
        assert_eq!(n.title.as_deref(), Some("My Title"));
    }

    #[test]
    fn test_notification_with_priority() {
        let n = Notification::new("msg").with_priority(NotificationPriority::Urgent);
        assert_eq!(n.priority, NotificationPriority::Urgent);
    }

    #[test]
    fn test_notification_with_channel() {
        let n = Notification::new("msg")
            .with_channel("telegram")
            .with_channel("email");
        assert_eq!(n.channels, vec!["telegram", "email"]);
    }

    #[test]
    fn test_notification_with_metadata() {
        let n = Notification::new("msg")
            .with_metadata("key", serde_json::json!("value"))
            .with_metadata("count", serde_json::json!(42));
        assert_eq!(n.metadata.len(), 2);
        assert_eq!(n.metadata["key"], serde_json::json!("value"));
        assert_eq!(n.metadata["count"], serde_json::json!(42));
    }

    #[test]
    fn test_notification_full_builder() {
        let n = Notification::new("Alert!")
            .with_title("System Alert")
            .with_priority(NotificationPriority::High)
            .with_channel("console")
            .with_channel("file")
            .with_metadata("source", serde_json::json!("monitor"));
        assert_eq!(n.message, "Alert!");
        assert_eq!(n.title.as_deref(), Some("System Alert"));
        assert_eq!(n.priority, NotificationPriority::High);
        assert_eq!(n.channels.len(), 2);
        assert_eq!(n.metadata.len(), 1);
    }

    #[test]
    fn test_notification_serialization() {
        let n = Notification::new("test")
            .with_title("T")
            .with_priority(NotificationPriority::Low);
        let json = serde_json::to_string(&n).expect("should serialize to JSON");
        let de: Notification = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.message, "test");
        assert_eq!(de.title.as_deref(), Some("T"));
        assert_eq!(de.priority, NotificationPriority::Low);
    }

    // -- Priority tests -----------------------------------------------------

    #[test]
    fn test_priority_default() {
        assert_eq!(
            NotificationPriority::default(),
            NotificationPriority::Normal
        );
    }

    #[test]
    fn test_priority_ordering() {
        assert_eq!(NotificationPriority::Low as u8, 0);
        assert_eq!(NotificationPriority::Normal as u8, 1);
        assert_eq!(NotificationPriority::High as u8, 2);
        assert_eq!(NotificationPriority::Urgent as u8, 3);
    }

    #[test]
    fn test_priority_serialization() {
        let json =
            serde_json::to_string(&NotificationPriority::High).expect("should serialize to JSON");
        assert_eq!(json, "\"high\"");
        let de: NotificationPriority =
            serde_json::from_str("\"urgent\"").expect("should parse successfully");
        assert_eq!(de, NotificationPriority::Urgent);
    }

    #[test]
    fn test_priority_str() {
        let n = Notification::new("").with_priority(NotificationPriority::Low);
        assert_eq!(n.priority_str(), "LOW");
        let n = Notification::new("").with_priority(NotificationPriority::Normal);
        assert_eq!(n.priority_str(), "NORMAL");
        let n = Notification::new("").with_priority(NotificationPriority::High);
        assert_eq!(n.priority_str(), "HIGH");
        let n = Notification::new("").with_priority(NotificationPriority::Urgent);
        assert_eq!(n.priority_str(), "URGENT");
    }

    #[test]
    fn test_priority_hash_key() {
        let mut map = HashMap::new();
        map.insert(NotificationPriority::Low, vec!["file".to_string()]);
        map.insert(NotificationPriority::High, vec!["telegram".to_string()]);
        assert_eq!(map.len(), 2);
        assert!(map.contains_key(&NotificationPriority::Low));
    }

    // -- HermesConfig tests -------------------------------------------------

    #[test]
    fn test_config_default() {
        let cfg = HermesConfig::default();
        assert!(cfg.default_channels.is_empty());
        assert!(!cfg.batch_low_priority);
        assert_eq!(cfg.batch_interval_secs, 0);
    }

    #[test]
    fn test_config_serialization() {
        let mut cfg = HermesConfig::default();
        cfg.batch_low_priority = true;
        cfg.batch_interval_secs = 600;
        cfg.default_channels.insert(
            NotificationPriority::High,
            vec!["telegram".to_string(), "console".to_string()],
        );
        let json = serde_json::to_string(&cfg).expect("should serialize to JSON");
        let de: HermesConfig = serde_json::from_str(&json).expect("should parse successfully");
        assert!(de.batch_low_priority);
        assert_eq!(de.batch_interval_secs, 600);
        assert_eq!(de.default_channels[&NotificationPriority::High].len(), 2);
    }

    // -- DeliveryResult tests -----------------------------------------------

    #[test]
    fn test_delivery_result_success() {
        let r = DeliveryResult {
            success: true,
            channel: "console".to_string(),
            message_id: Some("msg-123".to_string()),
            error: None,
        };
        assert!(r.success);
        assert_eq!(r.channel, "console");
        assert!(r.error.is_none());
    }

    #[test]
    fn test_delivery_result_failure() {
        let r = DeliveryResult {
            success: false,
            channel: "telegram".to_string(),
            message_id: None,
            error: Some("API timeout".to_string()),
        };
        assert!(!r.success);
        assert_eq!(r.error.as_deref(), Some("API timeout"));
    }

    #[test]
    fn test_delivery_result_serialization() {
        let r = DeliveryResult {
            success: true,
            channel: "file".to_string(),
            message_id: None,
            error: None,
        };
        let json = serde_json::to_string(&r).expect("should serialize to JSON");
        let de: DeliveryResult = serde_json::from_str(&json).expect("should parse successfully");
        assert!(de.success);
        assert_eq!(de.channel, "file");
    }

    // -- Hermes instance tests ----------------------------------------------

    #[test]
    fn test_hermes_default() {
        let h = Hermes::default();
        assert_eq!(h.stats().total_sent, 0);
        assert_eq!(h.stats().successful, 0);
        assert_eq!(h.stats().failed, 0);
    }

    #[test]
    fn test_hermes_with_config() {
        let mut cfg = HermesConfig::default();
        cfg.batch_low_priority = true;
        let h = Hermes::new(cfg);
        assert_eq!(h.stats().total_sent, 0);
    }

    #[tokio::test]
    async fn test_console_notification() {
        let mut h = Hermes::default();
        let n = Notification::new("Test message").with_channel("console");
        let results = h.notify(n).await.expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert_eq!(results[0].channel, "console");
    }

    #[tokio::test]
    async fn test_unknown_channel_fails() {
        let mut h = Hermes::default();
        let n = Notification::new("msg").with_channel("carrier_pigeon");
        let results = h.notify(n).await.expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(!results[0].success);
        assert!(
            results[0]
                .error
                .as_ref()
                .expect("as_ref should succeed")
                .contains("No sender registered")
        );
    }

    #[tokio::test]
    async fn test_multi_channel_notification() {
        let mut h = Hermes::default();
        let n = Notification::new("msg")
            .with_channel("console")
            .with_channel("unknown");
        let results = h.notify(n).await.expect("async operation should succeed");
        assert_eq!(results.len(), 2);
        assert!(results[0].success); // console succeeds
        assert!(!results[1].success); // unknown fails
    }

    #[tokio::test]
    async fn test_default_channel_fallback() {
        let mut h = Hermes::default();
        // No channels specified and no defaults configured — should use "console"
        let n = Notification::new("fallback test");
        let results = h.notify(n).await.expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert_eq!(results[0].channel, "console");
    }

    #[tokio::test]
    async fn test_configured_default_channels() {
        let mut cfg = HermesConfig::default();
        cfg.default_channels
            .insert(NotificationPriority::High, vec!["console".to_string()]);
        let mut h = Hermes::new(cfg);
        let n = Notification::new("high priority").with_priority(NotificationPriority::High);
        let results = h.notify(n).await.expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
    }

    // -- Stats tests --------------------------------------------------------

    #[tokio::test]
    async fn test_stats_tracking() {
        let mut h = Hermes::default();
        h.notify(Notification::new("a").with_channel("console"))
            .await
            .expect("Notification::new should succeed");
        h.notify(Notification::new("b").with_channel("console"))
            .await
            .expect("Notification::new should succeed");
        h.notify(Notification::new("c").with_channel("bad_channel"))
            .await
            .expect("Notification::new should succeed");

        assert_eq!(h.stats().total_sent, 3);
        assert_eq!(h.stats().successful, 2);
        assert_eq!(h.stats().failed, 1);
    }

    #[tokio::test]
    async fn test_stats_by_priority() {
        let mut h = Hermes::default();
        h.notify(
            Notification::new("a")
                .with_channel("console")
                .with_priority(NotificationPriority::High),
        )
        .await
        .expect("async operation should succeed");
        h.notify(
            Notification::new("b")
                .with_channel("console")
                .with_priority(NotificationPriority::Low),
        )
        .await
        .expect("async operation should succeed");

        assert_eq!(h.stats().by_priority[&NotificationPriority::High], 1);
        assert_eq!(h.stats().by_priority[&NotificationPriority::Low], 1);
    }

    #[tokio::test]
    async fn test_stats_by_channel() {
        let mut h = Hermes::default();
        h.notify(Notification::new("a").with_channel("console"))
            .await
            .expect("Notification::new should succeed");
        h.notify(Notification::new("b").with_channel("console"))
            .await
            .expect("Notification::new should succeed");

        assert_eq!(h.stats().by_channel["console"], 2);
    }

    #[tokio::test]
    async fn test_reset_stats() {
        let mut h = Hermes::default();
        h.notify(Notification::new("a").with_channel("console"))
            .await
            .expect("Notification::new should succeed");
        assert_eq!(h.stats().total_sent, 1);

        h.reset_stats();
        assert_eq!(h.stats().total_sent, 0);
        assert_eq!(h.stats().successful, 0);
        assert_eq!(h.stats().failed, 0);
    }

    // -- NotificationStats tests --------------------------------------------

    #[test]
    fn test_notification_stats_default() {
        let stats = NotificationStats::default();
        assert_eq!(stats.total_sent, 0);
        assert_eq!(stats.successful, 0);
        assert_eq!(stats.failed, 0);
        assert!(stats.by_priority.is_empty());
        assert!(stats.by_channel.is_empty());
    }

    // -- NotificationSender tests -------------------------------------------

    /// Mock sender that records messages and can be configured to succeed or fail.
    struct MockSender {
        channel: String,
        messages: Arc<tokio::sync::Mutex<Vec<(String, Option<String>)>>>,
        should_fail: bool,
    }

    impl MockSender {
        fn new(channel: &str) -> (Self, Arc<tokio::sync::Mutex<Vec<(String, Option<String>)>>>) {
            let messages = Arc::new(tokio::sync::Mutex::new(Vec::new()));
            (
                Self {
                    channel: channel.to_string(),
                    messages: messages.clone(),
                    should_fail: false,
                },
                messages,
            )
        }

        fn failing(channel: &str) -> Self {
            Self {
                channel: channel.to_string(),
                messages: Arc::new(tokio::sync::Mutex::new(Vec::new())),
                should_fail: true,
            }
        }
    }

    #[async_trait]
    impl NotificationSender for MockSender {
        fn channel_type(&self) -> &str {
            &self.channel
        }

        async fn send(
            &self,
            message: &str,
            target: Option<&str>,
        ) -> zeus_core::Result<Option<String>> {
            if self.should_fail {
                return Err(zeus_core::Error::Channel("Mock send failure".into()));
            }
            self.messages
                .lock()
                .await
                .push((message.to_string(), target.map(|s| s.to_string())));
            Ok(Some(format!(
                "mock-msg-{}",
                self.messages.lock().await.len()
            )))
        }
    }

    #[tokio::test]
    async fn test_sender_dispatches_to_registered_channel() {
        let (sender, messages) = MockSender::new("telegram");
        let mut h = Hermes::default();
        h.register_sender(Arc::new(sender));

        let n = Notification::new("Hello Telegram!").with_channel("telegram");
        let results = h.notify(n).await.expect("should succeed");

        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert_eq!(results[0].channel, "telegram");
        assert!(results[0].message_id.is_some());

        let msgs = messages.lock().await;
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].0.contains("Hello Telegram!"));
        assert!(msgs[0].1.is_none()); // no target configured
    }

    #[tokio::test]
    async fn test_sender_receives_target_from_config() {
        let (sender, messages) = MockSender::new("telegram");
        let mut cfg = HermesConfig::default();
        cfg.targets.insert(
            "telegram".to_string(),
            NotificationTarget {
                target_id: Some("-100123456".to_string()),
            },
        );

        let mut h = Hermes::new(cfg);
        h.register_sender(Arc::new(sender));

        let n = Notification::new("targeted msg").with_channel("telegram");
        let results = h.notify(n).await.expect("should succeed");
        assert!(results[0].success);

        let msgs = messages.lock().await;
        assert_eq!(msgs[0].1.as_deref(), Some("-100123456"));
    }

    #[tokio::test]
    async fn test_sender_failure_tracked_in_stats() {
        let sender = MockSender::failing("discord");
        let mut h = Hermes::default();
        h.register_sender(Arc::new(sender));

        let n = Notification::new("will fail").with_channel("discord");
        let results = h.notify(n).await.expect("should succeed");

        assert_eq!(results.len(), 1);
        assert!(!results[0].success);
        assert!(
            results[0]
                .error
                .as_ref()
                .unwrap()
                .contains("Mock send failure")
        );
        assert_eq!(h.stats().failed, 1);
        assert_eq!(h.stats().successful, 0);
    }

    #[tokio::test]
    async fn test_multi_sender_mixed_delivery() {
        let (tg_sender, tg_msgs) = MockSender::new("telegram");
        let (dc_sender, dc_msgs) = MockSender::new("discord");

        let mut h = Hermes::default();
        h.register_sender(Arc::new(tg_sender));
        h.register_sender(Arc::new(dc_sender));

        let n = Notification::new("broadcast")
            .with_channel("telegram")
            .with_channel("discord")
            .with_channel("console");
        let results = h.notify(n).await.expect("should succeed");

        assert_eq!(results.len(), 3);
        assert!(results[0].success); // telegram
        assert!(results[1].success); // discord
        assert!(results[2].success); // console (built-in)

        assert_eq!(tg_msgs.lock().await.len(), 1);
        assert_eq!(dc_msgs.lock().await.len(), 1);
        assert_eq!(h.stats().successful, 3);
    }

    #[tokio::test]
    async fn test_sender_with_default_channels_by_priority() {
        let (sender, messages) = MockSender::new("telegram");
        let mut cfg = HermesConfig::default();
        cfg.default_channels
            .insert(NotificationPriority::High, vec!["telegram".to_string()]);
        cfg.default_channels
            .insert(NotificationPriority::Low, vec!["console".to_string()]);

        let mut h = Hermes::new(cfg);
        h.register_sender(Arc::new(sender));

        // High priority → telegram
        let n = Notification::new("urgent").with_priority(NotificationPriority::High);
        let results = h.notify(n).await.expect("should succeed");
        assert_eq!(results[0].channel, "telegram");
        assert!(results[0].success);
        assert_eq!(messages.lock().await.len(), 1);

        // Low priority → console
        let n = Notification::new("minor").with_priority(NotificationPriority::Low);
        let results = h.notify(n).await.expect("should succeed");
        assert_eq!(results[0].channel, "console");
    }

    #[tokio::test]
    async fn test_registered_channels_list() {
        let (tg, _) = MockSender::new("telegram");
        let (dc, _) = MockSender::new("discord");

        let mut h = Hermes::default();
        assert!(h.registered_channels().is_empty());

        h.register_sender(Arc::new(tg));
        h.register_sender(Arc::new(dc));

        let channels = h.registered_channels();
        assert_eq!(channels.len(), 2);
        assert!(channels.contains(&"telegram"));
        assert!(channels.contains(&"discord"));
    }

    #[tokio::test]
    async fn test_sender_replaces_existing() {
        let (sender1, msgs1) = MockSender::new("telegram");
        let (sender2, msgs2) = MockSender::new("telegram");

        let mut h = Hermes::default();
        h.register_sender(Arc::new(sender1));
        h.register_sender(Arc::new(sender2)); // replaces first

        let n = Notification::new("test").with_channel("telegram");
        let _ = h.notify(n).await;

        assert_eq!(msgs1.lock().await.len(), 0); // first sender not called
        assert_eq!(msgs2.lock().await.len(), 1); // second sender called
    }

    #[tokio::test]
    async fn test_notification_message_formatting() {
        let (sender, messages) = MockSender::new("telegram");
        let mut h = Hermes::default();
        h.register_sender(Arc::new(sender));

        // With title
        let n = Notification::new("body text")
            .with_title("Alert")
            .with_priority(NotificationPriority::High)
            .with_channel("telegram");
        let _ = h.notify(n).await;

        let msgs = messages.lock().await;
        assert!(msgs[0].0.contains("[HIGH]"));
        assert!(msgs[0].0.contains("Alert"));
        assert!(msgs[0].0.contains("body text"));
    }

    #[tokio::test]
    async fn test_broadcast_with_senders() {
        let (sender, messages) = MockSender::new("telegram");
        let mut cfg = HermesConfig::default();
        cfg.default_channels.insert(
            NotificationPriority::Normal,
            vec!["console".to_string(), "telegram".to_string()],
        );

        let mut h = Hermes::new(cfg);
        h.register_sender(Arc::new(sender));

        let n = Notification::new("broadcast message");
        let results = h.broadcast(n).await.expect("should succeed");

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.success));
        assert_eq!(messages.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn test_file_channel_returns_delivery_result() {
        // The "file" channel writes to ~/.zeus/notifications.log.
        // Ensure it returns a properly-typed DeliveryResult (success or failure
        // depending on whether the directory exists) — the key thing is it uses
        // async I/O and does not panic.
        let mut h = Hermes::default();
        let n = Notification::new("file channel test").with_channel("file");
        let results = h.notify(n).await.expect("notify should not return Err");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].channel, "file");
        // success may be true or false depending on whether ~/.zeus exists;
        // either way stats must be updated consistently
        if results[0].success {
            assert_eq!(h.stats().successful, 1);
            assert_eq!(h.stats().failed, 0);
        } else {
            assert_eq!(h.stats().failed, 1);
            assert_eq!(h.stats().successful, 0);
            assert!(results[0].error.is_some());
        }
    }
}
