//! IRC Channel Adapter
//!
//! Tokio-based IRC client implementing `ChannelAdapter` for Zeus.
//! Supports connecting to IRC servers, joining channels, sending/receiving
//! messages, and nick management.

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc};
use tokio_rustls::TlsConnector as RustlsConnector;
use tracing::{debug, error, info, warn};

/// #66-L1: detect whether `text` highlights `own_nick` per IRC convention.
///
/// Returns true when `own_nick` (case-insensitive) appears in `text` as a
/// word-boundary-delimited token followed by a common highlight delimiter
/// (`:`, `,`, space, tab) or end-of-line. Returns false when `own_nick`
/// is empty.
///
/// Examples that match (own_nick = "zeus"):
///   "zeus: hi", "zeus, hi", "hey zeus", "yo Zeus", "ZEUS!"-no (no delim)
fn is_nick_highlighted(text: &str, own_nick: &str) -> bool {
    if own_nick.is_empty() {
        return false;
    }
    let lower_text = text.to_lowercase();
    let lower_nick = own_nick.to_lowercase();
    let nlen = lower_nick.len();
    let mut start = 0;
    while let Some(idx) = lower_text[start..].find(&lower_nick) {
        let abs = start + idx;
        let before_ok =
            abs == 0 || !lower_text.as_bytes()[abs - 1].is_ascii_alphanumeric();
        let after = abs + nlen;
        let after_ok = after == lower_text.len()
            || matches!(
                lower_text.as_bytes()[after],
                b':' | b',' | b' ' | b'\t'
            );
        if before_ok && after_ok {
            return true;
        }
        start = abs + nlen;
    }
    false
}
use zeus_core::{Error, Result};

// ── Config ──────────────────────────────────────────────────────────────

/// IRC connection configuration.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct IrcConfig {
    /// IRC server hostname (e.g. "irc.libera.chat").
    pub server: String,
    /// Port (default: 6667 for plain, 6697 for TLS).
    #[serde(default = "default_port")]
    pub port: u16,
    /// Bot nickname.
    pub nick: String,
    /// Optional username (defaults to nick).
    #[serde(default)]
    pub username: Option<String>,
    /// Optional real name (defaults to "Zeus IRC Bot").
    #[serde(default)]
    pub realname: Option<String>,
    /// Channels to join on connect (e.g. ["#zeus", "#dev"]).
    #[serde(default)]
    pub channels: Vec<String>,
    /// Use TLS (default: false).
    #[serde(default)]
    pub use_tls: bool,
    /// Server password (optional, for password-protected servers).
    #[serde(default, skip_serializing)]
    pub server_password: Option<String>,
    /// NickServ password (optional, for registered nicks).
    #[serde(default, skip_serializing)]
    pub nickserv_password: Option<String>,
    /// Command prefix for bot commands (default: "!").
    #[serde(default = "default_prefix")]
    pub command_prefix: Option<String>,
}

fn default_port() -> u16 {
    6667
}

fn default_prefix() -> Option<String> {
    Some("!".to_string())
}

// ── IRC Message Parsing ─────────────────────────────────────────────────

/// A parsed IRC protocol message.
#[derive(Debug, Clone)]
pub struct IrcMessage {
    /// Optional prefix (e.g. "nick!user@host").
    pub prefix: Option<String>,
    /// Command (e.g. "PRIVMSG", "PING", "001").
    pub command: String,
    /// Parameters.
    pub params: Vec<String>,
}

impl IrcMessage {
    /// Parse a raw IRC line into an IrcMessage.
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return None;
        }

        let mut remaining = line;
        let prefix = if remaining.starts_with(':') {
            let space = remaining.find(' ')?;
            let pfx = remaining[1..space].to_string();
            remaining = &remaining[space + 1..];
            Some(pfx)
        } else {
            None
        };

        // Split off trailing param (after " :")
        let (before_trailing, trailing) = if let Some(idx) = remaining.find(" :") {
            let trail = remaining[idx + 2..].to_string();
            (&remaining[..idx], Some(trail))
        } else {
            (remaining, None)
        };

        let mut parts: Vec<String> = before_trailing
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        if parts.is_empty() {
            return None;
        }

        let command = parts.remove(0);
        let mut params = parts;
        if let Some(trail) = trailing {
            params.push(trail);
        }

        Some(Self {
            prefix,
            command,
            params,
        })
    }

    /// Extract the nick from the prefix (before '!').
    pub fn nick(&self) -> Option<&str> {
        self.prefix.as_deref().and_then(|p| p.split('!').next())
    }
}

// ── IrcWriter ───────────────────────────────────────────────────────────

/// Shared writer for sending IRC commands (supports both plain TCP and TLS streams).
type IrcWriter = Arc<Mutex<Option<Box<dyn AsyncWrite + Unpin + Send>>>>;

/// Send a raw IRC line.
async fn send_raw(writer: &IrcWriter, line: &str) -> Result<()> {
    let mut guard = writer.lock().await;
    if let Some(w) = guard.as_mut() {
        let data = format!("{}\r\n", line);
        w.write_all(data.as_bytes())
            .await
            .map_err(|e| Error::channel(format!("IRC write error: {}", e)))?;
        w.flush()
            .await
            .map_err(|e| Error::channel(format!("IRC flush error: {}", e)))?;
        debug!("IRC >> {}", line);
        Ok(())
    } else {
        Err(Error::channel("IRC writer not connected"))
    }
}

// ── Adapter ─────────────────────────────────────────────────────────────

/// IRC channel adapter using raw tokio TCP.
pub struct IrcAdapter {
    config: IrcConfig,
    writer: IrcWriter,
    connected: Arc<AtomicBool>,
    shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl IrcAdapter {
    /// Create a new IRC adapter.
    pub fn new(config: IrcConfig) -> Self {
        Self {
            config,
            writer: Arc::new(Mutex::new(None)),
            connected: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(Mutex::new(None)),
        }
    }

    /// Send a PRIVMSG to a channel or user.
    ///
    /// Strips Markdown syntax before chunking — IRC has no markup-render
    /// layer, so naked `**bold**` / `` `code` `` from titans would otherwise
    /// reach end users as literal syntax (catch-#45 follow-up: β'-A).
    pub async fn privmsg(&self, target: &str, message: &str) -> Result<()> {
        // Sanitize BEFORE chunking so paired-delimiter strip sees the whole
        // message (chunk boundary could otherwise split a delimiter pair).
        let sanitized = crate::sanitize::strip_markdown(message);
        // Split long messages at 400-char boundaries (IRC limit ~512 including headers)
        let max_len = 400;
        for chunk in sanitized
            .as_bytes()
            .chunks(max_len)
            .map(|c| String::from_utf8_lossy(c))
        {
            send_raw(&self.writer, &format!("PRIVMSG {} :{}", target, chunk)).await?;
        }
        Ok(())
    }

    /// Join an IRC channel.
    pub async fn join_channel(&self, channel: &str) -> Result<()> {
        send_raw(&self.writer, &format!("JOIN {}", channel)).await
    }

    /// Part (leave) an IRC channel.
    pub async fn part_channel(&self, channel: &str, reason: Option<&str>) -> Result<()> {
        match reason {
            Some(r) => send_raw(&self.writer, &format!("PART {} :{}", channel, r)).await,
            None => send_raw(&self.writer, &format!("PART {}", channel)).await,
        }
    }

    /// Change the bot's nick.
    pub async fn change_nick(&self, new_nick: &str) -> Result<()> {
        send_raw(&self.writer, &format!("NICK {}", new_nick)).await
    }

    /// Send a raw IRC command.
    pub async fn raw_command(&self, command: &str) -> Result<()> {
        send_raw(&self.writer, command).await
    }
}

#[async_trait]
impl ChannelAdapter for IrcAdapter {
    fn channel_type(&self) -> &'static str {
        "irc"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Native
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        if self.config.server.is_empty() {
            return Err(Error::channel("IRC server hostname is required"));
        }
        if self.config.nick.is_empty() {
            return Err(Error::channel("IRC nick is required"));
        }

        let addr = format!("{}:{}", self.config.server, self.config.port);
        info!(
            "Connecting to IRC server {} (TLS: {})",
            addr, self.config.use_tls
        );

        let tcp = TcpStream::connect(&addr)
            .await
            .map_err(|e| Error::channel(format!("IRC connect failed: {}", e)))?;

        // Upgrade to TLS if requested, otherwise use plain TCP
        let (reader, writer_half): (
            Box<dyn AsyncRead + Unpin + Send>,
            Box<dyn AsyncWrite + Unpin + Send>,
        ) = if self.config.use_tls {
            let root_store = tokio_rustls::rustls::RootCertStore::from_iter(
                webpki_roots::TLS_SERVER_ROOTS.iter().cloned()
            );
            let tls_config = tokio_rustls::rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth();
            let connector = RustlsConnector::from(Arc::new(tls_config));
            let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from(self.config.server.clone())
                .map_err(|e| Error::channel(format!("IRC invalid server name: {}", e)))?;
            let tls_stream = connector
                .connect(server_name, tcp)
                .await
                .map_err(|e| Error::channel(format!("IRC TLS handshake failed: {}", e)))?;
            let (r, w) = tokio::io::split(tls_stream);
            (Box::new(r), Box::new(w))
        } else {
            let (r, w) = tokio::io::split(tcp);
            (Box::new(r), Box::new(w))
        };

        // Store writer
        {
            let mut w = self.writer.lock().await;
            *w = Some(writer_half);
        }

        // Send registration
        if let Some(ref pass) = self.config.server_password {
            send_raw(&self.writer, &format!("PASS {}", pass)).await?;
        }

        let username = self.config.username.as_deref().unwrap_or(&self.config.nick);
        let realname = self.config.realname.as_deref().unwrap_or("Zeus IRC Bot");

        send_raw(&self.writer, &format!("NICK {}", self.config.nick)).await?;
        send_raw(
            &self.writer,
            &format!("USER {} 0 * :{}", username, realname),
        )
        .await?;

        // Create shutdown channel
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        {
            let mut guard = self.shutdown.lock().await;
            *guard = Some(shutdown_tx);
        }

        let connected = self.connected.clone();
        let writer = self.writer.clone();
        let channels_to_join = self.config.channels.clone();
        let nickserv_pass = self.config.nickserv_password.clone();
        // #66-L1: capture own nick for mention/highlight detection on inbound PRIVMSG.
        let own_nick = self.config.nick.clone();

        // Spawn read loop
        tokio::spawn(async move {
            let mut buf_reader = BufReader::new(reader);
            let mut line_buf = String::new();
            let mut registered = false;

            loop {
                line_buf.clear();
                tokio::select! {
                    result = buf_reader.read_line(&mut line_buf) => {
                        match result {
                            Ok(0) => {
                                info!("IRC connection closed by server");
                                break;
                            }
                            Ok(_) => {
                                let line = line_buf.trim_end();
                                debug!("IRC << {}", line);

                                let Some(msg) = IrcMessage::parse(line) else {
                                    continue;
                                };

                                match msg.command.as_str() {
                                    "PING" => {
                                        let token = msg.params.first().map(|s| s.as_str()).unwrap_or("");
                                        let _ = send_raw(&writer, &format!("PONG :{}", token)).await;
                                    }
                                    // RPL_WELCOME — registration complete
                                    "001" => {
                                        info!("IRC registered successfully");
                                        connected.store(true, Ordering::SeqCst);
                                        registered = true;

                                        // Identify with NickServ
                                        if let Some(ref pass) = nickserv_pass {
                                            let _ = send_raw(&writer, &format!("PRIVMSG NickServ :IDENTIFY {}", pass)).await;
                                        }

                                        // Join configured channels
                                        for ch in &channels_to_join {
                                            let _ = send_raw(&writer, &format!("JOIN {}", ch)).await;
                                        }
                                    }
                                    "PRIVMSG" => {
                                        if !registered {
                                            continue;
                                        }
                                        let nick = msg.nick().unwrap_or("unknown").to_string();
                                        let target = msg.params.first().cloned().unwrap_or_default();
                                        let text = msg.params.get(1).cloned().unwrap_or_default();

                                        // Skip empty messages
                                        if text.is_empty() {
                                            continue;
                                        }

                                        let is_channel = target.starts_with('#') || target.starts_with('&');
                                        // #66-L1: DM → always addressed; channel →
                                        // addressed when our nick is highlighted in the text.
                                        let is_addressed = !is_channel
                                            || is_nick_highlighted(&text, &own_nick);

                                        let source = if is_channel {
                                            // Channel message
                                            ChannelSource::with_chat("irc", &nick, &target)
                                        } else {
                                            // Private message (DM)
                                            ChannelSource::new("irc", &nick)
                                        }
                                        // IRC has no bot/human distinction at the protocol level;
                                        // treat all PRIVMSG senders as Human so downstream filters
                                        // (mention routing, DM implicit-addressing) don't drop them
                                        // as Unknown. See fix/irc-dm-routing-v2.
                                        .with_sender_type(zeus_core::SenderType::Human);

                                        // Prefix sender nick into content (Discord parity, #317).
                                        // Applied AFTER is_addressed computation so trigger
                                        // detection never sees the mutated string.
                                        let prefixed_text = format!("[{}]: {}", nick, text);
                                        let channel_msg = ChannelMessage::new(source, prefixed_text)
                                            .with_addressed(is_addressed);
                                        if tx.send(channel_msg).await.is_err() {
                                            warn!("IRC message receiver dropped");
                                            break;
                                        }
                                    }
                                    "NOTICE" => {
                                        debug!("IRC NOTICE: {:?}", msg.params);
                                    }
                                    "JOIN" => {
                                        let channel = msg.params.first().map(|s| s.as_str()).unwrap_or("?");
                                        let who = msg.nick().unwrap_or("?");
                                        debug!("IRC {} joined {}", who, channel);
                                    }
                                    "PART" | "QUIT" => {
                                        let who = msg.nick().unwrap_or("?");
                                        debug!("IRC {} left: {:?}", who, msg.params);
                                    }
                                    // Numeric error replies
                                    "433" => {
                                        // Nick already in use
                                        warn!("IRC nick already in use, appending underscore");
                                        let _ = send_raw(&writer, &format!("NICK {}_", msg.params.get(1).map(|s| s.as_str()).unwrap_or("zeus"))).await;
                                    }
                                    "ERROR" => {
                                        let reason = msg.params.first().map(|s| s.as_str()).unwrap_or("unknown");
                                        error!("IRC ERROR: {}", reason);
                                        break;
                                    }
                                    _ => {
                                        // Ignore other numerics/commands
                                    }
                                }
                            }
                            Err(e) => {
                                error!("IRC read error: {}", e);
                                break;
                            }
                        }
                    }
                    _ = &mut shutdown_rx => {
                        info!("IRC shutdown signal received");
                        let _ = send_raw(&writer, "QUIT :Zeus shutting down").await;
                        break;
                    }
                }
            }

            connected.store(false, Ordering::SeqCst);
            // Clear writer
            let mut w = writer.lock().await;
            *w = None;
        });

        info!(
            "IRC adapter started ({}:{})",
            self.config.server, self.config.port
        );
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        // Signal shutdown
        {
            let mut guard = self.shutdown.lock().await;
            if let Some(tx) = guard.take() {
                let _ = tx.send(());
            }
        }

        self.connected.store(false, Ordering::SeqCst);

        // Clear writer
        {
            let mut w = self.writer.lock().await;
            *w = None;
        }

        info!("IRC adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "irc" {
            return Err(Error::channel("Invalid channel source for IRC"));
        }

        // Send to chat_id (channel) or user_id (DM)
        let target = to.chat_id.as_deref().unwrap_or(&to.user_id);

        self.privmsg(target, content).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn supports_typing(&self) -> bool {
        false // IRC has no typing indicator
    }

    fn supports_presence(&self) -> bool {
        false
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // -- privmsg outbound sanitization (catch-#45 β'-A) --

    #[test]
    fn privmsg_sanitizes_markdown_before_chunking() {
        // Contract: the sanitizer used by privmsg() strips Markdown so IRC
        // users don't see literal `**bold**` / `` `code` `` syntax. This
        // mirrors the Telegram outbound sanitization (#45-β) at the IRC
        // transport site (irc.rs::privmsg pre-chunk).
        let raw = "**bold** and `code` and # Heading";
        let out = crate::sanitize::strip_markdown(raw);
        assert!(!out.contains("**"));
        assert!(!out.contains('`'));
        assert!(!out.starts_with('#'));
        assert!(out.contains("bold"));
        assert!(out.contains("code"));
        assert!(out.contains("Heading"));
    }

    // -- IrcMessage parsing --

    #[test]
    fn test_parse_privmsg() {
        let line = ":nick!user@host PRIVMSG #channel :Hello world";
        let msg = IrcMessage::parse(line).unwrap();
        assert_eq!(msg.prefix.as_deref(), Some("nick!user@host"));
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(msg.params, vec!["#channel", "Hello world"]);
        assert_eq!(msg.nick(), Some("nick"));
    }

    #[test]
    fn test_parse_ping() {
        let line = "PING :server.example.com";
        let msg = IrcMessage::parse(line).unwrap();
        assert!(msg.prefix.is_none());
        assert_eq!(msg.command, "PING");
        assert_eq!(msg.params, vec!["server.example.com"]);
    }

    #[test]
    fn test_parse_numeric() {
        let line = ":server 001 zeus :Welcome to the IRC network";
        let msg = IrcMessage::parse(line).unwrap();
        assert_eq!(msg.prefix.as_deref(), Some("server"));
        assert_eq!(msg.command, "001");
        assert_eq!(msg.params, vec!["zeus", "Welcome to the IRC network"]);
    }

    #[test]
    fn test_parse_join() {
        let line = ":nick!user@host JOIN #channel";
        let msg = IrcMessage::parse(line).unwrap();
        assert_eq!(msg.command, "JOIN");
        assert_eq!(msg.params, vec!["#channel"]);
    }

    #[test]
    fn test_parse_nick_in_use() {
        let line = ":server 433 * zeus :Nickname is already in use";
        let msg = IrcMessage::parse(line).unwrap();
        assert_eq!(msg.command, "433");
        assert_eq!(msg.params[1], "zeus");
    }

    #[test]
    fn test_parse_empty_line() {
        assert!(IrcMessage::parse("").is_none());
        assert!(IrcMessage::parse("\r\n").is_none());
    }

    #[test]
    fn test_parse_no_trailing() {
        let line = ":nick!user@host QUIT";
        let msg = IrcMessage::parse(line).unwrap();
        assert_eq!(msg.command, "QUIT");
        assert!(msg.params.is_empty());
    }

    #[test]
    fn test_parse_multiple_params() {
        let line = ":server 353 zeus = #channel :nick1 nick2 nick3";
        let msg = IrcMessage::parse(line).unwrap();
        assert_eq!(msg.command, "353");
        assert_eq!(
            msg.params,
            vec!["zeus", "=", "#channel", "nick1 nick2 nick3"]
        );
    }

    #[test]
    fn test_nick_extraction() {
        let line = ":alice!alice@host.com PRIVMSG #test :hello";
        let msg = IrcMessage::parse(line).unwrap();
        assert_eq!(msg.nick(), Some("alice"));
    }

    #[test]
    fn test_nick_no_prefix() {
        let line = "PING :token";
        let msg = IrcMessage::parse(line).unwrap();
        assert!(msg.nick().is_none());
    }

    // -- IrcConfig --

    #[test]
    fn test_irc_config_default() {
        let config = IrcConfig::default();
        assert!(config.server.is_empty());
        assert_eq!(config.port, 0); // Default derive uses 0
        assert!(config.nick.is_empty());
        assert!(config.channels.is_empty());
        assert!(!config.use_tls);
    }

    #[test]
    fn test_irc_config_serialization() {
        let config = IrcConfig {
            server: "irc.libera.chat".to_string(),
            port: 6667,
            nick: "zeus-bot".to_string(),
            username: Some("zeus".to_string()),
            realname: Some("Zeus IRC Bot".to_string()),
            channels: vec!["#zeus".to_string(), "#dev".to_string()],
            use_tls: false,
            server_password: None,
            nickserv_password: Some("secret".to_string()),
            command_prefix: Some("!".to_string()),
        };

        let json = serde_json::to_string(&config).unwrap();
        // Credentials must not appear in serialized output
        assert!(
            !json.contains("secret"),
            "nickserv_password must not be serialized"
        );
        let back: IrcConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.server, "irc.libera.chat");
        assert_eq!(back.port, 6667);
        assert_eq!(back.nick, "zeus-bot");
        assert_eq!(back.channels.len(), 2);
        // Passwords are intentionally None after roundtrip (skip_serializing security policy)
        assert!(back.nickserv_password.is_none());
    }

    #[test]
    fn test_irc_config_deserialization_defaults() {
        let json = r#"{"server":"irc.freenode.net","nick":"bot","port":6667}"#;
        let config: IrcConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.server, "irc.freenode.net");
        assert!(config.channels.is_empty());
        assert!(!config.use_tls);
        assert!(config.server_password.is_none());
    }

    // -- IrcAdapter --

    #[test]
    fn test_adapter_creation() {
        let config = IrcConfig {
            server: "irc.libera.chat".to_string(),
            port: 6667,
            nick: "zeus".to_string(),
            ..Default::default()
        };
        let adapter = IrcAdapter::new(config);
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "irc");
    }

    #[test]
    fn test_adapter_channel_type() {
        let adapter = IrcAdapter::new(IrcConfig::default());
        assert_eq!(adapter.channel_type(), "irc");
    }

    #[test]
    fn test_adapter_receive_mode() {
        let adapter = IrcAdapter::new(IrcConfig::default());
        assert!(matches!(adapter.receive_mode(), ReceiveMode::Native));
    }

    #[test]
    fn test_adapter_supports_typing() {
        let adapter = IrcAdapter::new(IrcConfig::default());
        assert!(!adapter.supports_typing());
    }

    #[test]
    fn test_adapter_supports_presence() {
        let adapter = IrcAdapter::new(IrcConfig::default());
        assert!(!adapter.supports_presence());
    }

    #[tokio::test]
    async fn test_start_requires_server() {
        let config = IrcConfig {
            server: String::new(),
            nick: "bot".to_string(),
            ..Default::default()
        };
        let adapter = IrcAdapter::new(config);
        let (tx, _rx) = mpsc::channel(10);
        let result = adapter.start(tx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("server"));
    }

    #[tokio::test]
    async fn test_start_requires_nick() {
        let config = IrcConfig {
            server: "irc.example.com".to_string(),
            nick: String::new(),
            ..Default::default()
        };
        let adapter = IrcAdapter::new(config);
        let (tx, _rx) = mpsc::channel(10);
        let result = adapter.start(tx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("nick"));
    }

    #[tokio::test]
    async fn test_stop_idempotent() {
        let adapter = IrcAdapter::new(IrcConfig::default());
        // Stop without start should not panic
        let result = adapter.stop().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_wrong_channel_type() {
        let adapter = IrcAdapter::new(IrcConfig::default());
        let source = ChannelSource::new("telegram", "user");
        let result = adapter.send(&source, "hello").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid channel"));
    }

    #[tokio::test]
    async fn test_send_not_connected() {
        let adapter = IrcAdapter::new(IrcConfig::default());
        let source = ChannelSource::with_chat("irc", "user", "#channel");
        let result = adapter.send(&source, "hello").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not connected"));
    }

    // -- Channel source mapping --

    #[test]
    fn test_channel_source_for_irc_channel() {
        let source = ChannelSource::with_chat("irc", "alice", "#zeus");
        assert_eq!(source.channel_type(), "irc");
        assert_eq!(source.user_id, "alice");
        assert_eq!(source.chat_id.as_deref(), Some("#zeus"));
    }

    #[test]
    fn test_channel_source_for_irc_dm() {
        let source = ChannelSource::new("irc", "alice");
        assert_eq!(source.channel_type(), "irc");
        assert_eq!(source.user_id, "alice");
        assert!(source.chat_id.is_none());
    }

    /// Regression test for `fix/irc-dm-routing-v2`: IRC PRIVMSG handler must
    /// stamp `sender_type=Human` so downstream mention/DM filters don't drop
    /// the message as `Unknown` (the SenderType default). Without this, IRC DMs
    /// — which have no chat_id and no mention keyword to bypass the filter —
    /// were silently swallowed.
    #[test]
    fn test_irc_inbound_sender_type_is_human() {
        let dm_source = ChannelSource::new("irc", "miguel")
            .with_sender_type(zeus_core::SenderType::Human);
        assert_eq!(dm_source.sender_type, zeus_core::SenderType::Human);
        assert!(dm_source.sender_type.is_human());
        assert!(dm_source.chat_id.is_none(), "DM must have no chat_id");

        let chan_source = ChannelSource::with_chat("irc", "miguel", "#general")
            .with_sender_type(zeus_core::SenderType::Human);
        assert_eq!(chan_source.sender_type, zeus_core::SenderType::Human);
        assert_eq!(chan_source.chat_id.as_deref(), Some("#general"));
    }

    // -- #66-L1: nick-highlight detection (is_addressed for IRC channel msgs) --

    #[test]
    fn test_nick_highlight_colon_suffix() {
        assert!(is_nick_highlighted("zeus: hello there", "zeus"));
    }

    #[test]
    fn test_nick_highlight_comma_suffix() {
        assert!(is_nick_highlighted("zeus, ping", "zeus"));
    }

    #[test]
    fn test_nick_highlight_case_insensitive() {
        assert!(is_nick_highlighted("ZEUS: hi", "zeus"));
        assert!(is_nick_highlighted("hey Zeus, status?", "zeus"));
    }

    #[test]
    fn test_nick_highlight_midline_word() {
        assert!(is_nick_highlighted("hey zeus what's up", "zeus"));
    }

    #[test]
    fn test_nick_highlight_end_of_line() {
        assert!(is_nick_highlighted("ping zeus", "zeus"));
    }

    #[test]
    fn test_nick_no_highlight_substring() {
        // "zeusbot" contains "zeus" but isn't a word-boundary highlight
        assert!(!is_nick_highlighted("zeusbot is broken", "zeus"));
        assert!(!is_nick_highlighted("hello dzeus", "zeus"));
    }

    #[test]
    fn test_nick_no_highlight_unrelated() {
        assert!(!is_nick_highlighted("hello world", "zeus"));
    }

    #[test]
    fn test_nick_empty_returns_false() {
        assert!(!is_nick_highlighted("any text", ""));
    }

    #[test]
    fn test_nick_highlight_at_start() {
        assert!(is_nick_highlighted("zeus", "zeus"));
        assert!(is_nick_highlighted("zeus hi", "zeus"));
    }

    #[test]
    fn test_nick_highlight_punctuation_only_no_match() {
        // "zeus!" — no highlight delimiter follows, not a mention
        assert!(!is_nick_highlighted("zeus!", "zeus"));
    }

    // -- #317: sender prefix in content (Discord parity) --

    #[test]
    fn test_irc_sender_prefix_format() {
        // Contract: content is prefixed with "[nick]: " so the agent
        // knows who sent the message, mirroring discord.rs:573.
        let nick = "merakizzz";
        let text = "hello world";
        let prefixed = format!("[{}]: {}", nick, text);
        assert_eq!(prefixed, "[merakizzz]: hello world");
    }

    #[test]
    fn test_irc_prefix_applied_after_addressing() {
        // Contract: is_addressed is computed on the ORIGINAL text (before
        // prefix), so trigger detection never sees "[nick]: " in the string.
        // Simulate the flow: compute is_addressed on raw text, then prefix.
        let nick = "zeus";
        let text = "zeus hi there"; // would trigger is_nick_highlighted
        let is_addressed = is_nick_highlighted(text, nick);
        assert!(is_addressed); // detected on raw text

        // Now prefix — the prefixed string should NOT be re-evaluated
        let prefixed = format!("[{}]: {}", nick, text);
        // The prefix itself doesn't contain a bare "zeus " at start
        assert!(!prefixed.starts_with("zeus "));
    }
}
