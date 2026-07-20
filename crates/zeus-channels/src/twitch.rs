//! Twitch channel adapter
//!
//! Provides Twitch chat messaging support via IRC and Helix API.
//! Connects to Twitch chat channels and supports sending/receiving messages.

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{Notify, RwLock, mpsc};
use zeus_core::{Error, Result};

const TWITCH_IRC_HOST: &str = "irc.chat.twitch.tv";
const TWITCH_IRC_PORT: u16 = 6667;

/// Twitch channel adapter
pub struct TwitchAdapter {
    connected: Arc<AtomicBool>,
    config: TwitchConfig,
    shutdown: Arc<Notify>,
    /// Handle to the receive task
    task_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
    /// Writer for sending messages
    writer: Arc<RwLock<Option<tokio::net::tcp::OwnedWriteHalf>>>,
}

impl TwitchAdapter {
    /// Create a new Twitch adapter
    pub async fn new(config: TwitchConfig) -> Result<Self> {
        if config.oauth_token.is_empty() {
            return Err(Error::Config("Twitch oauth_token is required".into()));
        }
        if config.username.is_empty() {
            return Err(Error::Config("Twitch username is required".into()));
        }

        tracing::info!(username = %config.username, "Twitch adapter created");

        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            config,
            shutdown: Arc::new(Notify::new()),
            task_handle: RwLock::new(None),
            writer: Arc::new(RwLock::new(None)),
        })
    }

    /// Connect to Twitch IRC and start receiving messages
    async fn connect(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        let stream = TcpStream::connect((TWITCH_IRC_HOST, TWITCH_IRC_PORT))
            .await
            .map_err(|e| Error::Channel(format!("Failed to connect to Twitch: {}", e)))?;

        let (reader, mut writer) = stream.into_split();

        // Authenticate
        let pass_cmd = format!("PASS oauth:{}\r\n", self.config.oauth_token);
        let nick_cmd = format!("NICK {}\r\n", self.config.username);

        writer
            .write_all(pass_cmd.as_bytes())
            .await
            .map_err(|e| Error::Channel(format!("Failed to send PASS: {}", e)))?;
        writer
            .write_all(nick_cmd.as_bytes())
            .await
            .map_err(|e| Error::Channel(format!("Failed to send NICK: {}", e)))?;

        // Request capabilities for tags and commands
        writer
            .write_all(b"CAP REQ :twitch.tv/tags twitch.tv/commands\r\n")
            .await
            .map_err(|e| Error::Channel(format!("Failed to request capabilities: {}", e)))?;

        // Join configured channels
        for channel in &self.config.channels {
            let join_cmd = format!("JOIN #{}\r\n", channel.trim_start_matches('#'));
            writer
                .write_all(join_cmd.as_bytes())
                .await
                .map_err(|e| Error::Channel(format!("Failed to join channel: {}", e)))?;
            tracing::info!(channel = %channel, "Joined Twitch channel");
        }

        *self.writer.write().await = Some(writer);

        let connected = self.connected.clone();
        let shutdown = self.shutdown.clone();
        let username = self.config.username.clone();
        let writer_clone = self.writer.clone();

        let handle = tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut line = String::new();

            loop {
                line.clear();
                tokio::select! {
                    _ = shutdown.notified() => {
                        tracing::info!("Twitch IRC shutting down");
                        break;
                    }
                    result = reader.read_line(&mut line) => {
                        match result {
                            Ok(0) => {
                                tracing::info!("Twitch IRC connection closed");
                                break;
                            }
                            Ok(_) => {
                                if let Err(e) = Self::handle_irc_message(
                                    &line,
                                    &tx,
                                    &username,
                                    &writer_clone,
                                ).await {
                                    tracing::error!(error = %e, "Error handling IRC message");
                                }
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "Error reading from IRC");
                                break;
                            }
                        }
                    }
                }
            }
            connected.store(false, Ordering::SeqCst);
        });

        *self.task_handle.write().await = Some(handle);
        self.connected.store(true, Ordering::SeqCst);
        tracing::info!("Twitch IRC connected");

        Ok(())
    }

    /// Handle an IRC message
    async fn handle_irc_message(
        line: &str,
        tx: &mpsc::Sender<ChannelMessage>,
        my_username: &str,
        writer: &Arc<RwLock<Option<tokio::net::tcp::OwnedWriteHalf>>>,
    ) -> Result<()> {
        let line = line.trim();

        // Handle PING
        if line.starts_with("PING") {
            // #427: `line[5..]` panics if the server sends a bare "PING" with
            // no trailing " <token>" (line.len() < 6). Use get() to stay safe
            // against malformed/adversarial IRC input from the network.
            let ping_arg = line.get(5..).unwrap_or("");
            if let Some(w) = writer.write().await.as_mut() {
                let pong = format!("PONG {}\r\n", ping_arg);
                let _ = w.write_all(pong.as_bytes()).await;
            }
            return Ok(());
        }

        // Parse PRIVMSG
        // Format: @tags :user!user@user.tmi.twitch.tv PRIVMSG #channel :message
        if let Some(privmsg_pos) = line.find("PRIVMSG") {
            // Extract username. #427: if `!` appears before `:` (or is absent
            // while `:` is present later), `user_end` can land before
            // `user_start`, which panics on `&line[user_start..user_end]`.
            // Guard with get() and skip malformed lines instead of crashing.
            let user_start = line.find(':').unwrap_or(0) + 1;
            let user_end = line.find('!').unwrap_or(user_start);
            let username = match line.get(user_start..user_end) {
                Some(u) => u,
                None => return Ok(()), // malformed IRC line, ignore
            };

            // Skip our own messages
            if username.eq_ignore_ascii_case(my_username) {
                return Ok(());
            }

            // Extract channel. #427: `channel_start` can exceed `line.len()`
            // when "PRIVMSG" appears at/near the end of the line with no
            // trailing space or channel name, which panics on a raw slice.
            // Guard with get() and skip malformed lines instead of crashing.
            let channel_start = privmsg_pos + 8;
            let after_privmsg = match line.get(channel_start..) {
                Some(s) => s,
                None => return Ok(()), // malformed IRC line, ignore
            };
            let channel_end = after_privmsg
                .find(' ')
                .map(|p| channel_start + p)
                .unwrap_or(line.len());
            let channel = line
                .get(channel_start..channel_end)
                .unwrap_or("")
                .trim_start_matches('#');

            // Extract message
            if let Some(msg_start) = line[channel_end..].find(':') {
                let message = &line[channel_end + msg_start + 1..];

                let source = ChannelSource::with_chat("twitch", username, channel);
                let msg = ChannelMessage::new(source, message.to_string());

                tx.send(msg)
                    .await
                    .map_err(|e| Error::Channel(format!("Failed to forward message: {}", e)))?;
            }
        }

        Ok(())
    }

    /// Send a message to a channel
    pub async fn send_message(&self, channel: &str, text: &str) -> Result<()> {
        let mut writer_guard = self.writer.write().await;
        let writer = writer_guard
            .as_mut()
            .ok_or_else(|| Error::Channel("Twitch not connected".into()))?;

        let msg = format!("PRIVMSG #{} :{}\r\n", channel.trim_start_matches('#'), text);
        writer
            .write_all(msg.as_bytes())
            .await
            .map_err(|e| Error::Channel(format!("Failed to send message: {}", e)))?;

        tracing::info!(channel = %channel, "Twitch message sent");
        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for TwitchAdapter {
    fn channel_type(&self) -> &'static str {
        "twitch"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Native
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        self.connect(tx).await
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.shutdown.notify_one();

        // Send QUIT command
        if let Some(mut writer) = self.writer.write().await.take() {
            let _ = writer.write_all(b"QUIT\r\n").await;
        }

        if let Some(handle) = self.task_handle.write().await.take() {
            let _ = handle.await;
        }

        tracing::info!("Twitch adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "twitch" {
            return Err(Error::channel("Invalid channel source for Twitch"));
        }

        let channel = to
            .chat_id
            .as_deref()
            .ok_or_else(|| Error::channel("Twitch send requires a channel name"))?;

        self.send_message(channel, content).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

/// Twitch configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TwitchConfig {
    /// OAuth token (without the 'oauth:' prefix)
    #[serde(default)]
    pub oauth_token: String,
    /// Bot username
    #[serde(default)]
    pub username: String,
    /// Channels to join (without '#' prefix)
    #[serde(default)]
    pub channels: Vec<String>,
    /// Client ID (for Helix API)
    #[serde(default)]
    pub client_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_twitch_config_default() {
        let config = TwitchConfig::default();
        assert!(config.oauth_token.is_empty());
        assert!(config.username.is_empty());
        assert!(config.channels.is_empty());
    }

    #[tokio::test]
    async fn test_twitch_adapter_validation() {
        // Empty config should fail
        let config = TwitchConfig::default();
        assert!(TwitchAdapter::new(config).await.is_err());

        // Missing username should fail
        let config = TwitchConfig {
            oauth_token: "test-token".to_string(),
            ..Default::default()
        };
        assert!(TwitchAdapter::new(config).await.is_err());

        // Valid config should succeed
        let config = TwitchConfig {
            oauth_token: "test-token".to_string(),
            username: "testbot".to_string(),
            channels: vec!["testchannel".to_string()],
            ..Default::default()
        };
        assert!(TwitchAdapter::new(config).await.is_ok());
    }

    #[tokio::test]
    async fn test_twitch_adapter_lifecycle() {
        let config = TwitchConfig {
            oauth_token: "test-token".to_string(),
            username: "testbot".to_string(),
            channels: vec!["testchannel".to_string()],
            ..Default::default()
        };

        let adapter = TwitchAdapter::new(config)
            .await
            .expect("TwitchAdapter::new should succeed");
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "twitch");
    }

    /// #427: malformed/adversarial IRC lines from the Twitch server must
    /// never panic `handle_irc_message`, regardless of `!`/`:`/space
    /// placement or truncated PING commands. Each of these previously
    /// panicked with a byte-slice range/index-out-of-bounds error.
    #[tokio::test]
    async fn test_handle_irc_message_malformed_lines_do_not_panic() {
        let (tx, mut rx) = mpsc::channel::<ChannelMessage>(8);
        let writer: Arc<RwLock<Option<tokio::net::tcp::OwnedWriteHalf>>> =
            Arc::new(RwLock::new(None));

        let malformed_lines = [
            // Bare PING with no trailing " <token>" — used to panic on
            // `&line[5..]` when line.len() < 6.
            "PING",
            "PIN", // shorter than "PING" itself, doesn't even match starts_with
            // "!" appears before ":" so user_end < user_start — used to
            // panic on `&line[user_start..user_end]`.
            "!before:colon PRIVMSG #chan :hi",
            // No "!" and no ":" at all — user_start=0, user_end=0, safe,
            // but exercise anyway for the channel-slice path below.
            "PRIVMSG",
            // "PRIVMSG" right at the end of the line with nothing after —
            // used to panic on `line[channel_start..]`.
            ":user!user@host PRIVMSG",
            ":user!user@host PRIVMSGx", // one char short of the old bound
            "",
        ];

        for line in malformed_lines {
            let result =
                TwitchAdapter::handle_irc_message(line, &tx, "testbot", &writer).await;
            assert!(
                result.is_ok(),
                "handle_irc_message should not error (and must not panic) on line: {line:?}"
            );
        }

        // A well-formed PRIVMSG still parses correctly after the hardening.
        let ok_line = ":alice!alice@host.tmi.twitch.tv PRIVMSG #zeuschan :hello world";
        TwitchAdapter::handle_irc_message(ok_line, &tx, "testbot", &writer)
            .await
            .expect("well-formed PRIVMSG should still succeed");
        let msg = rx.try_recv().expect("well-formed message should be forwarded");
        assert_eq!(msg.content, "hello world");
    }
}
