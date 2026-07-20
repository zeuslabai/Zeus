//! AgentInbox — unified message queue for sequential agent processing.
//!
//! All input sources (TUI, Discord, WebSocket, Telegram, Matrix, Signal,
//! Email, MQTT, WhatsApp, Mattermost, heartbeat) push messages here.
//! One consumer drains the queue and calls agent.run() or cook() sequentially.
//! This prevents concurrent session writes and the entire class of
//! "orphaned tool_use" / "error decoding response body" bugs.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::fleet_telemetry::{self, FleetEventKind, FleetSeverity};
use crate::ChannelSource;

/// Maximum queued messages before backpressure kicks in.
const INBOX_CAPACITY: usize = 64;

/// A message waiting to be processed by the agent.
pub struct InboxMessage {
    /// The user's message content (may include Discord history prefix).
    pub content: String,
    /// Where this message came from (for routing responses back).
    pub source: Option<ChannelSource>,
    /// Attachments (images, files, etc.).
    pub attachments: Vec<crate::Attachment>,
    /// Whether this message addressed the bot directly (@mention, reply, DM, name-call).
    /// Set by channel adapters; consumed by ingest path for mention-tracking
    /// (workspace.append_mention + Mnemosyne MemoryType::Mention storage).
    /// None = adapter doesn't classify; treated as false downstream.
    pub is_addressed: Option<bool>,
    /// Optional session override for local seats (TUI/API) that need a stable,
    /// explicit conversation lane instead of the gateway agent default.
    pub session_id: Option<String>,
    /// How long the consumer should wait for agent.run() to complete.
    pub timeout_secs: u64,
    /// Whether to use the cooking loop (complex tasks) or simple agent.run().
    pub use_cooking: bool,
    /// Response channel — consumer sends the result here.
    /// Uses Result so errors propagate to callers.
    pub response_tx: ResponseChannel,
}

/// Response delivery mechanism — oneshot for simple requests,
/// mpsc for streaming (TUI/WebSocket token-by-token).
pub enum ResponseChannel {
    /// Single response (API POST /v1/chat, channel adapters).
    OneShot(oneshot::Sender<Result<String, String>>),
    /// Streaming response (WebSocket, TUI streaming).
    Stream(mpsc::Sender<StreamChunk>),
}

/// A chunk of a streaming response.
pub enum StreamChunk {
    /// A token/text fragment.
    Token(String),
    /// Thinking delta (streamed from thinking models).
    Thinking(String),
    /// Tool execution started (name, input summary).
    ToolStart { name: String, input: String },
    /// Tool execution completed (name, output summary).
    ToolEnd { name: String, output: String },
    /// Cooking loop iteration number.
    Iter(usize),
    /// The final complete response.
    Done(Result<String, String>),
}

/// Handle for sending messages to the inbox.
///
/// Holds a shared `Arc<AtomicUsize>` queue-depth counter that is incremented
/// before each `tx.send` (with symmetric rollback on send-error) so external
/// observers (e.g. Heartbeat busy-aware fire-decision) can read mpsc-buffer
/// depth without owning the receiver.
#[derive(Clone)]
pub struct InboxSender {
    tx: mpsc::Sender<InboxMessage>,
    queue_depth: Arc<AtomicUsize>,
}

/// Options for enqueueing an inbox message.
pub struct InboxSendOptions {
    pub attachments: Vec<crate::Attachment>,
    pub timeout_secs: u64,
    pub use_cooking: bool,
    pub is_addressed: Option<bool>,
    pub session_id: Option<String>,
}

impl InboxSendOptions {
    pub fn new(
        attachments: Vec<crate::Attachment>,
        timeout_secs: u64,
        use_cooking: bool,
        is_addressed: Option<bool>,
    ) -> Self {
        Self {
            attachments,
            timeout_secs,
            use_cooking,
            is_addressed,
            session_id: None,
        }
    }

    pub fn with_session_id(mut self, session_id: Option<String>) -> Self {
        self.session_id = session_id;
        self
    }
}

impl InboxSender {
    /// Get a handle to the shared queue-depth counter (for observability).
    ///
    /// Reads `> 0` indicate one or more messages are waiting in the mpsc
    /// buffer to be processed by the consumer. Used by the busy-aware
    /// heartbeat fire-decision (`busy: inbound` skip-reason).
    pub fn queue_depth(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.queue_depth)
    }

    /// Send a message and wait for the response.
    pub async fn send_and_wait(
        &self,
        content: String,
        source: Option<ChannelSource>,
        attachments: Vec<crate::Attachment>,
        timeout_secs: u64,
        use_cooking: bool,
        is_addressed: Option<bool>,
    ) -> Result<String, String> {
        self.send_and_wait_with_options(
            content,
            source,
            InboxSendOptions::new(attachments, timeout_secs, use_cooking, is_addressed),
        )
        .await
    }

    /// Send a message with explicit inbox options (used by session-aware local seats).
    pub async fn send_and_wait_with_options(
        &self,
        content: String,
        source: Option<ChannelSource>,
        options: InboxSendOptions,
    ) -> Result<String, String> {
        let (response_tx, response_rx) = oneshot::channel();
        let timeout_secs = options.timeout_secs;
        let use_cooking = options.use_cooking;
        let session_id = options.session_id.clone();
        let telemetry_channel = source
            .as_ref()
            .map(|s| s.channel_type.as_str())
            .unwrap_or("unknown")
            .to_string();
        let telemetry_channel_id = source.as_ref().and_then(|s| s.channel_id.clone());
        let content_len = content.len();
        let msg = InboxMessage {
            content,
            source,
            attachments: options.attachments,
            is_addressed: options.is_addressed,
            session_id: options.session_id,
            timeout_secs,
            use_cooking,
            response_tx: ResponseChannel::OneShot(response_tx),
        };
        // Counter-invariant: increment BEFORE enqueue, symmetric rollback on send-err.
        self.queue_depth.fetch_add(1, Ordering::Relaxed);
        self.tx.send(msg).await.map_err(|_| {
            self.queue_depth.fetch_sub(1, Ordering::Relaxed);
            "Agent inbox closed".to_string()
        })?;
        match tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            response_rx,
        ).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Agent dropped response channel".to_string()),
            Err(_) => {
                let summary = format!("agent cook timed out after {timeout_secs}s");
                if use_cooking {
                    let details = format!(
                        "timeout_secs={timeout_secs} use_cooking={use_cooking} channel={} channel_id={} session_id={} content_bytes={content_len}",
                        telemetry_channel,
                        telemetry_channel_id.as_deref().unwrap_or(""),
                        session_id.as_deref().unwrap_or("")
                    );
                    fleet_telemetry::record_event_best_effort(
                        FleetEventKind::CookTimeout,
                        FleetSeverity::Error,
                        "cook-wrapper",
                        &summary,
                        None,
                        Some(&details),
                    );
                }
                Err(summary)
            }
        }
    }

    /// Send a message through the agent and receive the result via a stream channel.
    /// Returns an mpsc::Receiver<StreamChunk> — caller gets `Done(result)` when agent finishes.
    /// This routes through the full agent pipeline (tools, cooking loop) unlike bare LLM streaming.
    pub async fn send_and_stream(
        &self,
        content: String,
        source: Option<ChannelSource>,
        attachments: Vec<crate::Attachment>,
        timeout_secs: u64,
        use_cooking: bool,
        is_addressed: Option<bool>,
    ) -> Result<mpsc::Receiver<StreamChunk>, String> {
        self.send_and_stream_with_options(
            content,
            source,
            InboxSendOptions::new(attachments, timeout_secs, use_cooking, is_addressed),
        )
        .await
    }

    /// Stream a message with explicit inbox options (used by session-aware local seats).
    pub async fn send_and_stream_with_options(
        &self,
        content: String,
        source: Option<ChannelSource>,
        options: InboxSendOptions,
    ) -> Result<mpsc::Receiver<StreamChunk>, String> {
        let (stream_tx, stream_rx) = mpsc::channel::<StreamChunk>(64);
        let msg = InboxMessage {
            content,
            source,
            attachments: options.attachments,
            is_addressed: options.is_addressed,
            session_id: options.session_id,
            timeout_secs: options.timeout_secs,
            use_cooking: options.use_cooking,
            response_tx: ResponseChannel::Stream(stream_tx),
        };
        // Counter-invariant: increment BEFORE enqueue, symmetric rollback on send-err.
        self.queue_depth.fetch_add(1, Ordering::Relaxed);
        self.tx.send(msg).await.map_err(|_| {
            self.queue_depth.fetch_sub(1, Ordering::Relaxed);
            "Agent inbox closed".to_string()
        })?;
        Ok(stream_rx)
    }

    /// Send a message without waiting (fire-and-forget, e.g. heartbeat).
    /// Returns false if the inbox is full (backpressure).
    pub fn try_send(
        &self,
        content: String,
        source: Option<ChannelSource>,
        timeout_secs: u64,
        use_cooking: bool,
    ) -> bool {
        let (response_tx, _response_rx) = oneshot::channel();
        let msg = InboxMessage {
            content,
            source,
            attachments: vec![],
            is_addressed: None,
            session_id: None,
            timeout_secs,
            use_cooking,
            response_tx: ResponseChannel::OneShot(response_tx),
        };
        // Counter-invariant: increment BEFORE enqueue, symmetric rollback on send-err.
        self.queue_depth.fetch_add(1, Ordering::Relaxed);
        if self.tx.try_send(msg).is_ok() {
            true
        } else {
            self.queue_depth.fetch_sub(1, Ordering::Relaxed);
            false
        }
    }
}

/// Create an inbox triple (sender + receiver + shared queue-depth counter).
///
/// The receiver should be spawned into a tokio task and drained via
/// [`run_consumer`]. The `Arc<AtomicUsize>` is the shared queue-depth counter
/// that `InboxSender` increments/decrements at every send-site (with symmetric
/// rollback on send-error) and `run_consumer` decrements before each handler
/// dispatch. External observers (e.g. busy-aware Heartbeat) clone the Arc and
/// read `> 0` as the `busy: inbound` skip-signal.
pub fn create_inbox() -> (InboxSender, mpsc::Receiver<InboxMessage>, Arc<AtomicUsize>) {
    let (tx, rx) = mpsc::channel(INBOX_CAPACITY);
    let queue_depth = Arc::new(AtomicUsize::new(0));
    (InboxSender { tx, queue_depth: Arc::clone(&queue_depth) }, rx, queue_depth)
}

/// Run the inbox consumer loop. Processes messages one at a time.
///
/// `handler` is called for each message and should return the response string.
/// This function runs forever until the sender half is dropped.
///
/// `queue_depth` is decremented BEFORE each handler dispatch (panic-drift
/// mitigation: if the handler panics or runs long, the queue-depth signal
/// must already reflect "received from buffer, no longer queued"). Cook-flight
/// "busy: cook" is the orthogonal in-flight signal via CookState.
pub async fn run_consumer<F, Fut>(
    mut rx: mpsc::Receiver<InboxMessage>,
    queue_depth: Arc<AtomicUsize>,
    handler: F,
) where
    F: Fn(InboxMessage) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    info!("AgentInbox consumer started (capacity: {})", INBOX_CAPACITY);
    while let Some(msg) = rx.recv().await {
        // Counter-invariant: decrement BEFORE handler dispatch (panic-drift mitigation).
        queue_depth.fetch_sub(1, Ordering::Relaxed);
        let timeout = std::time::Duration::from_secs(msg.timeout_secs);
        let source_desc = msg.source.as_ref()
            .map(|s| format!("{}/{}", s.channel_type, s.channel_id.as_deref().unwrap_or("?")))
            .unwrap_or_else(|| "unknown".to_string());

        info!("Inbox: processing message from {} ({} chars)", source_desc, msg.content.len());

        let response_tx = msg.response_tx;
        let result = match tokio::time::timeout(timeout, handler(InboxMessage {
            content: msg.content,
            source: msg.source,
            attachments: msg.attachments,
            is_addressed: msg.is_addressed,
            session_id: msg.session_id,
            timeout_secs: msg.timeout_secs,
            use_cooking: msg.use_cooking,
            // Pass a dummy response channel to the handler — we handle routing ourselves
            response_tx: ResponseChannel::OneShot(oneshot::channel().0),
        })).await {
            Ok(result) => result,
            Err(_) => {
                warn!("Inbox: message from {} timed out ({}s)", source_desc, msg.timeout_secs);
                Err(format!("Processing timed out ({}s)", msg.timeout_secs))
            }
        };

        // Route response back to caller
        match response_tx {
            ResponseChannel::OneShot(tx) => {
                let _ = tx.send(result);
            }
            ResponseChannel::Stream(tx) => {
                let chunk = StreamChunk::Done(result);
                let _ = tx.send(chunk).await;
            }
        }
    }
    info!("AgentInbox consumer shut down (sender dropped)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_inbox_send_and_receive() {
        let (sender, mut rx, _depth) = create_inbox();

        let handle = tokio::spawn(async move {
            if let Some(msg) = rx.recv().await {
                assert_eq!(msg.content, "hello");
                if let ResponseChannel::OneShot(tx) = msg.response_tx {
                    let _ = tx.send(Ok("world".to_string()));
                }
            }
        });

        let result = sender.send_and_wait(
            "hello".to_string(), None, vec![], 10, false, None,
        ).await;

        assert_eq!(result, Ok("world".to_string()));
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_inbox_backpressure() {
        let (sender, _rx, _depth) = create_inbox();

        // Fill the channel
        for i in 0..INBOX_CAPACITY {
            let (tx, _) = oneshot::channel();
            let msg = InboxMessage {
                content: format!("msg {}", i),
                source: None,
                attachments: vec![],
                is_addressed: None,
                session_id: None,
                timeout_secs: 10,
                use_cooking: false,
                response_tx: ResponseChannel::OneShot(tx),
            };
            assert!(sender.tx.try_send(msg).is_ok());
        }

        // Next send should fail (backpressure)
        assert!(!sender.try_send("overflow".to_string(), None, 10, false));
    }

    #[tokio::test]
    async fn test_inbox_sequential_processing() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (sender, rx, depth) = create_inbox();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        // Consumer increments counter for each message
        tokio::spawn(async move {
            run_consumer(rx, depth, move |_msg| {
                let c = counter_clone.clone();
                async move {
                    let val = c.fetch_add(1, Ordering::SeqCst);
                    // Small delay to prove sequential processing
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    Ok(format!("processed {}", val))
                }
            }).await;
        });

        // Send 3 messages concurrently
        let mut handles = vec![];
        for i in 0..3 {
            let s = sender.clone();
            handles.push(tokio::spawn(async move {
                s.send_and_wait(format!("msg {}", i), None, vec![], 10, false, None).await
            }));
        }

        for h in handles {
            let result = h.await.unwrap();
            assert!(result.is_ok());
        }

        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::time::Duration;

    // ── Timeout propagation ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_timeout_returns_error() {
        let (sender, rx, depth) = create_inbox();

        // Consumer that never responds (simulates hung agent)
        tokio::spawn(async move {
            run_consumer(rx, depth, |_msg| async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok("never".to_string())
            })
            .await;
        });

        let result = sender
            .send_and_wait("ping".to_string(), None, vec![], 1, false, None)
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("timed out") || err.contains("timeout"), "got: {}", err);
    }

    // ── Error propagation ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_error_propagates_to_caller() {
        let (sender, rx, depth) = create_inbox();

        tokio::spawn(async move {
            run_consumer(rx, depth, |_msg| async move {
                Err("agent exploded".to_string())
            })
            .await;
        });

        let result = sender
            .send_and_wait("trigger error".to_string(), None, vec![], 10, false, None)
            .await;

        assert_eq!(result, Err("agent exploded".to_string()));
    }

    // ── Stream channel delivery ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_stream_channel_receives_done_chunk() {
        let (sender, rx, depth) = create_inbox();

        tokio::spawn(async move {
            run_consumer(rx, depth, |_msg| async move { Ok("streamed".to_string()) }).await;
        });

        // Manually send a message with ResponseChannel::Stream
        let (stream_tx, mut stream_rx) = tokio::sync::mpsc::channel::<StreamChunk>(8);
        let msg = InboxMessage {
            content: "stream me".to_string(),
            source: None,
            attachments: vec![],
            is_addressed: None,
            session_id: None,
            timeout_secs: 10,
            use_cooking: false,
            response_tx: ResponseChannel::Stream(stream_tx),
        };
        sender.tx.send(msg).await.unwrap();

        match stream_rx.recv().await.expect("should receive chunk") {
            StreamChunk::Done(Ok(s)) => assert_eq!(s, "streamed"),
            StreamChunk::Done(Err(e)) => panic!("unexpected error: {}", e),
            StreamChunk::Token(_) => panic!("expected Done, got Token"),
            StreamChunk::ToolStart { .. } | StreamChunk::ToolEnd { .. } | StreamChunk::Iter(_) => {
                panic!("expected Done, got tool/iter chunk")
            }
            StreamChunk::Thinking(_) => panic!("expected Done, got Thinking chunk"),
        }
    }

    #[tokio::test]
    async fn test_stream_preserves_session_override() {
        let (sender, rx, depth) = create_inbox();

        let handle = tokio::spawn(async move {
            run_consumer(rx, depth, |msg| async move {
                assert_eq!(msg.content, "go check lightdm");
                assert_eq!(msg.session_id.as_deref(), Some("agent:main:main"));
                assert!(msg.use_cooking, "streaming chat must stay on the cook path");
                Ok("checked".to_string())
            })
            .await;
        });

        let mut stream_rx = sender
            .send_and_stream_with_options(
                "go check lightdm".to_string(),
                None,
                InboxSendOptions::new(vec![], 10, true, None)
                    .with_session_id(Some("agent:main:main".to_string())),
            )
            .await
            .expect("stream receiver");

        match stream_rx.recv().await.expect("done chunk") {
            StreamChunk::Done(Ok(s)) => assert_eq!(s, "checked"),
            _ => panic!("expected Done(Ok) stream chunk"),
        }

        drop(sender);
        handle.await.unwrap();
    }

    // ── Sequential ordering guarantee ───────────────────────────────────────

    #[tokio::test]
    async fn test_messages_processed_in_order() {
        let (sender, rx, depth) = create_inbox();
        let log = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
        let log_clone = log.clone();

        tokio::spawn(async move {
            run_consumer(rx, depth, move |msg| {
                let log = log_clone.clone();
                async move {
                    log.lock().await.push(msg.content.clone());
                    Ok(msg.content)
                }
            })
            .await;
        });

        // Fire 5 messages sequentially
        for i in 0..5u32 {
            sender
                .send_and_wait(format!("msg{}", i), None, vec![], 10, false, None)
                .await
                .unwrap();
        }

        let order = log.lock().await.clone();
        assert_eq!(order, vec!["msg0", "msg1", "msg2", "msg3", "msg4"]);
    }

    // ── Inbox closed: sender dropped ────────────────────────────────────────

    #[tokio::test]
    async fn test_consumer_exits_when_sender_dropped() {
        let (sender, rx, depth) = create_inbox();
        let done = Arc::new(AtomicUsize::new(0));
        let done_clone = done.clone();

        let handle = tokio::spawn(async move {
            run_consumer(rx, depth, |_msg| async move { Ok("ok".to_string()) }).await;
            done_clone.store(1, Ordering::SeqCst);
        });

        drop(sender);
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("consumer should exit after sender dropped")
            .unwrap();

        assert_eq!(done.load(Ordering::SeqCst), 1);
    }

    // ── Counter-invariant: queue-depth tracks mpsc-buffer depth ─────────────
    //
    // Spec §3.1: `busy: inbound` skip-signal is `queue_depth > 0`.
    // Counter-invariant: increment-BEFORE-enqueue at sender, decrement-BEFORE-handler
    // at consumer, symmetric rollback on send-err. Counter ≥ 0 under any interleaving.

    #[tokio::test]
    async fn test_queue_depth_zero_at_rest() {
        let (_sender, _rx, depth) = create_inbox();
        assert_eq!(depth.load(Ordering::Relaxed), 0,
            "freshly-created inbox must have queue_depth = 0");
    }

    #[tokio::test]
    async fn test_queue_depth_increments_on_send() {
        let (sender, _rx, depth) = create_inbox();
        // try_send is sync and doesn't drain — perfect for observing increment
        let ok = sender.try_send("msg".to_string(), None, 10, false);
        assert!(ok, "try_send should succeed on empty inbox");
        assert_eq!(depth.load(Ordering::Relaxed), 1,
            "queue_depth must increment after successful send");
    }

    #[tokio::test]
    async fn test_queue_depth_decrements_after_consume() {
        let (sender, rx, depth) = create_inbox();
        let depth_observe = Arc::clone(&depth);
        let depth_in_consumer = Arc::clone(&depth);

        let handle = tokio::spawn(async move {
            run_consumer(rx, depth_in_consumer, |_msg| async move { Ok("ok".to_string()) }).await;
        });

        for _ in 0..3 {
            sender.send_and_wait("msg".to_string(), None, vec![], 10, false, None).await.ok();
        }

        // Allow consumer to drain
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(depth_observe.load(Ordering::Relaxed), 0,
            "queue_depth must return to 0 after all messages consumed");

        drop(sender);
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
    }

    #[tokio::test]
    async fn test_queue_depth_rolls_back_on_send_to_closed_channel() {
        // Construct a closed-receiver scenario: drop rx before send.
        let (sender, rx, depth) = create_inbox();
        drop(rx);

        let result = sender.send_and_wait("msg".to_string(), None, vec![], 10, false, None).await;
        assert!(result.is_err(), "send to closed channel must error");
        assert_eq!(depth.load(Ordering::Relaxed), 0,
            "queue_depth must roll back to 0 on send-error (symmetric rollback invariant)");
    }

    #[tokio::test]
    async fn test_queue_depth_try_send_rolls_back_when_full() {
        // INBOX_CAPACITY = 64; flood past it without draining.
        let (sender, _rx, depth) = create_inbox();
        let mut accepted = 0usize;
        for _ in 0..(INBOX_CAPACITY + 8) {
            if sender.try_send("msg".to_string(), None, 10, false) {
                accepted += 1;
            }
        }
        assert_eq!(accepted, INBOX_CAPACITY,
            "exactly INBOX_CAPACITY try_sends should succeed before backpressure");
        assert_eq!(depth.load(Ordering::Relaxed), INBOX_CAPACITY,
            "queue_depth must equal accepted-count (failed try_sends rolled back)");
    }
}
