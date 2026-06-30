//! Channel Message Pipeline — Unified processing for all inbound messages
//!
//! All channel adapters feed messages through this pipeline before reaching
//! the agent. The pipeline provides:
//!
//! 1. **Deduplication**: reject duplicate messages within a time window
//! 2. **Rate limiting**: per-channel and per-sender rate limits
//! 3. **Priority queuing**: urgent messages skip the queue
//! 4. **Content filtering**: spam detection, command extraction
//! 5. **Metrics**: per-channel message counts, latency tracking
//!
//! Architecture: messages flow through a series of stages, each of which
//! can pass, reject, or transform the message.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use tracing::{debug, info, warn};

// ============================================================================
// Pipeline Configuration
// ============================================================================

/// Configuration for the message pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Enable deduplication
    pub dedup_enabled: bool,
    /// Dedup window in seconds (messages with same content+sender within this window are rejected)
    pub dedup_window_secs: i64,
    /// Enable rate limiting
    pub rate_limit_enabled: bool,
    /// Max messages per sender per minute
    pub rate_limit_per_minute: u32,
    /// Max messages per channel per minute
    pub channel_rate_limit_per_minute: u32,
    /// Enable spam detection
    pub spam_filter_enabled: bool,
    /// Maximum message length (characters)
    pub max_message_length: usize,
    /// Command prefix for extracting commands (e.g. "/")
    pub command_prefix: String,
    /// Priority keywords that bump message priority
    pub priority_keywords: Vec<String>,
    /// Per-channel-type media attachment size limits.
    /// When `validate_media_size()` is called with an attachment size that exceeds
    /// the limit for its channel type, the pipeline returns a Reject verdict.
    #[serde(default)]
    pub media_limits: crate::config::MediaLimits,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            dedup_enabled: true,
            dedup_window_secs: 5,
            rate_limit_enabled: true,
            rate_limit_per_minute: 30,
            channel_rate_limit_per_minute: 100,
            spam_filter_enabled: true,
            max_message_length: 10_000,
            command_prefix: "/".into(),
            priority_keywords: vec![
                "urgent".into(),
                "critical".into(),
                "emergency".into(),
                "ASAP".into(),
            ],
            media_limits: crate::config::MediaLimits::default(),
        }
    }
}

// ============================================================================
// Pipeline Message
// ============================================================================

/// Priority level for pipeline messages
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub enum MessagePriority {
    Low,
    #[default]
    Normal,
    High,
    Urgent,
}

/// A message flowing through the pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineMessage {
    /// Unique message ID
    pub id: String,
    /// Channel type (telegram, discord, slack, etc.)
    pub channel_type: String,
    /// Channel ID
    pub channel_id: String,
    /// Sender identifier
    pub sender_id: String,
    /// Message content
    pub content: String,
    /// Timestamp when received
    pub received_at: DateTime<Utc>,
    /// Assigned priority
    pub priority: MessagePriority,
    /// Extracted command (if any)
    pub command: Option<String>,
    /// Command arguments (if command was extracted)
    pub command_args: Option<String>,
    /// Whether content was truncated
    pub truncated: bool,
    /// Pipeline processing time in microseconds
    pub processing_us: u64,
}

/// Result of pipeline processing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipelineVerdict {
    /// Message accepted, proceed to agent
    Accept,
    /// Message rejected with reason
    Reject(String),
    /// Message accepted but flagged
    Flag(String),
}

impl PipelineVerdict {
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accept | Self::Flag(_))
    }
}

// ============================================================================
// Pipeline Stages
// ============================================================================

/// Deduplication state — tracks recent messages per sender
struct DedupState {
    /// (sender_id, content_hash) → last seen timestamp
    seen: HashMap<(String, u64), DateTime<Utc>>,
    window: Duration,
}

impl DedupState {
    fn new(window_secs: i64) -> Self {
        Self {
            seen: HashMap::new(),
            window: Duration::seconds(window_secs),
        }
    }

    fn is_duplicate(&mut self, sender_id: &str, content: &str) -> bool {
        let hash = Self::hash_content(content);
        let key = (sender_id.to_string(), hash);
        let now = Utc::now();

        // Cleanup old entries
        self.seen.retain(|_, ts| now - *ts < self.window);

        if let Some(last_seen) = self.seen.get(&key)
            && now - *last_seen < self.window
        {
            return true;
        }

        self.seen.insert(key, now);
        false
    }

    fn hash_content(content: &str) -> u64 {
        // Simple FNV-1a hash
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in content.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }
}

/// Rate limiter — sliding window per sender and per channel
struct RateLimiter {
    /// sender_id → list of message timestamps
    sender_windows: HashMap<String, VecDeque<DateTime<Utc>>>,
    /// channel_id → list of message timestamps
    channel_windows: HashMap<String, VecDeque<DateTime<Utc>>>,
    sender_limit: u32,
    channel_limit: u32,
}

impl RateLimiter {
    fn new(sender_limit: u32, channel_limit: u32) -> Self {
        Self {
            sender_windows: HashMap::new(),
            channel_windows: HashMap::new(),
            sender_limit,
            channel_limit,
        }
    }

    fn check_sender(&mut self, sender_id: &str) -> bool {
        let window = self
            .sender_windows
            .entry(sender_id.to_string())
            .or_default();
        let cutoff = Utc::now() - Duration::minutes(1);
        while window.front().is_some_and(|t| *t < cutoff) {
            window.pop_front();
        }
        if window.len() >= self.sender_limit as usize {
            false
        } else {
            window.push_back(Utc::now());
            true
        }
    }

    fn check_channel(&mut self, channel_id: &str) -> bool {
        let window = self
            .channel_windows
            .entry(channel_id.to_string())
            .or_default();
        let cutoff = Utc::now() - Duration::minutes(1);
        while window.front().is_some_and(|t| *t < cutoff) {
            window.pop_front();
        }
        if window.len() >= self.channel_limit as usize {
            false
        } else {
            window.push_back(Utc::now());
            true
        }
    }
}

// ============================================================================
// Pipeline Metrics
// ============================================================================

/// Per-channel pipeline metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelMetrics {
    pub total_messages: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub flagged: u64,
    pub duplicates: u64,
    pub rate_limited: u64,
    pub commands_extracted: u64,
    pub avg_processing_us: u64,
    /// Attachments rejected because they exceeded the per-channel size limit.
    pub media_size_bytes_rejected: u64,
}

/// Aggregate pipeline metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineMetrics {
    pub channels: HashMap<String, ChannelMetrics>,
    pub total_processed: u64,
    pub total_accepted: u64,
    pub total_rejected: u64,
    pub uptime_secs: i64,
    pub timestamp: DateTime<Utc>,
}

// ============================================================================
// Message Pipeline
// ============================================================================

/// The unified message processing pipeline
pub struct MessagePipeline {
    config: PipelineConfig,
    dedup: DedupState,
    rate_limiter: RateLimiter,
    metrics: HashMap<String, ChannelMetrics>,
    started_at: DateTime<Utc>,
}

impl MessagePipeline {
    pub fn new(config: PipelineConfig) -> Self {
        let dedup = DedupState::new(config.dedup_window_secs);
        let rate_limiter = RateLimiter::new(
            config.rate_limit_per_minute,
            config.channel_rate_limit_per_minute,
        );
        Self {
            config,
            dedup,
            rate_limiter,
            metrics: HashMap::new(),
            started_at: Utc::now(),
        }
    }

    /// Process an inbound message through the pipeline.
    /// Returns the processed message and the verdict.
    pub fn process(
        &mut self,
        channel_type: &str,
        channel_id: &str,
        sender_id: &str,
        content: &str,
        message_id: &str,
    ) -> (PipelineMessage, PipelineVerdict) {
        let start = std::time::Instant::now();
        let metrics = self.metrics.entry(channel_id.to_string()).or_default();
        metrics.total_messages += 1;

        // Stage 1: Content length check
        let (processed_content, truncated) = if content.len() > self.config.max_message_length {
            (
                {
                    let mut end = self.config.max_message_length;
                    while !content.is_char_boundary(end) && end < content.len() {
                        end += 1;
                    }
                    content[..end].to_string()
                },
                true,
            )
        } else {
            (content.to_string(), false)
        };

        // Stage 2: Deduplication
        if self.config.dedup_enabled && self.dedup.is_duplicate(sender_id, &processed_content) {
            metrics.duplicates += 1;
            metrics.rejected += 1;
            let msg = self.build_message(
                message_id,
                channel_type,
                channel_id,
                sender_id,
                &processed_content,
                truncated,
                start,
            );
            debug!(
                sender = sender_id,
                channel = channel_id,
                "Duplicate message rejected"
            );
            return (msg, PipelineVerdict::Reject("duplicate message".into()));
        }

        // Stage 3: Rate limiting
        if self.config.rate_limit_enabled {
            if !self.rate_limiter.check_sender(sender_id) {
                metrics.rate_limited += 1;
                metrics.rejected += 1;
                let msg = self.build_message(
                    message_id,
                    channel_type,
                    channel_id,
                    sender_id,
                    &processed_content,
                    truncated,
                    start,
                );
                warn!(sender = sender_id, "Sender rate limit exceeded");
                return (
                    msg,
                    PipelineVerdict::Reject("sender rate limit exceeded".into()),
                );
            }
            if !self.rate_limiter.check_channel(channel_id) {
                metrics.rate_limited += 1;
                metrics.rejected += 1;
                let msg = self.build_message(
                    message_id,
                    channel_type,
                    channel_id,
                    sender_id,
                    &processed_content,
                    truncated,
                    start,
                );
                warn!(channel = channel_id, "Channel rate limit exceeded");
                return (
                    msg,
                    PipelineVerdict::Reject("channel rate limit exceeded".into()),
                );
            }
        }

        // Stage 4: Spam detection
        if self.config.spam_filter_enabled
            && let Some(reason) = detect_spam(&processed_content)
        {
            metrics.rejected += 1;
            let msg = self.build_message(
                message_id,
                channel_type,
                channel_id,
                sender_id,
                &processed_content,
                truncated,
                start,
            );
            info!(sender = sender_id, reason = %reason, "Spam detected");
            return (msg, PipelineVerdict::Flag(reason));
        }

        // Stage 5: Command extraction
        let (command, command_args) =
            extract_command(&self.config.command_prefix, &processed_content);
        if command.is_some() {
            metrics.commands_extracted += 1;
        }

        // Stage 6: Priority classification
        let priority = classify_priority(&self.config.priority_keywords, &processed_content);

        let elapsed = start.elapsed().as_micros() as u64;

        // Update metrics
        metrics.accepted += 1;
        let total = metrics.accepted + metrics.rejected;
        metrics.avg_processing_us = if total > 0 {
            ((metrics.avg_processing_us as u128 * (total - 1) as u128 + elapsed as u128)
                / total as u128) as u64
        } else {
            elapsed
        };

        let msg = PipelineMessage {
            id: message_id.to_string(),
            channel_type: channel_type.to_string(),
            channel_id: channel_id.to_string(),
            sender_id: sender_id.to_string(),
            content: processed_content,
            received_at: Utc::now(),
            priority,
            command,
            command_args,
            truncated,
            processing_us: elapsed,
        };

        (msg, PipelineVerdict::Accept)
    }

    /// Get metrics for a specific channel
    pub fn channel_metrics(&self, channel_id: &str) -> Option<&ChannelMetrics> {
        self.metrics.get(channel_id)
    }

    /// Get aggregate metrics
    pub fn metrics(&self) -> PipelineMetrics {
        let total_processed: u64 = self.metrics.values().map(|m| m.total_messages).sum();
        let total_accepted: u64 = self.metrics.values().map(|m| m.accepted).sum();
        let total_rejected: u64 = self.metrics.values().map(|m| m.rejected).sum();
        let uptime = (Utc::now() - self.started_at).num_seconds();

        PipelineMetrics {
            channels: self.metrics.clone(),
            total_processed,
            total_accepted,
            total_rejected,
            uptime_secs: uptime,
            timestamp: Utc::now(),
        }
    }

    /// Reset metrics
    pub fn reset_metrics(&mut self) {
        self.metrics.clear();
    }

    /// Validate an inbound attachment size against per-channel limits.
    ///
    /// Call this before processing any file attachment received on a channel.
    /// Returns `Ok(())` if the attachment is within the configured limit, or
    /// `Err(PipelineVerdict::Reject(...))` if it exceeds it.
    ///
    /// Also increments `media_size_bytes_rejected` on the channel metrics when rejected.
    ///
    /// # Example
    /// ```ignore
    /// if let Err(verdict) = pipeline.validate_media_size("telegram", "ch-1", 60_000_000) {
    ///     // reject the attachment
    /// }
    /// ```
    pub fn validate_media_size(
        &mut self,
        channel_type: &str,
        channel_id: &str,
        size_bytes: u64,
    ) -> Result<(), PipelineVerdict> {
        if let Err(reason) = self.config.media_limits.validate(channel_type, size_bytes) {
            let metrics = self.metrics.entry(channel_id.to_string()).or_default();
            metrics.media_size_bytes_rejected += 1;
            metrics.rejected += 1;
            warn!(
                channel_type,
                channel_id, size_bytes, "Media attachment rejected: {}", reason
            );
            Err(PipelineVerdict::Reject(reason))
        } else {
            Ok(())
        }
    }

    /// Get config
    pub fn config(&self) -> &PipelineConfig {
        &self.config
    }

    /// Update config at runtime
    pub fn set_config(&mut self, config: PipelineConfig) {
        self.dedup = DedupState::new(config.dedup_window_secs);
        self.rate_limiter = RateLimiter::new(
            config.rate_limit_per_minute,
            config.channel_rate_limit_per_minute,
        );
        self.config = config;
    }

    // -- internal helpers --

    #[allow(clippy::too_many_arguments)]
    fn build_message(
        &self,
        id: &str,
        channel_type: &str,
        channel_id: &str,
        sender_id: &str,
        content: &str,
        truncated: bool,
        start: std::time::Instant,
    ) -> PipelineMessage {
        PipelineMessage {
            id: id.to_string(),
            channel_type: channel_type.to_string(),
            channel_id: channel_id.to_string(),
            sender_id: sender_id.to_string(),
            content: content.to_string(),
            received_at: Utc::now(),
            priority: MessagePriority::Normal,
            command: None,
            command_args: None,
            truncated,
            processing_us: start.elapsed().as_micros() as u64,
        }
    }
}

/// Detect spam patterns in message content (free function to avoid borrow conflicts)
fn detect_spam(content: &str) -> Option<String> {
    if content.trim().is_empty() {
        return Some("empty message".into());
    }
    let chars: Vec<char> = content.chars().collect();
    if chars.len() > 20 {
        let mut max_repeat = 1;
        let mut current_repeat = 1;
        for i in 1..chars.len() {
            if chars[i] == chars[i - 1] {
                current_repeat += 1;
                max_repeat = max_repeat.max(current_repeat);
            } else {
                current_repeat = 1;
            }
        }
        if max_repeat >= 20 {
            return Some("excessive character repetition".into());
        }
    }
    let url_count = content.matches("http://").count() + content.matches("https://").count();
    if url_count >= 5 {
        return Some(format!("excessive URLs ({url_count})"));
    }
    None
}

/// Extract command from message content (free function to avoid borrow conflicts)
fn extract_command(prefix: &str, content: &str) -> (Option<String>, Option<String>) {
    let trimmed = content.trim();
    if !trimmed.starts_with(prefix) {
        return (None, None);
    }
    let without_prefix = &trimmed[prefix.len()..];
    let parts: Vec<&str> = without_prefix.splitn(2, char::is_whitespace).collect();
    let cmd = parts.first().map(|s| s.to_string());
    let args = parts.get(1).map(|s| s.trim().to_string());
    (cmd, args)
}

/// Classify message priority based on keywords (free function to avoid borrow conflicts)
fn classify_priority(priority_keywords: &[String], content: &str) -> MessagePriority {
    let lower = content.to_lowercase();
    for keyword in priority_keywords {
        if lower.contains(&keyword.to_lowercase()) {
            return MessagePriority::Urgent;
        }
    }
    if content.contains('!') && content.len() < 100 {
        return MessagePriority::High;
    }
    MessagePriority::Normal
}

impl Default for MessagePipeline {
    fn default() -> Self {
        Self::new(PipelineConfig::default())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pipeline() -> MessagePipeline {
        MessagePipeline::new(PipelineConfig::default())
    }

    fn process_msg(
        pipeline: &mut MessagePipeline,
        content: &str,
    ) -> (PipelineMessage, PipelineVerdict) {
        pipeline.process("telegram", "ch-1", "user-1", content, "msg-1")
    }

    #[test]
    fn test_accept_normal_message() {
        let mut pipeline = test_pipeline();
        let (msg, verdict) = process_msg(&mut pipeline, "Hello, how are you?");
        assert_eq!(verdict, PipelineVerdict::Accept);
        assert_eq!(msg.content, "Hello, how are you?");
        assert_eq!(msg.priority, MessagePriority::Normal);
    }

    #[test]
    fn test_truncate_long_message() {
        let mut pipeline = MessagePipeline::new(PipelineConfig {
            max_message_length: 10,
            ..Default::default()
        });
        let (msg, verdict) = process_msg(&mut pipeline, "This is a very long message");
        assert!(verdict.is_accepted());
        assert_eq!(msg.content.len(), 10);
        assert!(msg.truncated);
    }

    #[test]
    fn test_dedup_rejects_duplicate() {
        let mut pipeline = test_pipeline();
        let (_, v1) = pipeline.process("tg", "ch1", "u1", "hello", "m1");
        assert_eq!(v1, PipelineVerdict::Accept);
        let (_, v2) = pipeline.process("tg", "ch1", "u1", "hello", "m2");
        assert_eq!(v2, PipelineVerdict::Reject("duplicate message".into()));
    }

    #[test]
    fn test_dedup_allows_different_content() {
        let mut pipeline = test_pipeline();
        let (_, v1) = pipeline.process("tg", "ch1", "u1", "hello", "m1");
        assert_eq!(v1, PipelineVerdict::Accept);
        let (_, v2) = pipeline.process("tg", "ch1", "u1", "world", "m2");
        assert_eq!(v2, PipelineVerdict::Accept);
    }

    #[test]
    fn test_dedup_allows_different_sender() {
        let mut pipeline = test_pipeline();
        let (_, v1) = pipeline.process("tg", "ch1", "u1", "hello", "m1");
        assert_eq!(v1, PipelineVerdict::Accept);
        let (_, v2) = pipeline.process("tg", "ch1", "u2", "hello", "m2");
        assert_eq!(v2, PipelineVerdict::Accept);
    }

    #[test]
    fn test_rate_limit_sender() {
        let mut pipeline = MessagePipeline::new(PipelineConfig {
            rate_limit_per_minute: 3,
            dedup_enabled: false,
            ..Default::default()
        });
        for i in 0..3 {
            let (_, v) = pipeline.process("tg", "ch1", "u1", &format!("msg {i}"), &format!("m{i}"));
            assert!(v.is_accepted());
        }
        let (_, v) = pipeline.process("tg", "ch1", "u1", "msg 3", "m3");
        assert_eq!(
            v,
            PipelineVerdict::Reject("sender rate limit exceeded".into())
        );
    }

    #[test]
    fn test_rate_limit_channel() {
        let mut pipeline = MessagePipeline::new(PipelineConfig {
            channel_rate_limit_per_minute: 2,
            rate_limit_per_minute: 100,
            dedup_enabled: false,
            ..Default::default()
        });
        pipeline.process("tg", "ch1", "u1", "a", "m1");
        pipeline.process("tg", "ch1", "u2", "b", "m2");
        let (_, v) = pipeline.process("tg", "ch1", "u3", "c", "m3");
        assert_eq!(
            v,
            PipelineVerdict::Reject("channel rate limit exceeded".into())
        );
    }

    #[test]
    fn test_spam_empty_message() {
        let mut pipeline = test_pipeline();
        let (_, verdict) = process_msg(&mut pipeline, "   ");
        match verdict {
            PipelineVerdict::Flag(reason) => assert!(reason.contains("empty")),
            other => panic!("Expected Flag, got {:?}", other),
        }
    }

    #[test]
    fn test_spam_excessive_repetition() {
        let mut pipeline = test_pipeline();
        let spam = "a".repeat(25);
        let (_, verdict) = process_msg(&mut pipeline, &spam);
        match verdict {
            PipelineVerdict::Flag(reason) => assert!(reason.contains("repetition")),
            other => panic!("Expected Flag, got {:?}", other),
        }
    }

    #[test]
    fn test_spam_excessive_urls() {
        let mut pipeline = test_pipeline();
        let spam = "Check https://a.com https://b.com https://c.com https://d.com https://e.com";
        let (_, verdict) = process_msg(&mut pipeline, spam);
        match verdict {
            PipelineVerdict::Flag(reason) => assert!(reason.contains("URLs")),
            other => panic!("Expected Flag, got {:?}", other),
        }
    }

    #[test]
    fn test_command_extraction() {
        let mut pipeline = test_pipeline();
        let (msg, _) = process_msg(&mut pipeline, "/help me with this");
        assert_eq!(msg.command.as_deref(), Some("help"));
        assert_eq!(msg.command_args.as_deref(), Some("me with this"));
    }

    #[test]
    fn test_command_no_args() {
        let mut pipeline = test_pipeline();
        let (msg, _) = process_msg(&mut pipeline, "/status");
        assert_eq!(msg.command.as_deref(), Some("status"));
        assert!(msg.command_args.is_none());
    }

    #[test]
    fn test_no_command_prefix() {
        let mut pipeline = test_pipeline();
        let (msg, _) = process_msg(&mut pipeline, "just a regular message");
        assert!(msg.command.is_none());
    }

    #[test]
    fn test_priority_urgent_keyword() {
        let mut pipeline = test_pipeline();
        let (msg, _) = process_msg(&mut pipeline, "This is urgent please help");
        assert_eq!(msg.priority, MessagePriority::Urgent);
    }

    #[test]
    fn test_priority_high_exclamation() {
        let mut pipeline = test_pipeline();
        let (msg, _) = process_msg(&mut pipeline, "Fix this now!");
        assert_eq!(msg.priority, MessagePriority::High);
    }

    #[test]
    fn test_priority_normal() {
        let mut pipeline = test_pipeline();
        let (msg, _) = process_msg(&mut pipeline, "Can you review this PR?");
        assert_eq!(msg.priority, MessagePriority::Normal);
    }

    #[test]
    fn test_metrics_tracking() {
        let mut pipeline = test_pipeline();
        pipeline.process("tg", "ch1", "u1", "hello", "m1");
        pipeline.process("tg", "ch1", "u1", "hello", "m2"); // dup
        pipeline.process("tg", "ch2", "u2", "world", "m3");

        let metrics = pipeline.metrics();
        assert_eq!(metrics.total_processed, 3);
        assert_eq!(metrics.total_accepted, 2);
        assert_eq!(metrics.total_rejected, 1);

        let ch1 = pipeline.channel_metrics("ch1").unwrap();
        assert_eq!(ch1.duplicates, 1);
    }

    #[test]
    fn test_reset_metrics() {
        let mut pipeline = test_pipeline();
        pipeline.process("tg", "ch1", "u1", "hello", "m1");
        pipeline.reset_metrics();
        let metrics = pipeline.metrics();
        assert_eq!(metrics.total_processed, 0);
    }

    #[test]
    fn test_runtime_config_update() {
        let mut pipeline = test_pipeline();
        pipeline.set_config(PipelineConfig {
            command_prefix: "!".into(),
            ..Default::default()
        });
        let (msg, _) = pipeline.process("tg", "ch1", "u1", "!help", "m1");
        assert_eq!(msg.command.as_deref(), Some("help"));
    }

    #[test]
    fn test_dedup_disabled() {
        let mut pipeline = MessagePipeline::new(PipelineConfig {
            dedup_enabled: false,
            ..Default::default()
        });
        let (_, v1) = pipeline.process("tg", "ch1", "u1", "hello", "m1");
        let (_, v2) = pipeline.process("tg", "ch1", "u1", "hello", "m2");
        assert_eq!(v1, PipelineVerdict::Accept);
        assert_eq!(v2, PipelineVerdict::Accept);
    }

    #[test]
    fn test_verdict_is_accepted() {
        assert!(PipelineVerdict::Accept.is_accepted());
        assert!(PipelineVerdict::Flag("test".into()).is_accepted());
        assert!(!PipelineVerdict::Reject("test".into()).is_accepted());
    }

    #[test]
    fn test_processing_time_recorded() {
        let mut pipeline = test_pipeline();
        let (msg, _) = process_msg(&mut pipeline, "hello");
        // Processing should be fast (< 1ms = 1000us)
        assert!(msg.processing_us < 1_000_000);
    }

    #[test]
    fn test_default_pipeline() {
        let pipeline = MessagePipeline::default();
        assert!(pipeline.config().dedup_enabled);
        assert!(pipeline.config().rate_limit_enabled);
    }

    // ── Media size validation tests ──────────────────────────────────────────

    #[test]
    fn test_media_size_within_limit_accepted() {
        let mut pipeline = test_pipeline();
        // 10 MB < 50 MB Telegram default
        assert!(
            pipeline
                .validate_media_size("telegram", "ch-1", 10 * 1024 * 1024)
                .is_ok()
        );
    }

    #[test]
    fn test_media_size_exceeds_telegram_limit_rejected() {
        let mut pipeline = test_pipeline();
        // 60 MB > 50 MB Telegram default
        let result = pipeline.validate_media_size("telegram", "ch-1", 60 * 1024 * 1024);
        assert!(result.is_err());
        match result.unwrap_err() {
            PipelineVerdict::Reject(reason) => {
                assert!(reason.contains("telegram"));
                assert!(reason.contains("exceeds"));
            }
            other => panic!("Expected Reject, got {:?}", other),
        }
    }

    #[test]
    fn test_media_size_exceeds_discord_limit_rejected() {
        let mut pipeline = test_pipeline();
        // 30 MB > 25 MB Discord default
        let result = pipeline.validate_media_size("discord", "ch-1", 30 * 1024 * 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_media_size_slack_large_file_accepted() {
        let mut pipeline = test_pipeline();
        // 500 MB < 1 GB Slack default
        assert!(
            pipeline
                .validate_media_size("slack", "ch-1", 500 * 1024 * 1024)
                .is_ok()
        );
    }

    #[test]
    fn test_media_size_fallback_unknown_channel() {
        let mut pipeline = test_pipeline();
        // Unknown channel uses 50 MB fallback
        assert!(
            pipeline
                .validate_media_size("synology", "ch-1", 10 * 1024 * 1024)
                .is_ok()
        );
        let result = pipeline.validate_media_size("synology", "ch-1", 60 * 1024 * 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_media_size_rejection_increments_metrics() {
        let mut pipeline = test_pipeline();
        let _ = pipeline.validate_media_size("telegram", "ch-1", 60 * 1024 * 1024);
        let metrics = pipeline.channel_metrics("ch-1").unwrap();
        assert_eq!(metrics.media_size_bytes_rejected, 1);
        assert_eq!(metrics.rejected, 1);
    }

    #[test]
    fn test_media_size_config_override() {
        use crate::config::MediaLimits;
        use std::collections::HashMap;

        let mut per_channel = HashMap::new();
        per_channel.insert("telegram".into(), 5 * 1024 * 1024); // 5 MB override
        let limits = MediaLimits {
            per_channel,
            fallback_bytes: 50 * 1024 * 1024,
        };

        let mut pipeline = MessagePipeline::new(PipelineConfig {
            media_limits: limits,
            ..Default::default()
        });

        // 4 MB < 5 MB override → ok
        assert!(
            pipeline
                .validate_media_size("telegram", "ch-1", 4 * 1024 * 1024)
                .is_ok()
        );
        // 6 MB > 5 MB override → rejected
        assert!(
            pipeline
                .validate_media_size("telegram", "ch-1", 6 * 1024 * 1024)
                .is_err()
        );
    }

    #[test]
    fn test_media_limits_default_channels() {
        use crate::config::MediaLimits;
        let limits = MediaLimits::default();
        assert_eq!(limits.limit_for("telegram"), 50 * 1024 * 1024);
        assert_eq!(limits.limit_for("discord"), 25 * 1024 * 1024);
        assert_eq!(limits.limit_for("slack"), 1024 * 1024 * 1024);
        assert_eq!(limits.limit_for("email"), 25 * 1024 * 1024);
        assert_eq!(limits.limit_for("whatsapp"), 100 * 1024 * 1024);
        assert_eq!(limits.limit_for("unknown_channel"), 50 * 1024 * 1024); // fallback
    }

    #[test]
    fn test_media_limits_validate_ok() {
        use crate::config::MediaLimits;
        let limits = MediaLimits::default();
        assert!(limits.validate("telegram", 1024).is_ok());
    }

    #[test]
    fn test_media_limits_validate_err() {
        use crate::config::MediaLimits;
        let limits = MediaLimits::default();
        let err = limits.validate("discord", 30 * 1024 * 1024).unwrap_err();
        assert!(err.contains("discord"));
        assert!(err.contains("exceeds"));
    }
}
