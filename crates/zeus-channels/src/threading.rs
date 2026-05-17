//! Threading / Reply System for Zeus Channels
//!
//! Provides thread-aware messaging across all channel adapters:
//! - `ReplyMode` controls auto-reply behavior (First, All, Off)
//! - `ThreadContext` carries platform-specific thread metadata
//! - `ThreadingConfig` per-channel thread settings
//! - `ThreadRouter` decides whether inbound messages should get a reply
//! - `ThreadedReplyOptions` controls outbound reply behavior

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

/// How the bot auto-replies to messages in threads
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyMode {
    /// Reply only to the first message in a thread (ignore follow-ups)
    First,
    /// Reply to every message in a thread
    #[default]
    All,
    /// Do not auto-reply — only respond when explicitly mentioned or commanded
    Off,
}

impl std::fmt::Display for ReplyMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::First => write!(f, "first"),
            Self::All => write!(f, "all"),
            Self::Off => write!(f, "off"),
        }
    }
}

/// Thread context attached to an inbound or outbound message
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadContext {
    /// Platform-specific thread ID (e.g., Slack thread_ts, Discord thread/channel ID)
    pub thread_id: String,
    /// Message ID of the root/parent message that started the thread
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
    /// User who started the thread
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_user_id: Option<String>,
    /// Number of replies in the thread (if known)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_count: Option<u32>,
}

impl ThreadContext {
    /// Create a new thread context with just a thread ID
    pub fn new(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            parent_message_id: None,
            parent_user_id: None,
            reply_count: None,
        }
    }

    /// Builder: set parent message ID
    pub fn with_parent_message(mut self, msg_id: impl Into<String>) -> Self {
        self.parent_message_id = Some(msg_id.into());
        self
    }

    /// Builder: set parent user ID
    pub fn with_parent_user(mut self, user_id: impl Into<String>) -> Self {
        self.parent_user_id = Some(user_id.into());
        self
    }

    /// Builder: set reply count
    pub fn with_reply_count(mut self, count: u32) -> Self {
        self.reply_count = Some(count);
        self
    }

    /// Whether this is the first message in the thread
    pub fn is_thread_start(&self) -> bool {
        self.reply_count.is_none_or(|c| c == 0)
    }
}

/// Per-channel threading configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadingConfig {
    /// How the bot handles replies in threads
    pub reply_mode: ReplyMode,
    /// Whether outbound replies should stay in the same thread as the inbound message
    pub keep_in_thread: bool,
    /// Maximum thread depth before starting a new top-level message (0 = unlimited)
    pub max_thread_depth: u32,
    /// Whether to broadcast thread replies to the main channel (Slack-specific)
    pub broadcast_replies: bool,
}

impl Default for ThreadingConfig {
    fn default() -> Self {
        Self {
            reply_mode: ReplyMode::All,
            keep_in_thread: true,
            max_thread_depth: 0,
            broadcast_replies: false,
        }
    }
}

/// Options passed when sending a threaded reply
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreadedReplyOptions {
    /// Thread to reply in (platform-specific ID)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Specific message to reply to within the thread
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    /// Whether to also broadcast to the main channel (Slack reply_broadcast)
    #[serde(default)]
    pub broadcast: bool,
}

impl ThreadedReplyOptions {
    /// Create options that reply in a specific thread
    pub fn in_thread(thread_id: impl Into<String>) -> Self {
        Self {
            thread_id: Some(thread_id.into()),
            reply_to_message_id: None,
            broadcast: false,
        }
    }

    /// Create options that reply to a specific message
    pub fn reply_to(message_id: impl Into<String>) -> Self {
        Self {
            thread_id: None,
            reply_to_message_id: Some(message_id.into()),
            broadcast: false,
        }
    }

    /// Create options that reply in a thread and to a specific message
    pub fn in_thread_reply_to(thread_id: impl Into<String>, message_id: impl Into<String>) -> Self {
        Self {
            thread_id: Some(thread_id.into()),
            reply_to_message_id: Some(message_id.into()),
            broadcast: false,
        }
    }

    /// Builder: set broadcast flag
    pub fn with_broadcast(mut self, broadcast: bool) -> Self {
        self.broadcast = broadcast;
        self
    }

    /// Whether any threading info is present
    pub fn has_threading(&self) -> bool {
        self.thread_id.is_some() || self.reply_to_message_id.is_some()
    }

    /// Build from a ThreadContext (used for auto-reply)
    pub fn from_context(ctx: &ThreadContext, config: &ThreadingConfig) -> Self {
        Self {
            thread_id: if config.keep_in_thread {
                Some(ctx.thread_id.clone())
            } else {
                None
            },
            reply_to_message_id: ctx.parent_message_id.clone(),
            broadcast: config.broadcast_replies,
        }
    }
}

/// Decides whether an inbound message should receive an auto-reply based on
/// threading configuration and thread state.
pub struct ThreadRouter {
    /// Per-channel-type threading configs
    configs: HashMap<String, ThreadingConfig>,
    /// Tracks which threads we've already replied to (thread_id -> replied)
    /// Used for ReplyMode::First to avoid replying to follow-up messages.
    replied_threads: RwLock<HashMap<String, bool>>,
}

impl ThreadRouter {
    /// Create a new thread router with default config for all channels
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
            replied_threads: RwLock::new(HashMap::new()),
        }
    }

    /// Set the threading config for a specific channel type
    pub fn set_config(&mut self, channel_type: impl Into<String>, config: ThreadingConfig) {
        self.configs.insert(channel_type.into(), config);
    }

    /// Get the threading config for a channel type (returns default if not set)
    pub fn config_for(&self, channel_type: &str) -> ThreadingConfig {
        self.configs.get(channel_type).cloned().unwrap_or_default()
    }

    /// Decide whether to reply to an inbound message based on its thread context
    ///
    /// Returns `true` if the bot should generate a reply, `false` to skip.
    pub fn should_reply(&self, channel_type: &str, thread: Option<&ThreadContext>) -> bool {
        let config = self.config_for(channel_type);

        match config.reply_mode {
            ReplyMode::Off => false,
            ReplyMode::All => true,
            ReplyMode::First => {
                match thread {
                    // No thread context — top-level message, always reply
                    None => true,
                    Some(ctx) => {
                        let mut replied = self
                            .replied_threads
                            .write()
                            .unwrap_or_else(|e| e.into_inner());
                        if *replied.get(&ctx.thread_id).unwrap_or(&false) {
                            // Already replied to this thread
                            false
                        } else {
                            // First reply in this thread — mark it
                            replied.insert(ctx.thread_id.clone(), true);
                            true
                        }
                    }
                }
            }
        }
    }

    /// Build reply options for an outbound response to an inbound message
    ///
    /// If the inbound message has thread context and the config says keep_in_thread,
    /// the reply options will include the thread_id so the adapter replies in-thread.
    pub fn reply_options(
        &self,
        channel_type: &str,
        thread: Option<&ThreadContext>,
    ) -> ThreadedReplyOptions {
        let config = self.config_for(channel_type);

        match thread {
            Some(ctx) => ThreadedReplyOptions::from_context(ctx, &config),
            None => ThreadedReplyOptions::default(),
        }
    }

    /// Mark a thread as replied-to (for external tracking)
    pub fn mark_replied(&self, thread_id: &str) {
        let mut replied = self
            .replied_threads
            .write()
            .unwrap_or_else(|e| e.into_inner());
        replied.insert(thread_id.to_string(), true);
    }

    /// Reset reply tracking for a thread (e.g., after timeout)
    pub fn reset_thread(&self, thread_id: &str) {
        let mut replied = self
            .replied_threads
            .write()
            .unwrap_or_else(|e| e.into_inner());
        replied.remove(thread_id);
    }

    /// Clear all reply tracking state
    pub fn clear_tracking(&self) {
        let mut replied = self
            .replied_threads
            .write()
            .unwrap_or_else(|e| e.into_inner());
        replied.clear();
    }

    /// Number of threads currently tracked
    pub fn tracked_thread_count(&self) -> usize {
        self.replied_threads
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }
}

impl Default for ThreadRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Inject thread metadata into a context string for the LLM
///
/// This adds thread awareness to the system prompt so the LLM knows
/// it's responding within a thread and can reference the parent message.
pub fn inject_thread_context(
    existing_context: &str,
    thread: &ThreadContext,
    channel_type: &str,
) -> String {
    let mut ctx = existing_context.to_string();
    ctx.push_str("\n\n[Thread Context]\n");
    ctx.push_str(&format!("Channel: {}\n", channel_type));
    ctx.push_str(&format!("Thread ID: {}\n", thread.thread_id));
    if let Some(ref parent_msg) = thread.parent_message_id {
        ctx.push_str(&format!("Parent Message: {}\n", parent_msg));
    }
    if let Some(ref parent_user) = thread.parent_user_id {
        ctx.push_str(&format!("Thread Starter: {}\n", parent_user));
    }
    if let Some(count) = thread.reply_count {
        ctx.push_str(&format!("Replies in Thread: {}\n", count));
    }
    ctx
}

/// Convert platform-specific thread identifiers to a normalized ThreadContext
///
/// This handles the various ways different platforms represent threads:
/// - Slack: thread_ts timestamp string
/// - Discord: thread channel ID or message reference ID
/// - Telegram: reply_to_message_id integer
/// - Matrix: event relation m.thread
pub fn normalize_thread_id(
    channel_type: &str,
    raw_thread_id: &str,
    raw_message_id: Option<&str>,
) -> ThreadContext {
    let mut ctx = ThreadContext::new(raw_thread_id);

    // Set the parent message — for some platforms, thread_id IS the parent message
    match channel_type {
        "slack" => {
            // Slack thread_ts is both the thread ID and the parent message timestamp
            ctx.parent_message_id = Some(raw_thread_id.to_string());
        }
        "telegram" => {
            // Telegram reply_to_message_id is the parent; thread_id might be chat-level
            if let Some(msg_id) = raw_message_id {
                ctx.parent_message_id = Some(msg_id.to_string());
            }
        }
        "discord" => {
            // Discord threads are channels; the message_id is the starter message
            if let Some(msg_id) = raw_message_id {
                ctx.parent_message_id = Some(msg_id.to_string());
            }
        }
        _ => {
            if let Some(msg_id) = raw_message_id {
                ctx.parent_message_id = Some(msg_id.to_string());
            }
        }
    }

    ctx
}

#[cfg(test)]
mod tests {
    use super::*;

    // === ReplyMode ===

    #[test]
    fn test_reply_mode_default() {
        assert_eq!(ReplyMode::default(), ReplyMode::All);
    }

    #[test]
    fn test_reply_mode_display() {
        assert_eq!(ReplyMode::First.to_string(), "first");
        assert_eq!(ReplyMode::All.to_string(), "all");
        assert_eq!(ReplyMode::Off.to_string(), "off");
    }

    #[test]
    fn test_reply_mode_serde() {
        let json = serde_json::to_string(&ReplyMode::First).expect("should serialize to JSON");
        assert_eq!(json, "\"first\"");
        let back: ReplyMode = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(back, ReplyMode::First);
    }

    // === ThreadContext ===

    #[test]
    fn test_thread_context_new() {
        let ctx = ThreadContext::new("thread-123");
        assert_eq!(ctx.thread_id, "thread-123");
        assert!(ctx.parent_message_id.is_none());
        assert!(ctx.parent_user_id.is_none());
        assert!(ctx.reply_count.is_none());
    }

    #[test]
    fn test_thread_context_builders() {
        let ctx = ThreadContext::new("t1")
            .with_parent_message("msg-1")
            .with_parent_user("user-1")
            .with_reply_count(5);
        assert_eq!(ctx.thread_id, "t1");
        assert_eq!(ctx.parent_message_id.as_deref(), Some("msg-1"));
        assert_eq!(ctx.parent_user_id.as_deref(), Some("user-1"));
        assert_eq!(ctx.reply_count, Some(5));
    }

    #[test]
    fn test_thread_context_is_thread_start() {
        let ctx_no_count = ThreadContext::new("t1");
        assert!(ctx_no_count.is_thread_start()); // unknown = assume start

        let ctx_zero = ThreadContext::new("t1").with_reply_count(0);
        assert!(ctx_zero.is_thread_start());

        let ctx_replies = ThreadContext::new("t1").with_reply_count(3);
        assert!(!ctx_replies.is_thread_start());
    }

    #[test]
    fn test_thread_context_serde() {
        let ctx = ThreadContext::new("slack-ts-123")
            .with_parent_message("msg-456")
            .with_reply_count(2);
        let json = serde_json::to_string(&ctx).expect("should serialize to JSON");
        let back: ThreadContext = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(back.thread_id, "slack-ts-123");
        assert_eq!(back.parent_message_id.as_deref(), Some("msg-456"));
        assert!(back.parent_user_id.is_none());
        assert_eq!(back.reply_count, Some(2));
    }

    #[test]
    fn test_thread_context_serde_minimal() {
        // Only thread_id should be required
        let json = r#"{"thread_id":"t1"}"#;
        let ctx: ThreadContext = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(ctx.thread_id, "t1");
        assert!(ctx.parent_message_id.is_none());
    }

    // === ThreadingConfig ===

    #[test]
    fn test_threading_config_default() {
        let cfg = ThreadingConfig::default();
        assert_eq!(cfg.reply_mode, ReplyMode::All);
        assert!(cfg.keep_in_thread);
        assert_eq!(cfg.max_thread_depth, 0);
        assert!(!cfg.broadcast_replies);
    }

    #[test]
    fn test_threading_config_serde() {
        let cfg = ThreadingConfig {
            reply_mode: ReplyMode::First,
            keep_in_thread: false,
            max_thread_depth: 10,
            broadcast_replies: true,
        };
        let json = serde_json::to_string(&cfg).expect("should serialize to JSON");
        let back: ThreadingConfig = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(back.reply_mode, ReplyMode::First);
        assert!(!back.keep_in_thread);
        assert_eq!(back.max_thread_depth, 10);
        assert!(back.broadcast_replies);
    }

    // === ThreadedReplyOptions ===

    #[test]
    fn test_reply_options_default() {
        let opts = ThreadedReplyOptions::default();
        assert!(!opts.has_threading());
        assert!(!opts.broadcast);
    }

    #[test]
    fn test_reply_options_in_thread() {
        let opts = ThreadedReplyOptions::in_thread("thread-1");
        assert!(opts.has_threading());
        assert_eq!(opts.thread_id.as_deref(), Some("thread-1"));
        assert!(opts.reply_to_message_id.is_none());
    }

    #[test]
    fn test_reply_options_reply_to() {
        let opts = ThreadedReplyOptions::reply_to("msg-42");
        assert!(opts.has_threading());
        assert!(opts.thread_id.is_none());
        assert_eq!(opts.reply_to_message_id.as_deref(), Some("msg-42"));
    }

    #[test]
    fn test_reply_options_in_thread_reply_to() {
        let opts = ThreadedReplyOptions::in_thread_reply_to("t1", "m1").with_broadcast(true);
        assert!(opts.has_threading());
        assert_eq!(opts.thread_id.as_deref(), Some("t1"));
        assert_eq!(opts.reply_to_message_id.as_deref(), Some("m1"));
        assert!(opts.broadcast);
    }

    #[test]
    fn test_reply_options_from_context() {
        let ctx = ThreadContext::new("thread-99").with_parent_message("msg-1");
        let cfg = ThreadingConfig {
            reply_mode: ReplyMode::All,
            keep_in_thread: true,
            max_thread_depth: 0,
            broadcast_replies: true,
        };
        let opts = ThreadedReplyOptions::from_context(&ctx, &cfg);
        assert_eq!(opts.thread_id.as_deref(), Some("thread-99"));
        assert_eq!(opts.reply_to_message_id.as_deref(), Some("msg-1"));
        assert!(opts.broadcast);
    }

    #[test]
    fn test_reply_options_from_context_no_keep() {
        let ctx = ThreadContext::new("thread-99");
        let cfg = ThreadingConfig {
            keep_in_thread: false,
            ..Default::default()
        };
        let opts = ThreadedReplyOptions::from_context(&ctx, &cfg);
        // keep_in_thread=false means don't include thread_id
        assert!(opts.thread_id.is_none());
        assert!(!opts.broadcast);
    }

    // === ThreadRouter ===

    #[test]
    fn test_router_default_config() {
        let router = ThreadRouter::new();
        let cfg = router.config_for("telegram");
        assert_eq!(cfg.reply_mode, ReplyMode::All);
    }

    #[test]
    fn test_router_custom_config() {
        let mut router = ThreadRouter::new();
        router.set_config(
            "slack",
            ThreadingConfig {
                reply_mode: ReplyMode::First,
                ..Default::default()
            },
        );
        assert_eq!(router.config_for("slack").reply_mode, ReplyMode::First);
        // Other channels still get default
        assert_eq!(router.config_for("telegram").reply_mode, ReplyMode::All);
    }

    #[test]
    fn test_router_should_reply_all() {
        let router = ThreadRouter::new(); // default = All
        let ctx = ThreadContext::new("t1").with_reply_count(5);
        assert!(router.should_reply("telegram", Some(&ctx)));
        assert!(router.should_reply("telegram", None));
    }

    #[test]
    fn test_router_should_reply_off() {
        let mut router = ThreadRouter::new();
        router.set_config(
            "slack",
            ThreadingConfig {
                reply_mode: ReplyMode::Off,
                ..Default::default()
            },
        );
        assert!(!router.should_reply("slack", None));
        assert!(!router.should_reply("slack", Some(&ThreadContext::new("t1"))));
    }

    #[test]
    fn test_router_should_reply_first_no_thread() {
        let mut router = ThreadRouter::new();
        router.set_config(
            "discord",
            ThreadingConfig {
                reply_mode: ReplyMode::First,
                ..Default::default()
            },
        );
        // Top-level message (no thread) — always reply
        assert!(router.should_reply("discord", None));
    }

    #[test]
    fn test_router_should_reply_first_in_thread() {
        let mut router = ThreadRouter::new();
        router.set_config(
            "slack",
            ThreadingConfig {
                reply_mode: ReplyMode::First,
                ..Default::default()
            },
        );
        let ctx = ThreadContext::new("thread-abc");

        // First time seeing this thread — reply
        assert!(router.should_reply("slack", Some(&ctx)));
        // Second time — skip
        assert!(!router.should_reply("slack", Some(&ctx)));
    }

    #[test]
    fn test_router_first_separate_threads() {
        let mut router = ThreadRouter::new();
        router.set_config(
            "slack",
            ThreadingConfig {
                reply_mode: ReplyMode::First,
                ..Default::default()
            },
        );

        let ctx1 = ThreadContext::new("thread-1");
        let ctx2 = ThreadContext::new("thread-2");

        assert!(router.should_reply("slack", Some(&ctx1)));
        assert!(router.should_reply("slack", Some(&ctx2)));
        // Now both are replied — should not reply again
        assert!(!router.should_reply("slack", Some(&ctx1)));
        assert!(!router.should_reply("slack", Some(&ctx2)));
    }

    #[test]
    fn test_router_mark_replied() {
        let mut router = ThreadRouter::new();
        router.set_config(
            "slack",
            ThreadingConfig {
                reply_mode: ReplyMode::First,
                ..Default::default()
            },
        );

        let ctx = ThreadContext::new("thread-x");

        // Pre-mark as replied
        router.mark_replied("thread-x");
        assert!(!router.should_reply("slack", Some(&ctx)));
    }

    #[test]
    fn test_router_reset_thread() {
        let mut router = ThreadRouter::new();
        router.set_config(
            "slack",
            ThreadingConfig {
                reply_mode: ReplyMode::First,
                ..Default::default()
            },
        );

        let ctx = ThreadContext::new("thread-y");

        assert!(router.should_reply("slack", Some(&ctx)));
        assert!(!router.should_reply("slack", Some(&ctx)));

        // Reset — should reply again
        router.reset_thread("thread-y");
        assert!(router.should_reply("slack", Some(&ctx)));
    }

    #[test]
    fn test_router_clear_tracking() {
        let router = ThreadRouter::new();
        router.mark_replied("t1");
        router.mark_replied("t2");
        assert_eq!(router.tracked_thread_count(), 2);
        router.clear_tracking();
        assert_eq!(router.tracked_thread_count(), 0);
    }

    #[test]
    fn test_router_reply_options_with_thread() {
        let mut router = ThreadRouter::new();
        router.set_config(
            "slack",
            ThreadingConfig {
                keep_in_thread: true,
                broadcast_replies: true,
                ..Default::default()
            },
        );

        let ctx = ThreadContext::new("ts-123").with_parent_message("msg-0");
        let opts = router.reply_options("slack", Some(&ctx));
        assert_eq!(opts.thread_id.as_deref(), Some("ts-123"));
        assert_eq!(opts.reply_to_message_id.as_deref(), Some("msg-0"));
        assert!(opts.broadcast);
    }

    #[test]
    fn test_router_reply_options_without_thread() {
        let router = ThreadRouter::new();
        let opts = router.reply_options("telegram", None);
        assert!(!opts.has_threading());
    }

    // === inject_thread_context ===

    #[test]
    fn test_inject_thread_context_full() {
        let ctx = ThreadContext::new("thread-1")
            .with_parent_message("msg-42")
            .with_parent_user("alice")
            .with_reply_count(3);
        let result = inject_thread_context("existing prompt", &ctx, "slack");
        assert!(result.starts_with("existing prompt"));
        assert!(result.contains("[Thread Context]"));
        assert!(result.contains("Channel: slack"));
        assert!(result.contains("Thread ID: thread-1"));
        assert!(result.contains("Parent Message: msg-42"));
        assert!(result.contains("Thread Starter: alice"));
        assert!(result.contains("Replies in Thread: 3"));
    }

    #[test]
    fn test_inject_thread_context_minimal() {
        let ctx = ThreadContext::new("t1");
        let result = inject_thread_context("", &ctx, "telegram");
        assert!(result.contains("Thread ID: t1"));
        assert!(!result.contains("Parent Message:"));
        assert!(!result.contains("Thread Starter:"));
    }

    // === normalize_thread_id ===

    #[test]
    fn test_normalize_slack() {
        let ctx = normalize_thread_id("slack", "1234567890.123456", None);
        assert_eq!(ctx.thread_id, "1234567890.123456");
        // Slack: thread_ts IS the parent message
        assert_eq!(ctx.parent_message_id.as_deref(), Some("1234567890.123456"));
    }

    #[test]
    fn test_normalize_telegram() {
        let ctx = normalize_thread_id("telegram", "chat-100", Some("msg-42"));
        assert_eq!(ctx.thread_id, "chat-100");
        assert_eq!(ctx.parent_message_id.as_deref(), Some("msg-42"));
    }

    #[test]
    fn test_normalize_discord() {
        let ctx = normalize_thread_id("discord", "thread-ch-999", Some("starter-msg"));
        assert_eq!(ctx.thread_id, "thread-ch-999");
        assert_eq!(ctx.parent_message_id.as_deref(), Some("starter-msg"));
    }

    #[test]
    fn test_normalize_unknown() {
        let ctx = normalize_thread_id("matrix", "room-thread", Some("event-id"));
        assert_eq!(ctx.thread_id, "room-thread");
        assert_eq!(ctx.parent_message_id.as_deref(), Some("event-id"));
    }

    #[test]
    fn test_normalize_no_message_id() {
        let ctx = normalize_thread_id("discord", "thread-1", None);
        assert!(ctx.parent_message_id.is_none());
    }

    // === ThreadedReplyOptions serde ===

    #[test]
    fn test_reply_options_serde_round_trip() {
        let opts = ThreadedReplyOptions::in_thread_reply_to("t1", "m1").with_broadcast(true);
        let json = serde_json::to_string(&opts).expect("should serialize to JSON");
        let back: ThreadedReplyOptions =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(back.thread_id.as_deref(), Some("t1"));
        assert_eq!(back.reply_to_message_id.as_deref(), Some("m1"));
        assert!(back.broadcast);
    }

    #[test]
    fn test_reply_options_serde_minimal() {
        let json = "{}";
        let opts: ThreadedReplyOptions =
            serde_json::from_str(json).expect("should parse successfully");
        assert!(!opts.has_threading());
        assert!(!opts.broadcast);
    }
}
