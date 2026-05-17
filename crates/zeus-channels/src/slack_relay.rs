//! Slack Relay — Socket Mode WebSocket relay with tmux forwarding
//!
//! Similar to TelegramRelay: connects to Slack via Socket Mode,
//! receives inbound messages, and forwards them to the active tmux
//! session for Claude Code to process. Also queues messages for
//! MCP tool consumption via `slack_get_messages`.
//!
//! # Config
//! ```toml
//! [slack_relay]
//! bot_token = "xoxb-..."
//! app_token = "xapp-..."
//! channel_ids = "C01234,C56789"          # optional: limit to specific channels
//! allowed_users = "U01234,U56789"        # optional: limit to specific users
//! require_mention_in_channels = true     # require @mention in channels (DMs always accepted)
//! target_session = "z-0"                 # optional: explicit tmux session
//! ```

use crate::tmux_forward::{forward_to_tmux, resolve_tmux_target};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, Notify, RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tracing::{debug, info, warn};
use zeus_core::Result;

/// Type alias for agent message callback — mirrors TelegramRelay pattern
type MessageCallback =
    Arc<RwLock<Option<Box<dyn Fn(String) -> tokio::task::JoinHandle<String> + Send + Sync>>>>;

// ============================================================================
// Config
// ============================================================================

/// Slack relay configuration (mirrors TelegramRelayConfig pattern)
#[derive(Debug, Clone)]
pub struct SlackRelayConfig {
    pub bot_token: String,
    pub app_token: String,
    pub channel_ids: Vec<String>,
    pub allowed_users: Vec<String>,
    pub require_mention_in_channels: bool,
    pub target_session: Option<String>,
    pub max_queue: usize,
    pub rate_limit_per_minute: u32,
}

impl Default for SlackRelayConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            app_token: String::new(),
            channel_ids: Vec::new(),
            allowed_users: Vec::new(),
            require_mention_in_channels: true,
            target_session: None,
            max_queue: 100,
            rate_limit_per_minute: 30,
        }
    }
}

// ============================================================================
// Incoming message type
// ============================================================================

/// An incoming Slack message queued for MCP tool consumption
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackIncoming {
    pub user_id: String,
    pub user_name: String,
    pub channel_id: String,
    pub channel_name: String,
    pub text: String,
    pub thread_ts: Option<String>,
    pub is_dm: bool,
    pub timestamp: DateTime<Utc>,
}

// ============================================================================
// Socket Mode envelope types
// ============================================================================

#[derive(Debug, Deserialize)]
struct SocketModeEnvelope {
    envelope_id: String,
    #[serde(rename = "type")]
    event_type: String,
    payload: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct SocketModeAck {
    envelope_id: String,
}

#[derive(Debug, Deserialize)]
struct EventPayload {
    event: Option<SlackEvent>,
}

#[derive(Debug, Deserialize)]
struct SlackEvent {
    #[serde(rename = "type")]
    event_type: String,
    user: Option<String>,
    text: Option<String>,
    channel: Option<String>,
    channel_type: Option<String>,
    thread_ts: Option<String>,
    bot_id: Option<String>,
}

// ============================================================================
// Slack Relay
// ============================================================================

pub struct SlackRelay {
    config: SlackRelayConfig,
    client: reqwest::Client,
    messages: Arc<Mutex<VecDeque<SlackIncoming>>>,
    running: Arc<AtomicBool>,
    poll_handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    target_session: Arc<RwLock<Option<String>>>,
    bot_user_id: Arc<RwLock<Option<String>>>,
    shutdown: Arc<Notify>,
    message_callback: MessageCallback,
}

impl SlackRelay {
    /// Create a new Slack relay
    pub fn new(config: SlackRelayConfig) -> Self {
        // config.toml is sole source of truth — no env var fallback.
        let target_session = config.target_session.clone();

        if let Some(ref session) = target_session {
            info!("Slack relay targeting tmux session: {}", session);
        }

        Self {
            config,
            client: reqwest::Client::new(),
            messages: Arc::new(Mutex::new(VecDeque::new())),
            running: Arc::new(AtomicBool::new(false)),
            poll_handle: tokio::sync::Mutex::new(None),
            target_session: Arc::new(RwLock::new(target_session)),
            bot_user_id: Arc::new(RwLock::new(None)),
            shutdown: Arc::new(Notify::new()),
            message_callback: Arc::new(RwLock::new(None)),
        }
    }

    /// Register an agent inbox callback for inbound Slack messages.
    ///
    /// When set, every qualifying message routes to the callback (agent inbox)
    /// AND continues forwarding to tmux — both paths run independently,
    /// identical to the TelegramRelay pattern from `e67da8d9`.
    pub async fn set_message_callback<F>(&self, callback: F)
    where
        F: Fn(String) -> tokio::task::JoinHandle<String> + Send + Sync + 'static,
    {
        let mut cb = self.message_callback.write().await;
        *cb = Some(Box::new(callback));
    }

    /// Start the relay (Socket Mode WebSocket connection)
    pub async fn start(&self) -> Result<()> {
        if self.config.bot_token.is_empty() || self.config.app_token.is_empty() {
            return Err(zeus_core::Error::Config(
                "Slack relay requires both bot_token and app_token".into(),
            ));
        }

        // Fetch bot user ID for mention detection
        match self.fetch_bot_user_id().await {
            Ok(id) => {
                info!("Slack relay bot user ID: {}", id);
                *self.bot_user_id.write().await = Some(id);
            }
            Err(e) => {
                warn!(
                    "Could not fetch Slack bot user ID: {} — mention detection disabled",
                    e
                );
            }
        }

        self.running.store(true, Ordering::SeqCst);

        // Spawn the Socket Mode loop with auto-reconnect
        let config = self.config.clone();
        let client = self.client.clone();
        let messages = self.messages.clone();
        let running = self.running.clone();
        let target_session = self.target_session.clone();
        let bot_user_id = self.bot_user_id.clone();
        let shutdown = self.shutdown.clone();
        let message_callback = self.message_callback.clone();

        let handle = tokio::spawn(async move {
            let mut backoff_ms = 1000u64;

            while running.load(Ordering::SeqCst) {
                match Self::run_socket_mode(
                    &config,
                    &client,
                    &messages,
                    &running,
                    &target_session,
                    &bot_user_id,
                    &shutdown,
                    &message_callback,
                )
                .await
                {
                    Ok(()) => {
                        info!("Slack Socket Mode loop exited cleanly");
                        break;
                    }
                    Err(e) => {
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }
                        warn!(
                            "Slack Socket Mode error: {} — reconnecting in {}ms",
                            e, backoff_ms
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                        backoff_ms = (backoff_ms * 2).min(30_000);
                    }
                }
            }
        });

        *self.poll_handle.lock().await = Some(handle);
        info!("Slack relay started (Socket Mode)");
        Ok(())
    }

    /// Stop the relay
    pub async fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.shutdown.notify_one();
        if let Some(handle) = self.poll_handle.lock().await.take() {
            handle.abort();
        }
        info!("Slack relay stopped");
    }

    /// Get queued incoming messages (for MCP tool consumption)
    pub async fn get_messages(&self, limit: usize) -> Vec<SlackIncoming> {
        let mut queue = self.messages.lock().await;
        let count = limit.min(queue.len());
        queue.drain(..count).collect()
    }

    /// Send a message via Slack Web API
    pub async fn send_message(&self, channel: &str, text: &str) -> Result<()> {
        let url = "https://slack.com/api/chat.postMessage";
        let resp = self
            .client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .json(&serde_json::json!({
                "channel": channel,
                "text": text,
            }))
            .send()
            .await
            .map_err(|e| zeus_core::Error::Channel(format!("Slack send failed: {}", e)))?;

        let body: serde_json::Value = resp.json().await.map_err(|e| {
            zeus_core::Error::Channel(format!("Slack response parse failed: {}", e))
        })?;

        if body.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let error = body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(zeus_core::Error::Channel(format!(
                "Slack API error: {}",
                error
            )));
        }

        Ok(())
    }

    /// Get relay status
    pub async fn status(&self) -> String {
        let mut status = String::new();
        status.push_str(&format!(
            "Slack relay: {}\n",
            if self.running.load(Ordering::SeqCst) {
                "running"
            } else {
                "stopped"
            }
        ));

        let queue_len = self.messages.lock().await.len();
        status.push_str(&format!("Queued messages: {}\n", queue_len));

        let explicit = self.target_session.read().await.clone();
        if let Some(ref target) = explicit {
            status.push_str(&format!("Target session: {} (explicit)\n", target));
        } else if let Some(detected) = crate::tmux_forward::detect_active_tmux_session().await {
            status.push_str(&format!("Target session: {} (auto-detected)\n", detected));
        } else {
            status.push_str("Target session: none (no tmux sessions found)\n");
        }

        if let Some(ref bot_id) = *self.bot_user_id.read().await {
            status.push_str(&format!("Bot user ID: {}\n", bot_id));
        }

        if !self.config.channel_ids.is_empty() {
            status.push_str(&format!(
                "Listening on channels: {}\n",
                self.config.channel_ids.join(", ")
            ));
        } else {
            status.push_str("Listening on: all channels\n");
        }

        status
    }

    // ========================================================================
    // Thread history
    // ========================================================================

    /// Fetch thread replies from Slack's conversations.replies API.
    ///
    /// Returns a vec of (user_id, text, timestamp) for messages in the thread.
    /// Limited to the most recent `limit` messages (default 20).
    pub async fn get_thread_replies(
        &self,
        channel: &str,
        thread_ts: &str,
        limit: Option<usize>,
    ) -> Result<Vec<(String, String, String)>> {
        let limit = limit.unwrap_or(20);
        let resp = self
            .client
            .get("https://slack.com/api/conversations.replies")
            .bearer_auth(&self.config.bot_token)
            .query(&[
                ("channel", channel),
                ("ts", thread_ts),
                ("limit", &limit.to_string()),
            ])
            .send()
            .await
            .map_err(|e| {
                zeus_core::Error::channel(format!("conversations.replies failed: {}", e))
            })?;

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| zeus_core::Error::channel(format!("Failed to parse replies: {}", e)))?;

        if body["ok"].as_bool() != Some(true) {
            let err = body["error"].as_str().unwrap_or("unknown");
            return Err(zeus_core::Error::channel(format!(
                "conversations.replies error: {}",
                err
            )));
        }

        let messages = body["messages"]
            .as_array()
            .map(|msgs| {
                msgs.iter()
                    .filter_map(|m| {
                        let user = m["user"].as_str().unwrap_or("unknown").to_string();
                        let text = m["text"].as_str().unwrap_or("").to_string();
                        let ts = m["ts"].as_str().unwrap_or("").to_string();
                        if text.is_empty() {
                            None
                        } else {
                            Some((user, text, ts))
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(messages)
    }

    /// Build a thread context string from thread replies.
    ///
    /// Returns a formatted block like:
    /// ```text
    /// [Thread context - 5 messages]
    /// user1: How do we deploy to production?
    /// user2: Use the deploy script
    /// ...
    /// ```
    pub async fn fetch_thread_context(
        &self,
        channel: &str,
        thread_ts: &str,
        max_messages: Option<usize>,
    ) -> Option<String> {
        match self
            .get_thread_replies(channel, thread_ts, max_messages)
            .await
        {
            Ok(replies) if replies.len() > 1 => {
                // Skip the last message (it's the one we're replying to)
                let history: Vec<String> = replies[..replies.len() - 1]
                    .iter()
                    .map(|(user, text, _)| format!("{}: {}", user, text))
                    .collect();

                if history.is_empty() {
                    return None;
                }

                Some(format!(
                    "[Thread context — {} prior messages]\n{}",
                    history.len(),
                    history.join("\n")
                ))
            }
            Ok(_) => None, // Only 1 message = thread root, no context needed
            Err(e) => {
                debug!("Failed to fetch thread context: {}", e);
                None
            }
        }
    }

    /// Static version of fetch_thread_context for use in handle_envelope
    /// (which doesn't have &self access).
    async fn fetch_thread_context_static(
        client: &reqwest::Client,
        bot_token: &str,
        channel: &str,
        thread_ts: &str,
        max_messages: Option<usize>,
    ) -> Option<String> {
        let limit = max_messages.unwrap_or(20);
        let resp = client
            .get("https://slack.com/api/conversations.replies")
            .bearer_auth(bot_token)
            .query(&[
                ("channel", channel),
                ("ts", thread_ts),
                ("limit", &limit.to_string()),
            ])
            .send()
            .await
            .ok()?;

        let body: serde_json::Value = resp.json().await.ok()?;

        if body["ok"].as_bool() != Some(true) {
            debug!(
                "conversations.replies error: {}",
                body["error"].as_str().unwrap_or("unknown")
            );
            return None;
        }

        let messages: Vec<String> = body["messages"]
            .as_array()?
            .iter()
            .filter_map(|m| {
                let user = m["user"].as_str().unwrap_or("unknown");
                let text = m["text"].as_str()?;
                if text.is_empty() {
                    None
                } else {
                    Some(format!("{}: {}", user, text))
                }
            })
            .collect();

        // Skip if only 1 message (the thread root we're replying to)
        if messages.len() <= 1 {
            return None;
        }

        // Exclude the last message (the one that triggered this handler)
        let history = &messages[..messages.len() - 1];
        Some(format!(
            "[Thread context — {} prior messages]\n{}",
            history.len(),
            history.join("\n")
        ))
    }

    // ========================================================================
    // Internal
    // ========================================================================

    /// Fetch bot user ID via auth.test
    async fn fetch_bot_user_id(&self) -> Result<String> {
        let resp = self
            .client
            .post("https://slack.com/api/auth.test")
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .send()
            .await
            .map_err(|e| zeus_core::Error::Channel(format!("auth.test failed: {}", e)))?;

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| zeus_core::Error::Channel(format!("auth.test parse failed: {}", e)))?;

        body.get("user_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| zeus_core::Error::Channel("No user_id in auth.test".into()))
    }

    /// Get a Socket Mode WebSocket URL
    async fn get_ws_url(client: &reqwest::Client, app_token: &str) -> Result<String> {
        let resp = client
            .post("https://slack.com/api/apps.connections.open")
            .header("Authorization", format!("Bearer {}", app_token))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send()
            .await
            .map_err(|e| {
                zeus_core::Error::Channel(format!("Socket Mode connection failed: {}", e))
            })?;

        let body: serde_json::Value = resp.json().await.map_err(|e| {
            zeus_core::Error::Channel(format!("Socket Mode response parse failed: {}", e))
        })?;

        if body.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let error = body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(zeus_core::Error::Channel(format!(
                "Socket Mode open failed: {}",
                error
            )));
        }

        body.get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| zeus_core::Error::Channel("No URL in Socket Mode response".into()))
    }

    /// Run the Socket Mode WebSocket loop (single connection)
    async fn run_socket_mode(
        config: &SlackRelayConfig,
        client: &reqwest::Client,
        messages: &Arc<Mutex<VecDeque<SlackIncoming>>>,
        running: &Arc<AtomicBool>,
        target_session: &Arc<RwLock<Option<String>>>,
        bot_user_id: &Arc<RwLock<Option<String>>>,
        shutdown: &Arc<Notify>,
        message_callback: &MessageCallback,
    ) -> Result<()> {
        let ws_url = Self::get_ws_url(client, &config.app_token).await?;
        info!("Connecting to Slack Socket Mode WebSocket");

        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .map_err(|e| zeus_core::Error::Channel(format!("WebSocket connect failed: {}", e)))?;

        let (mut write, mut read) = ws_stream.split();
        info!("Slack Socket Mode connected");

        loop {
            tokio::select! {
                _ = shutdown.notified() => {
                    info!("Slack relay shutdown signal received");
                    return Ok(());
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(WsMessage::Text(text))) => {
                            if let Err(e) = Self::handle_envelope(
                                &text,
                                &mut write,
                                config,
                                messages,
                                target_session,
                                bot_user_id,
                                client,
                                message_callback,
                            ).await {
                                warn!("Error handling Slack envelope: {}", e);
                            }
                        }
                        Some(Ok(WsMessage::Ping(data))) => {
                            let _ = write.send(WsMessage::Pong(data)).await;
                        }
                        Some(Ok(WsMessage::Close(_))) => {
                            info!("Slack Socket Mode connection closed by server");
                            return Err(zeus_core::Error::Channel("Connection closed".into()));
                        }
                        Some(Err(e)) => {
                            return Err(zeus_core::Error::Channel(format!("WebSocket error: {}", e)));
                        }
                        None => {
                            return Err(zeus_core::Error::Channel("WebSocket stream ended".into()));
                        }
                        _ => {}
                    }
                }
            }

            if !running.load(Ordering::SeqCst) {
                return Ok(());
            }
        }
    }

    /// Handle a Socket Mode envelope
    async fn handle_envelope<S>(
        text: &str,
        write: &mut S,
        config: &SlackRelayConfig,
        messages: &Arc<Mutex<VecDeque<SlackIncoming>>>,
        target_session: &Arc<RwLock<Option<String>>>,
        bot_user_id: &Arc<RwLock<Option<String>>>,
        http_client: &reqwest::Client,
        message_callback: &MessageCallback,
    ) -> Result<()>
    where
        S: futures_util::Sink<WsMessage> + Unpin,
        S::Error: std::fmt::Display,
    {
        let envelope: SocketModeEnvelope = serde_json::from_str(text)
            .map_err(|e| zeus_core::Error::Channel(format!("Envelope parse failed: {}", e)))?;

        // Always acknowledge
        let ack = SocketModeAck {
            envelope_id: envelope.envelope_id.clone(),
        };
        let ack_json = serde_json::to_string(&ack)
            .map_err(|e| zeus_core::Error::Channel(format!("Ack serialize failed: {}", e)))?;
        write
            .send(WsMessage::Text(ack_json))
            .await
            .map_err(|e| zeus_core::Error::Channel(format!("Ack send failed: {}", e)))?;

        // Only handle events_api for now (messages)
        if envelope.event_type != "events_api" {
            debug!("Slack relay ignoring event type: {}", envelope.event_type);
            return Ok(());
        }

        let payload = match envelope.payload {
            Some(p) => p,
            None => return Ok(()),
        };

        let event_payload: EventPayload = serde_json::from_value(payload)
            .map_err(|e| zeus_core::Error::Channel(format!("Event parse failed: {}", e)))?;

        let event = match event_payload.event {
            Some(e) if e.event_type == "message" => e,
            _ => return Ok(()),
        };

        // Skip bot messages
        if event.bot_id.is_some() {
            return Ok(());
        }

        // Skip our own messages
        let bot_id = bot_user_id.read().await.clone();
        if let (Some(user), Some(bid)) = (&event.user, &bot_id)
            && user == bid
        {
            return Ok(());
        }

        let text = match event.text {
            Some(t) => t,
            None => return Ok(()),
        };
        let channel = match event.channel {
            Some(c) => c,
            None => return Ok(()),
        };
        let user = match event.user {
            Some(u) => u,
            None => return Ok(()),
        };

        // Channel filter
        if !config.channel_ids.is_empty() && !config.channel_ids.contains(&channel) {
            debug!(
                "Slack relay: ignoring message from non-monitored channel {}",
                channel
            );
            return Ok(());
        }

        // User filter
        if !config.allowed_users.is_empty() && !config.allowed_users.contains(&user) {
            debug!(
                "Slack relay: ignoring message from non-allowed user {}",
                user
            );
            return Ok(());
        }

        // DM detection: Slack DM channel IDs start with 'D'
        let is_dm = event
            .channel_type
            .as_deref()
            .map(|ct| ct == "im")
            .unwrap_or_else(|| channel.starts_with('D'));

        // Mention check for channels (DMs always pass)
        if !is_dm && config.require_mention_in_channels {
            let is_mention = bot_id
                .as_ref()
                .map(|bid| text.contains(&format!("<@{}>", bid)))
                .unwrap_or(false);
            if !is_mention {
                debug!("Slack relay: ignoring non-mention in channel {}", channel);
                return Ok(());
            }
        }

        // Strip the bot mention from the text for cleaner forwarding
        let clean_text = if let Some(ref bid) = bot_id {
            text.replace(&format!("<@{}>", bid), "").trim().to_string()
        } else {
            text.clone()
        };

        info!(
            "Slack relay: message from {} in {}: {}",
            user,
            channel,
            if clean_text.len() > 50 {
                zeus_core::truncate_str(&clean_text, 50)
            } else {
                &clean_text
            }
        );

        // Queue for MCP tool consumption
        let incoming = SlackIncoming {
            user_id: user.clone(),
            user_name: user.clone(), // Would need users.info API for display name
            channel_id: channel.clone(),
            channel_name: channel.clone(),
            text: clean_text.clone(),
            thread_ts: event.thread_ts.clone(),
            is_dm,
            timestamp: Utc::now(),
        };

        {
            let mut queue = messages.lock().await;
            if queue.len() >= config.max_queue {
                queue.pop_front();
            }
            queue.push_back(incoming);
        }

        // Fetch thread context if this message is part of a thread
        let thread_context = if let Some(ref ts) = event.thread_ts {
            Self::fetch_thread_context_static(
                http_client,
                &config.bot_token,
                &channel,
                ts,
                Some(20),
            )
            .await
        } else {
            None
        };

        // Build the forwarding text (used by both tmux and agent callback)
        let fwd_text = if let Some(ref ctx) = thread_context {
            format!(
                "(Slack from {} in {} [thread]):\n{}\n---\n{}",
                user, channel, ctx, clean_text
            )
        } else {
            format!("(Slack from {} in {}): {}", user, channel, clean_text)
        };

        // Fire agent inbox callback if registered (mirrors Telegram pattern)
        if let Some(ref cb) = *message_callback.read().await {
            cb(fwd_text.clone());
        }

        // Forward to tmux (runs independently — both paths execute)
        let explicit_target = target_session.read().await.clone();
        if let Some(session) = resolve_tmux_target(&explicit_target).await {
            forward_to_tmux(&session, &fwd_text).await;
        } else {
            debug!("Slack relay: no tmux session available for forwarding");
        }

        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slack_relay_config_default() {
        let config = SlackRelayConfig::default();
        assert!(config.bot_token.is_empty());
        assert!(config.app_token.is_empty());
        assert!(config.channel_ids.is_empty());
        assert!(config.allowed_users.is_empty());
        assert!(config.require_mention_in_channels);
        assert_eq!(config.max_queue, 100);
    }

    #[test]
    fn test_slack_relay_creation() {
        let config = SlackRelayConfig {
            bot_token: "xoxb-test".to_string(),
            app_token: "xapp-test".to_string(),
            ..Default::default()
        };
        let relay = SlackRelay::new(config);
        assert!(!relay.running.load(Ordering::SeqCst));
    }

    #[test]
    fn test_slack_incoming_serialization() {
        let msg = SlackIncoming {
            user_id: "U1234".to_string(),
            user_name: "testuser".to_string(),
            channel_id: "C5678".to_string(),
            channel_name: "general".to_string(),
            text: "hello Zeus".to_string(),
            thread_ts: None,
            is_dm: false,
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("U1234"));
        assert!(json.contains("hello Zeus"));
    }

    #[test]
    fn test_channel_id_filtering() {
        let config = SlackRelayConfig {
            channel_ids: vec!["C001".to_string(), "C002".to_string()],
            ..Default::default()
        };
        assert!(config.channel_ids.contains(&"C001".to_string()));
        assert!(!config.channel_ids.contains(&"C999".to_string()));
    }

    #[test]
    fn test_user_filtering() {
        let config = SlackRelayConfig {
            allowed_users: vec!["U001".to_string()],
            ..Default::default()
        };
        assert!(config.allowed_users.contains(&"U001".to_string()));
        assert!(!config.allowed_users.contains(&"U999".to_string()));
    }

    #[tokio::test]
    async fn test_message_queue() {
        let config = SlackRelayConfig {
            bot_token: "xoxb-test".to_string(),
            app_token: "xapp-test".to_string(),
            max_queue: 3,
            ..Default::default()
        };
        let relay = SlackRelay::new(config);

        // Add messages to queue
        {
            let mut queue = relay.messages.lock().await;
            for i in 0..5 {
                queue.push_back(SlackIncoming {
                    user_id: format!("U{}", i),
                    user_name: format!("user{}", i),
                    channel_id: "C001".to_string(),
                    channel_name: "test".to_string(),
                    text: format!("msg {}", i),
                    thread_ts: None,
                    is_dm: false,
                    timestamp: Utc::now(),
                });
            }
        }

        let msgs = relay.get_messages(2).await;
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text, "msg 0");
        assert_eq!(msgs[1].text, "msg 1");

        // Should have 3 remaining
        let remaining = relay.get_messages(10).await;
        assert_eq!(remaining.len(), 3);
    }

    #[tokio::test]
    async fn test_start_requires_tokens() {
        let relay = SlackRelay::new(SlackRelayConfig::default());
        assert!(relay.start().await.is_err());
    }

    #[tokio::test]
    async fn test_status_output() {
        let config = SlackRelayConfig {
            bot_token: "xoxb-test".to_string(),
            app_token: "xapp-test".to_string(),
            channel_ids: vec!["C001".to_string()],
            ..Default::default()
        };
        let relay = SlackRelay::new(config);
        let status = relay.status().await;
        assert!(status.contains("Slack relay: stopped"));
        assert!(status.contains("C001"));
    }
}
