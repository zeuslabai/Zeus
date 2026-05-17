//! Streaming delivery support for channels with message editing capabilities.
//!
//! Provides the ability to deliver LLM streaming responses to messaging channels
//! with live message updates. Channels that support message editing (Telegram,
//! Discord, Slack) will see incremental updates, while non-editable channels
//! (Email, iMessage) receive the complete response once streaming completes.

use async_trait::async_trait;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use zeus_core::Result;

use crate::ChannelSource;

/// Extended trait for channels that support message editing (for streaming delivery).
///
/// Channels like Telegram, Discord, and Slack support editing previously sent
/// messages, which enables live streaming updates. Channels that implement this
/// trait can deliver streaming LLM responses with periodic message edits.
#[async_trait]
pub trait EditableChannel: Send + Sync {
    /// Send an initial message, returns a message ID for later editing.
    ///
    /// This is called with the first chunk of content when the minimum character
    /// threshold is reached during streaming.
    async fn send_initial(&self, to: &ChannelSource, content: &str) -> Result<String>;

    /// Edit an existing message by ID.
    ///
    /// This is called periodically during streaming to update the message with
    /// accumulated content, and once at the end with the final complete content.
    async fn edit_message(&self, to: &ChannelSource, msg_id: &str, content: &str) -> Result<()>;

    /// Whether this channel supports editing (default: true).
    ///
    /// Override this to return false if editing is temporarily unavailable
    /// or not supported in certain contexts.
    fn supports_editing(&self) -> bool {
        true
    }
}

/// Delivers streaming token responses to channels with intelligent coalescing.
///
/// This component manages the delivery of streaming LLM responses to messaging
/// channels. It handles two modes:
///
/// 1. **Editable channels**: Sends an initial message once the minimum character
///    threshold is reached, then periodically edits the message with new content
///    as tokens arrive. Edits are coalesced to avoid rate limiting.
///
/// 2. **Non-editable channels**: Buffers all tokens in memory and sends the
///    complete response once streaming completes.
///
/// # Example
///
/// ```ignore
/// use zeus_channels::streaming::StreamingDelivery;
/// use tokio::sync::mpsc;
///
/// let delivery = StreamingDelivery::new()
///     .with_coalesce_ms(500)
///     .with_min_chars(50);
///
/// let mut token_rx = /* receive tokens from LLM */;
/// let result = delivery.deliver(
///     Some(&telegram_adapter),
///     &|content| Box::pin(simple_send(content)),
///     &channel_source,
///     &mut token_rx,
/// ).await?;
/// ```
pub struct StreamingDelivery {
    /// Minimum time between message edits in milliseconds (default: 500ms).
    ///
    /// This prevents rate limiting by coalescing rapid token updates into
    /// less frequent message edits.
    coalesce_ms: u64,

    /// Minimum characters accumulated before sending initial message (default: 50).
    ///
    /// This ensures the first message has enough content to be meaningful
    /// and avoids sending very short initial messages.
    min_chars: usize,
}

impl StreamingDelivery {
    /// Create a new streaming delivery handler with default settings.
    ///
    /// Defaults:
    /// - `coalesce_ms`: 500ms (edits at most every 500ms)
    /// - `min_chars`: 50 (wait for 50 characters before initial send)
    pub fn new() -> Self {
        Self {
            coalesce_ms: 500,
            min_chars: 50,
        }
    }

    /// Set the coalescing interval in milliseconds.
    ///
    /// This controls how frequently message edits are sent during streaming.
    /// Lower values provide more real-time updates but may hit rate limits.
    /// Higher values reduce API calls but make updates less responsive.
    ///
    /// Recommended range: 200-1000ms.
    pub fn with_coalesce_ms(mut self, ms: u64) -> Self {
        self.coalesce_ms = ms;
        self
    }

    /// Set the minimum characters before sending the initial message.
    ///
    /// This threshold prevents sending very short initial messages. Once
    /// this many characters have been accumulated, the initial message is sent.
    ///
    /// Recommended range: 20-100 characters.
    pub fn with_min_chars(mut self, chars: usize) -> Self {
        self.min_chars = chars;
        self
    }

    /// Consume a token stream and deliver to a channel with live updates.
    ///
    /// This is the main entry point for streaming delivery. It receives tokens
    /// from an mpsc channel and delivers them to the target channel with
    /// appropriate coalescing and editing logic.
    ///
    /// # Parameters
    ///
    /// - `editable`: Optional reference to an editable channel adapter. If `None`,
    ///   falls back to buffering mode.
    /// - `send_fn`: Fallback send function for non-editable channels or when
    ///   editing is not supported.
    /// - `to`: The target channel source (recipient).
    /// - `rx`: Receiver for streaming tokens from the LLM.
    ///
    /// # Returns
    ///
    /// Returns the complete accumulated content after streaming completes.
    ///
    /// # Behavior
    ///
    /// - **Editable channels**: Accumulates tokens until `min_chars` is reached,
    ///   sends initial message, then edits periodically at `coalesce_ms` intervals.
    ///   Sends a final edit when the stream closes.
    ///
    /// - **Non-editable channels**: Buffers all tokens and calls `send_fn` once
    ///   with the complete content when streaming completes.
    pub async fn deliver<F, Fut>(
        &self,
        editable: Option<&dyn EditableChannel>,
        send_fn: F,
        to: &ChannelSource,
        rx: &mut mpsc::Receiver<String>,
    ) -> Result<String>
    where
        F: Fn(&str) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let mut accumulated = String::new();
        let mut msg_id: Option<String> = None;
        let mut last_edit_time = tokio::time::Instant::now();

        // Check if we should use editable mode
        let use_editable = editable
            .as_ref()
            .map(|e| e.supports_editing())
            .unwrap_or(false);

        if !use_editable {
            // Non-editable mode: buffer everything and send at end
            while let Some(token) = rx.recv().await {
                accumulated.push_str(&token);
            }

            if !accumulated.is_empty() {
                (send_fn)(&accumulated).await?;
            }

            return Ok(accumulated);
        }

        // Editable mode: send initial message and periodic edits
        let editable = editable.expect("editable channel verified above");

        while let Some(token) = rx.recv().await {
            accumulated.push_str(&token);

            // Send initial message once we have enough content
            if msg_id.is_none() && accumulated.len() >= self.min_chars {
                let id = editable.send_initial(to, &accumulated).await?;
                msg_id = Some(id);
                last_edit_time = tokio::time::Instant::now();
                continue;
            }

            // Edit message if enough time has passed since last edit
            if let Some(ref id) = msg_id {
                let elapsed = last_edit_time.elapsed();
                if elapsed >= Duration::from_millis(self.coalesce_ms) {
                    editable.edit_message(to, id, &accumulated).await?;
                    last_edit_time = tokio::time::Instant::now();
                }
            }
        }

        // Final edit with complete content
        if let Some(ref id) = msg_id {
            // Small delay to ensure any in-flight tokens have arrived
            sleep(Duration::from_millis(50)).await;
            editable.edit_message(to, id, &accumulated).await?;
        } else if !accumulated.is_empty() {
            // If we never reached min_chars, send what we have
            editable.send_initial(to, &accumulated).await?;
        }

        Ok(accumulated)
    }
}

impl Default for StreamingDelivery {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    // Mock editable channel for testing
    #[derive(Clone)]
    struct MockEditableChannel {
        sent_messages: Arc<Mutex<Vec<String>>>,
        edited_messages: Arc<Mutex<Vec<(String, String)>>>,
        supports_editing: bool,
    }

    impl MockEditableChannel {
        fn new() -> Self {
            Self {
                sent_messages: Arc::new(Mutex::new(Vec::new())),
                edited_messages: Arc::new(Mutex::new(Vec::new())),
                supports_editing: true,
            }
        }

        fn without_editing() -> Self {
            Self {
                sent_messages: Arc::new(Mutex::new(Vec::new())),
                edited_messages: Arc::new(Mutex::new(Vec::new())),
                supports_editing: false,
            }
        }

        async fn get_sent_count(&self) -> usize {
            self.sent_messages.lock().await.len()
        }

        async fn get_edit_count(&self) -> usize {
            self.edited_messages.lock().await.len()
        }

        async fn get_last_sent(&self) -> Option<String> {
            self.sent_messages.lock().await.last().cloned()
        }

        async fn get_last_edited(&self) -> Option<String> {
            self.edited_messages
                .lock()
                .await
                .last()
                .map(|(_, content)| content.clone())
        }
    }

    #[async_trait]
    impl EditableChannel for MockEditableChannel {
        async fn send_initial(&self, _to: &ChannelSource, content: &str) -> Result<String> {
            let mut sent = self.sent_messages.lock().await;
            sent.push(content.to_string());
            Ok(format!("msg_{}", sent.len()))
        }

        async fn edit_message(
            &self,
            _to: &ChannelSource,
            msg_id: &str,
            content: &str,
        ) -> Result<()> {
            let mut edits = self.edited_messages.lock().await;
            edits.push((msg_id.to_string(), content.to_string()));
            Ok(())
        }

        fn supports_editing(&self) -> bool {
            self.supports_editing
        }
    }

    #[tokio::test]
    async fn test_streaming_delivery_defaults() {
        let delivery = StreamingDelivery::new();
        assert_eq!(delivery.coalesce_ms, 500);
        assert_eq!(delivery.min_chars, 50);
    }

    #[tokio::test]
    async fn test_streaming_delivery_builder() {
        let delivery = StreamingDelivery::new()
            .with_coalesce_ms(1000)
            .with_min_chars(100);
        assert_eq!(delivery.coalesce_ms, 1000);
        assert_eq!(delivery.min_chars, 100);
    }

    #[tokio::test]
    async fn test_non_editable_channel_buffers_full_response() {
        let (tx, mut rx) = mpsc::channel(10);
        let received = Arc::new(Mutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);

        let send_fn = |content: &str| {
            let received = Arc::clone(&received_clone);
            let content = content.to_string();
            async move {
                received.lock().await.push(content);
                Ok(())
            }
        };

        let delivery = StreamingDelivery::new().with_min_chars(10);
        let to = ChannelSource::new("test", "user123");

        // Spawn task that sends tokens
        tokio::spawn(async move {
            tx.send("Hello ".to_string())
                .await
                .expect("channel send should succeed");
            tx.send("streaming ".to_string())
                .await
                .expect("channel send should succeed");
            tx.send("world!".to_string())
                .await
                .expect("channel send should succeed");
            drop(tx); // Close channel
        });

        // Wait for delivery
        let result = delivery
            .deliver(None, send_fn, &to, &mut rx)
            .await
            .expect("async operation should succeed");

        // Verify behavior
        assert_eq!(result, "Hello streaming world!");
        let messages = received.lock().await;
        assert_eq!(messages.len(), 1, "Should send exactly once");
        assert_eq!(messages[0], "Hello streaming world!");
    }

    #[tokio::test]
    async fn test_editable_channel_sends_initial_and_edits() {
        let (tx, mut rx) = mpsc::channel(10);
        let mock_channel = MockEditableChannel::new();
        let mock_channel_check = mock_channel.clone();
        let to = ChannelSource::new("telegram", "user456");

        let send_fn = |_content: &str| async { Ok(()) };

        // Use a fast coalesce time and low min_chars for testing
        let delivery = StreamingDelivery::new()
            .with_coalesce_ms(50)
            .with_min_chars(10);

        // Spawn task that sends tokens
        tokio::spawn(async move {
            // Send enough tokens to trigger initial send
            tx.send("Hello world ".to_string())
                .await
                .expect("channel send should succeed");
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Send more tokens to trigger edits
            tx.send("this is ".to_string())
                .await
                .expect("channel send should succeed");
            tokio::time::sleep(Duration::from_millis(100)).await;

            tx.send("streaming!".to_string())
                .await
                .expect("channel send should succeed");
            tokio::time::sleep(Duration::from_millis(100)).await;

            drop(tx);
        });

        let result = delivery
            .deliver(Some(&mock_channel), send_fn, &to, &mut rx)
            .await
            .expect("async operation should succeed");

        assert_eq!(result, "Hello world this is streaming!");
        assert_eq!(
            mock_channel_check.get_sent_count().await,
            1,
            "Should send initial message once"
        );
        assert!(
            mock_channel_check.get_edit_count().await >= 1,
            "Should have at least one edit"
        );

        // Verify final content matches
        let last_edited = mock_channel_check.get_last_edited().await;
        assert_eq!(
            last_edited,
            Some("Hello world this is streaming!".to_string())
        );
    }

    #[tokio::test]
    async fn test_coalescing_prevents_excessive_edits() {
        let (tx, mut rx) = mpsc::channel(10);
        let mock_channel = MockEditableChannel::new();
        let mock_channel_check = mock_channel.clone();
        let to = ChannelSource::new("discord", "user789");

        let send_fn = |_content: &str| async { Ok(()) };

        // High coalesce time means few edits
        let delivery = StreamingDelivery::new()
            .with_coalesce_ms(500)
            .with_min_chars(5);

        // Spawn task that sends tokens
        tokio::spawn(async move {
            // Send many tokens quickly
            for i in 0..10 {
                tx.send(format!("token{} ", i))
                    .await
                    .expect("channel send should succeed");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            drop(tx);
        });

        let result = delivery
            .deliver(Some(&mock_channel), send_fn, &to, &mut rx)
            .await
            .expect("async operation should succeed");

        assert!(result.contains("token0"));
        assert!(result.contains("token9"));

        // With 500ms coalesce and 10ms intervals, we should have very few edits
        let edit_count = mock_channel_check.get_edit_count().await;
        assert!(
            edit_count <= 3,
            "Should coalesce to ≤3 edits, got {}",
            edit_count
        );
    }

    #[tokio::test]
    async fn test_min_chars_threshold() {
        let (tx, mut rx) = mpsc::channel(10);
        let mock_channel = MockEditableChannel::new();
        let mock_channel_check = mock_channel.clone();
        let to = ChannelSource::new("slack", "user000");

        let send_fn = |_content: &str| async { Ok(()) };

        let delivery = StreamingDelivery::new()
            .with_coalesce_ms(50)
            .with_min_chars(20);

        // Spawn task that sends tokens
        tokio::spawn(async move {
            // Send less than min_chars first
            tx.send("Short".to_string())
                .await
                .expect("channel send should succeed");
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Now send enough to exceed threshold
            tx.send(" message that is now long enough!".to_string())
                .await
                .expect("channel send should succeed");
            tokio::time::sleep(Duration::from_millis(100)).await;

            drop(tx);
        });

        // Check before min_chars reached
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Note: we can't check mid-stream anymore, so we'll just verify final state

        delivery
            .deliver(Some(&mock_channel), send_fn, &to, &mut rx)
            .await
            .expect("async operation should succeed");

        // Should have sent after exceeding threshold
        assert_eq!(mock_channel_check.get_sent_count().await, 1);
        let last_sent = mock_channel_check
            .get_last_sent()
            .await
            .expect("async operation should succeed");
        assert!(last_sent.len() >= 20);
    }

    #[tokio::test]
    async fn test_empty_stream() {
        let (tx, mut rx) = mpsc::channel::<String>(10);
        let mock_channel = MockEditableChannel::new();
        let mock_channel_check = mock_channel.clone();
        let to = ChannelSource::new("telegram", "user111");

        let send_fn = |_content: &str| async { Ok(()) };

        let delivery = StreamingDelivery::new();

        // Close the channel immediately to signal end of stream
        drop(tx);

        let result = delivery
            .deliver(Some(&mock_channel), send_fn, &to, &mut rx)
            .await
            .expect("async operation should succeed");

        assert_eq!(result, "");
        assert_eq!(mock_channel_check.get_sent_count().await, 0);
        assert_eq!(mock_channel_check.get_edit_count().await, 0);
    }

    #[tokio::test]
    async fn test_channel_without_editing_support() {
        let (tx, mut rx) = mpsc::channel(10);
        let mock_channel = MockEditableChannel::without_editing();
        let mock_channel_check = mock_channel.clone();
        let received = Arc::new(Mutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);

        let send_fn = |content: &str| {
            let received = Arc::clone(&received_clone);
            let content = content.to_string();
            async move {
                received.lock().await.push(content);
                Ok(())
            }
        };

        let delivery = StreamingDelivery::new().with_min_chars(10);
        let to = ChannelSource::new("email", "user222");

        // Spawn task that sends tokens
        tokio::spawn(async move {
            tx.send("Hello ".to_string())
                .await
                .expect("channel send should succeed");
            tx.send("from ".to_string())
                .await
                .expect("channel send should succeed");
            tx.send("email!".to_string())
                .await
                .expect("channel send should succeed");
            drop(tx);
        });

        let result = delivery
            .deliver(Some(&mock_channel), send_fn, &to, &mut rx)
            .await
            .expect("async operation should succeed");

        // Should buffer and send once via send_fn
        assert_eq!(result, "Hello from email!");
        let messages = received.lock().await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], "Hello from email!");

        // Should not have used editable methods
        assert_eq!(mock_channel_check.get_sent_count().await, 0);
        assert_eq!(mock_channel_check.get_edit_count().await, 0);
    }
}
