//! Per-author-per-channel message debouncer (OpenClaw Layer 3 + Layer 7).
//!
//! Accumulates rapid messages from the same author in the same channel within
//! a configurable window (default 1.5s). When the window expires, all buffered
//! messages are flushed as a single combined `ChannelMessage`.
//!
//! Key format: `{channel_type}:{account_id}:{channel_id}:{author_id}`

use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};
use tracing::{debug, info};

use crate::ChannelMessage;

/// A batch of debounced messages from the same author+channel.
/// Preserves all individual messages and their IDs for proper reply threading.
#[derive(Debug, Clone)]
pub struct MessageBatch {
    /// All messages in the burst, in arrival order.
    pub messages: Vec<ChannelMessage>,
    /// Platform message IDs for threading replies (extracted from each message).
    pub reply_to_ids: Vec<String>,
}

impl MessageBatch {
    /// Build a batch from a vec of messages.
    fn from_messages(messages: Vec<ChannelMessage>) -> Self {
        let reply_to_ids: Vec<String> = messages
            .iter()
            .filter_map(|m| m.platform_message_id.clone())
            .collect();
        Self {
            messages,
            reply_to_ids,
        }
    }

    /// Combined content of all messages, formatted with author attribution.
    /// Used when passing the batch to the LLM.
    pub fn combined_content(&self) -> String {
        if self.messages.len() == 1 {
            return self.messages[0].content.clone();
        }
        self.messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Combined content with author attribution for multi-author batches.
    pub fn combined_content_attributed(&self) -> String {
        if self.messages.len() == 1 {
            return self.messages[0].content.clone();
        }
        self.messages
            .iter()
            .enumerate()
            .map(|(i, m)| format!("{}. {}: {}", i + 1, m.source.user_id, m.content))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The first message's platform ID (for threading the reply).
    pub fn first_reply_id(&self) -> Option<&str> {
        self.reply_to_ids.first().map(|s| s.as_str())
    }

    /// Number of messages in this batch.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

/// Returns true if this message should bypass the debounce window
/// (commands and media are processed immediately).
fn should_bypass_debounce(msg: &ChannelMessage) -> bool {
    // Commands (messages starting with /)
    if msg.content.starts_with('/') {
        return true;
    }
    // Media/attachments
    if !msg.attachments.is_empty() {
        return true;
    }
    false
}

/// Configuration for the debouncer.
#[derive(Debug, Clone)]
pub struct DebouncerConfig {
    /// How long to wait after the last message before flushing (default: 1.5s).
    pub window: Duration,
    /// Maximum messages to buffer per key before force-flushing (default: 10).
    pub max_batch: usize,
}

impl Default for DebouncerConfig {
    fn default() -> Self {
        Self {
            window: Duration::from_millis(1500),
            max_batch: 10,
        }
    }
}

/// Buffered entry for a single debounce key.
struct PendingBatch {
    messages: Vec<ChannelMessage>,
    deadline: Instant,
}

/// Debounces channel messages by author+channel, flushing combined batches.
///
/// Usage:
/// ```ignore
/// let (debouncer, mut flushed_rx) = MessageDebouncer::new(config);
/// // Feed messages in:
/// debouncer.push(msg).await;
/// // Receive flushed batches:
/// while let Some(combined_msg) = flushed_rx.recv().await { ... }
/// ```
pub struct MessageDebouncer {
    tx: mpsc::Sender<ChannelMessage>,
}

impl MessageDebouncer {
    /// Create a new debouncer. Returns the debouncer handle and a receiver
    /// for flushed `MessageBatch`es ready for agent dispatch.
    ///
    /// Each batch preserves all individual messages and their platform IDs,
    /// allowing the gateway to format them for the LLM and thread replies.
    pub fn new(config: DebouncerConfig) -> (Self, mpsc::Receiver<MessageBatch>) {
        let (in_tx, mut in_rx) = mpsc::channel::<ChannelMessage>(256);
        let (out_tx, out_rx) = mpsc::channel::<MessageBatch>(256);

        tokio::spawn(async move {
            let mut pending: HashMap<String, PendingBatch> = HashMap::new();

            loop {
                // Calculate next deadline across all pending batches
                let next_deadline = pending
                    .values()
                    .map(|b| b.deadline)
                    .min();

                let timeout = match next_deadline {
                    Some(deadline) => {
                        let now = Instant::now();
                        if deadline <= now {
                            Duration::ZERO
                        } else {
                            deadline - now
                        }
                    }
                    None => Duration::from_secs(3600), // idle — wait for messages
                };

                tokio::select! {
                    msg = in_rx.recv() => {
                        match msg {
                            Some(msg) => {
                                // Bypass: commands and media skip debounce entirely.
                                // Flush any pending batch for this key first, then send immediately.
                                if should_bypass_debounce(&msg) {
                                    let key = debounce_key(&msg);
                                    if let Some(batch) = pending.remove(&key) {
                                        debug!("Debouncer: flushing {} pending before bypass for '{}'", batch.messages.len(), key);
                                        let _ = out_tx.send(MessageBatch::from_messages(batch.messages)).await;
                                    }
                                    debug!("Debouncer: bypass for command/media message");
                                    let _ = out_tx.send(MessageBatch::from_messages(vec![msg])).await;
                                    continue;
                                }

                                let key = debounce_key(&msg);
                                let now = Instant::now();

                                let batch = pending.entry(key.clone()).or_insert_with(|| PendingBatch {
                                    messages: Vec::new(),
                                    deadline: now + config.window,
                                });

                                batch.messages.push(msg);
                                batch.deadline = now + config.window;

                                // Force-flush if batch is full
                                if batch.messages.len() >= config.max_batch {
                                    debug!("Debouncer: force-flushing {} messages for key '{}'", batch.messages.len(), key);
                                    let batch = pending.remove(&key).unwrap();
                                    let _ = out_tx.send(MessageBatch::from_messages(batch.messages)).await;
                                }
                            }
                            None => break, // Input channel closed
                        }
                    }
                    _ = tokio::time::sleep(timeout) => {
                        // Flush all expired batches
                        let now = Instant::now();
                        let expired_keys: Vec<String> = pending
                            .iter()
                            .filter(|(_, b)| b.deadline <= now)
                            .map(|(k, _)| k.clone())
                            .collect();

                        for key in expired_keys {
                            if let Some(batch) = pending.remove(&key) {
                                let count = batch.messages.len();
                                if count > 1 {
                                    info!("Debouncer: flushing {} messages for '{}'", count, key);
                                }
                                let _ = out_tx.send(MessageBatch::from_messages(batch.messages)).await;
                            }
                        }
                    }
                }
            }
        });

        (Self { tx: in_tx }, out_rx)
    }

    /// Push a message into the debouncer. Returns immediately.
    pub async fn push(&self, msg: ChannelMessage) {
        let _ = self.tx.send(msg).await;
    }
}

/// Build the debounce key: `{channel_type}:{account}:{channel}:{author}`
fn debounce_key(msg: &ChannelMessage) -> String {
    let channel_type = msg.source.channel_type();
    let account = msg.source.account_id.as_deref().unwrap_or("default");
    let channel = msg.source.chat_id.as_deref().unwrap_or("dm");
    let author = &msg.source.user_id;
    format!("{}:{}:{}:{}", channel_type, account, channel, author)
}

// combine_messages removed in S53 T2 — replaced by MessageBatch which preserves all messages

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChannelSource;

    fn test_msg(user: &str, content: &str) -> ChannelMessage {
        test_msg_with_id(user, content, None)
    }

    fn test_msg_with_id(user: &str, content: &str, platform_id: Option<&str>) -> ChannelMessage {
        ChannelMessage {
            id: uuid::Uuid::new_v4().to_string(),
            content: content.to_string(),
            source: ChannelSource {
                channel_type: "discord".to_string(),
                user_id: user.to_string(),
                chat_id: Some("test-channel".to_string()),
                account_id: Some("test-account".to_string()),
                thread_id: None,
                reply_to_message_id: None,
                sender_type: zeus_core::SenderType::Unknown,
            },
            timestamp: chrono::Utc::now(),
            attachments: vec![],
            thread: None,
            text_dir: None,
            platform_message_id: platform_id.map(|s| s.to_string()),
            is_addressed: None,
        }
    }

    #[test]
    fn test_debounce_key_format() {
        let msg = test_msg("user1", "hello");
        let key = debounce_key(&msg);
        assert_eq!(key, "discord:test-account:test-channel:user1");
    }

    #[test]
    fn test_combine_single_message() {
        let msg = test_msg("user1", "hello");
        let batch = MessageBatch::from_messages(vec![msg]);
        assert_eq!(batch.combined_content(), "hello");
        assert_eq!(batch.len(), 1);
    }

    #[test]
    fn test_combine_multiple_messages() {
        let m1 = test_msg("user1", "hello");
        let m2 = test_msg("user1", "world");
        let m3 = test_msg("user1", "!");
        let batch = MessageBatch::from_messages(vec![m1, m2, m3]);
        assert_eq!(batch.combined_content(), "hello\nworld\n!");
        assert_eq!(batch.len(), 3);
    }

    #[test]
    fn test_batch_preserves_reply_ids() {
        let m1 = test_msg_with_id("user1", "hello", Some("msg-001"));
        let m2 = test_msg_with_id("user1", "world", Some("msg-002"));
        let m3 = test_msg("user1", "no-id"); // no platform_message_id
        let batch = MessageBatch::from_messages(vec![m1, m2, m3]);
        assert_eq!(batch.reply_to_ids, vec!["msg-001", "msg-002"]);
        assert_eq!(batch.first_reply_id(), Some("msg-001"));
    }

    #[test]
    fn test_batch_attributed_content() {
        let m1 = test_msg("alice", "hello");
        let m2 = test_msg("alice", "world");
        let batch = MessageBatch::from_messages(vec![m1, m2]);
        let attributed = batch.combined_content_attributed();
        assert!(attributed.contains("1. alice: hello"));
        assert!(attributed.contains("2. alice: world"));
    }

    #[test]
    fn test_should_bypass_command() {
        let cmd = test_msg("user1", "/stop");
        assert!(should_bypass_debounce(&cmd));

        let normal = test_msg("user1", "hello");
        assert!(!should_bypass_debounce(&normal));
    }

    #[test]
    fn test_should_bypass_media() {
        let mut msg = test_msg("user1", "check this");
        msg.attachments = vec![
            crate::ChannelAttachment::from_url("https://example.com/photo.jpg", "image/jpeg"),
        ];
        assert!(should_bypass_debounce(&msg));
    }

    #[tokio::test]
    async fn test_debouncer_single_message_flushes() {
        let config = DebouncerConfig {
            window: Duration::from_millis(50),
            max_batch: 10,
        };
        let (debouncer, mut rx) = MessageDebouncer::new(config);

        debouncer.push(test_msg("user1", "hello")).await;

        let batch = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("should flush within timeout")
            .expect("should receive batch");

        assert_eq!(batch.len(), 1);
        assert_eq!(batch.combined_content(), "hello");
    }

    #[tokio::test]
    async fn test_debouncer_batches_rapid_messages() {
        let config = DebouncerConfig {
            window: Duration::from_millis(100),
            max_batch: 10,
        };
        let (debouncer, mut rx) = MessageDebouncer::new(config);

        // Send 3 rapid messages from same author
        debouncer.push(test_msg("user1", "msg1")).await;
        debouncer.push(test_msg("user1", "msg2")).await;
        debouncer.push(test_msg("user1", "msg3")).await;

        let batch = tokio::time::timeout(Duration::from_millis(300), rx.recv())
            .await
            .expect("should flush within timeout")
            .expect("should receive batch");

        assert_eq!(batch.len(), 3);
        assert_eq!(batch.combined_content(), "msg1\nmsg2\nmsg3");
        // All individual messages preserved
        assert_eq!(batch.messages[0].content, "msg1");
        assert_eq!(batch.messages[1].content, "msg2");
        assert_eq!(batch.messages[2].content, "msg3");
    }

    #[tokio::test]
    async fn test_debouncer_different_authors_separate() {
        let config = DebouncerConfig {
            window: Duration::from_millis(50),
            max_batch: 10,
        };
        let (debouncer, mut rx) = MessageDebouncer::new(config);

        debouncer.push(test_msg("user1", "from-user1")).await;
        debouncer.push(test_msg("user2", "from-user2")).await;

        // Should get two separate batches
        let mut results = Vec::new();
        for _ in 0..2 {
            let batch = tokio::time::timeout(Duration::from_millis(200), rx.recv())
                .await
                .expect("should flush")
                .expect("should receive");
            results.push(batch.combined_content());
        }

        results.sort();
        assert_eq!(results, vec!["from-user1", "from-user2"]);
    }

    #[tokio::test]
    async fn test_debouncer_force_flush_on_max_batch() {
        let config = DebouncerConfig {
            window: Duration::from_secs(60), // Long window — shouldn't trigger
            max_batch: 3,
        };
        let (debouncer, mut rx) = MessageDebouncer::new(config);

        // Send exactly max_batch messages
        debouncer.push(test_msg("user1", "a")).await;
        debouncer.push(test_msg("user1", "b")).await;
        debouncer.push(test_msg("user1", "c")).await;

        // Should force-flush immediately despite long window
        let batch = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("should force-flush")
            .expect("should receive");

        assert_eq!(batch.len(), 3);
        assert_eq!(batch.combined_content(), "a\nb\nc");
    }

    #[tokio::test]
    async fn test_debouncer_command_bypasses() {
        let config = DebouncerConfig {
            window: Duration::from_secs(60), // Long window
            max_batch: 10,
        };
        let (debouncer, mut rx) = MessageDebouncer::new(config);

        // Send a command — should bypass immediately
        debouncer.push(test_msg("user1", "/status")).await;

        let batch = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("command should bypass debounce")
            .expect("should receive");

        assert_eq!(batch.len(), 1);
        assert_eq!(batch.combined_content(), "/status");
    }

    #[tokio::test]
    async fn test_debouncer_command_flushes_pending_first() {
        let config = DebouncerConfig {
            window: Duration::from_secs(60), // Long window
            max_batch: 10,
        };
        let (debouncer, mut rx) = MessageDebouncer::new(config);

        // Send a normal message (will be pending)
        debouncer.push(test_msg("user1", "hello")).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        // Send a command from same user — should flush pending first
        debouncer.push(test_msg("user1", "/stop")).await;

        // First: the flushed pending batch
        let batch1 = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("pending should flush")
            .expect("should receive");
        assert_eq!(batch1.combined_content(), "hello");

        // Second: the command itself
        let batch2 = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("command should arrive")
            .expect("should receive");
        assert_eq!(batch2.combined_content(), "/stop");
    }
}
