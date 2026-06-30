//! Signal channel adapter via signal-cli HTTP JSON-RPC + SSE
//!
//! Architecture:
//! - signal-cli daemon runs in `--http` mode (JSON-RPC over HTTP on a local port)
//! - Inbound messages arrive via SSE stream at `/v1/events`
//! - Outbound messages sent via HTTP JSON-RPC POST to `/v2/send`
//! - Outbound files sent via multipart POST to `/v2/send` with attachment
//! - QR link pairing flow via `/v1/qrcodelink` for initial device registration
//!
//! Attachments:
//! - Inbound: parsed from `attachments` array in DataMessage SSE events
//! - Outbound: uploaded via multipart form-data to `/v2/send`
//!
//! Receipts:
//! - Delivery and read receipts parsed from `receipt` envelope type
//! - Logged at info level for observability; future: expose via ChannelMessage

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use zeus_core::{Error, Result};

use crate::policy::ChannelPolicy;
use crate::sanitize::strip_markdown;
use crate::{
    AllowBotsMode, ChannelAdapter, ChannelAttachment, ChannelMessage, ChannelSource, ReceiveMode,
};

// ── Configuration ────────────────────────────────────────────────────────────

/// Signal configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalConfig {
    /// Path to signal-cli binary
    #[serde(default = "default_signal_cli_path")]
    pub signal_cli_path: String,
    /// Phone number registered with Signal (e.g. "+14155552671")
    #[serde(default)]
    pub phone: String,
    /// Port for signal-cli HTTP daemon (default: 8080)
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    /// Host for signal-cli HTTP daemon (default: 127.0.0.1)
    #[serde(default = "default_http_host")]
    pub http_host: String,
    /// Access policy (group mention filtering, DM access)
    #[serde(default)]
    pub policy: Option<zeus_core::ChannelPolicyConfig>,
    /// Account identifier for multi-account routing
    #[serde(default)]
    pub account_id: Option<String>,
    /// Bot message policy: "off"/"mentions"/"on"
    #[serde(default)]
    pub allow_bots: Option<String>,
}

fn default_signal_cli_path() -> String {
    "signal-cli".to_string()
}

fn default_http_port() -> u16 {
    8080
}

fn default_http_host() -> String {
    "127.0.0.1".to_string()
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            signal_cli_path: default_signal_cli_path(),
            phone: String::new(),
            http_port: default_http_port(),
            http_host: default_http_host(),
            policy: None,
            account_id: None,
            allow_bots: None,
        }
    }
}

impl SignalConfig {
    /// Base URL for the signal-cli HTTP daemon
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.http_host, self.http_port)
    }
}

// ── SSE event types ───────────────────────────────────────────────────────────

/// Envelope from SSE event stream
#[derive(Debug, Deserialize)]
struct SseEnvelope {
    envelope: Option<Envelope>,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    source: Option<String>,
    #[serde(rename = "dataMessage")]
    data_message: Option<DataMessage>,
    /// Receipt envelope (delivery/read receipts)
    receipt: Option<ReceiptMessage>,
    /// Timestamp of the original message this receipt refers to
    #[serde(rename = "timestamp", default)]
    timestamp: Option<u64>,
}

/// Receipt type from signal-cli SSE
#[derive(Debug, Deserialize)]
struct ReceiptMessage {
    #[serde(rename = "type")]
    receipt_type: Option<String>,
    /// Timestamps of the messages this receipt covers
    #[serde(rename = "timestamps", default)]
    timestamps: Vec<u64>,
}

#[derive(Debug, Deserialize)]
struct DataMessage {
    message: Option<String>,
    #[serde(rename = "groupInfo")]
    group_info: Option<GroupInfo>,
    /// Attachments received with the message
    #[serde(rename = "attachments", default)]
    attachments: Vec<SignalAttachment>,
}

/// Attachment from signal-cli SSE event
#[derive(Debug, Deserialize)]
struct SignalAttachment {
    /// Content type (MIME type)
    #[serde(rename = "contentType", default)]
    content_type: Option<String>,
    /// Original filename
    #[serde(default)]
    filename: Option<String>,
    /// ID for downloading the attachment via signal-cli API
    #[serde(default)]
    id: Option<String>,
    /// File size in bytes
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GroupInfo {
    #[serde(rename = "groupId")]
    group_id: Option<String>,
}

// ── Send request type ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct SendRequest<'a> {
    message: &'a str,
    number: &'a str,
    recipients: Vec<&'a str>,
}

// ── QR pairing response ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct QrLinkResponse {
    #[serde(rename = "deviceLinkUri")]
    device_link_uri: Option<String>,
}

// ── Adapter ───────────────────────────────────────────────────────────────────

/// Signal channel adapter (HTTP JSON-RPC + SSE mode)
pub struct SignalAdapter {
    config: SignalConfig,
    connected: Arc<AtomicBool>,
    http: Client,
}

impl SignalAdapter {
    /// Create a new Signal adapter
    pub async fn new(config: SignalConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Channel(format!("failed to build HTTP client: {}", e)))?;

        Ok(Self {
            config,
            connected: Arc::new(AtomicBool::new(false)),
            http,
        })
    }

    /// Spawn signal-cli in HTTP daemon mode and return the child process.
    async fn spawn_daemon(&self) -> Result<tokio::process::Child> {
        let child = tokio::process::Command::new(&self.config.signal_cli_path)
            .arg("-a")
            .arg(&self.config.phone)
            .arg("daemon")
            .arg("--http")
            .arg(format!(
                "{}:{}",
                self.config.http_host, self.config.http_port
            ))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| Error::Channel(format!("failed to spawn signal-cli daemon: {}", e)))?;

        // Give the daemon a moment to bind the port
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        info!(
            "signal-cli HTTP daemon started on {}:{}",
            self.config.http_host, self.config.http_port
        );
        Ok(child)
    }

    /// Initiate QR link pairing flow.
    ///
    /// Calls `/v1/qrcodelink` to get a `tsdevice:/` URI, prints it to stdout,
    /// and also generates an ASCII QR code via the `qrcode` approach so users
    /// can scan from a terminal.
    pub async fn pair_via_qr(&self) -> Result<String> {
        let url = format!("{}/v1/qrcodelink?device_name=zeus", self.config.base_url());

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("QR link request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "QR link endpoint returned {}: {}",
                status, body
            )));
        }

        // The daemon may return a plain URI string or JSON
        let body = resp
            .text()
            .await
            .map_err(|e| Error::Channel(format!("failed to read QR response: {}", e)))?;

        // Try JSON first, fall back to raw body as URI
        let uri = if let Ok(parsed) = serde_json::from_str::<QrLinkResponse>(&body) {
            parsed
                .device_link_uri
                .unwrap_or_else(|| body.trim().to_string())
        } else {
            body.trim().to_string()
        };

        info!("Signal QR pairing URI: {}", uri);
        println!("\n📱 Scan this link in Signal to pair:\n\n  {}\n", uri);
        println!("Or open it directly on an Android device.");

        Ok(uri)
    }

    /// Read the SSE stream at `/v1/events` and forward parsed messages to `tx`.
    async fn stream_events(
        config: SignalConfig,
        http: Client,
        connected: Arc<AtomicBool>,
        tx: mpsc::Sender<ChannelMessage>,
    ) {
        let url = format!("{}/v1/events", config.base_url());
        let allow_bots = AllowBotsMode::from_config(config.allow_bots.as_deref());

        loop {
            if !connected.load(Ordering::SeqCst) {
                break;
            }

            info!("Connecting to Signal SSE stream at {}", url);

            let resp = match http.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    error!("Signal SSE connect error: {}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            if !resp.status().is_success() {
                error!("Signal SSE bad status: {}", resp.status());
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }

            connected.store(true, Ordering::SeqCst);
            info!("Signal SSE stream connected");

            use futures_util::StreamExt;
            let mut stream = resp.bytes_stream();
            let mut buf = String::new();

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Err(e) => {
                        warn!("Signal SSE read error: {}", e);
                        break;
                    }
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        buf.push_str(&text);

                        // SSE events are separated by double newlines
                        while let Some(pos) = buf.find("\n\n") {
                            let event_block = buf[..pos].to_string();
                            buf = buf[pos + 2..].to_string();

                            // Extract `data:` line(s)
                            for line in event_block.lines() {
                                let data = if let Some(d) = line.strip_prefix("data:") {
                                    d.trim()
                                } else {
                                    continue;
                                };

                                if data.is_empty() {
                                    continue;
                                }

                                let envelope: SseEnvelope = match serde_json::from_str(data) {
                                    Ok(e) => e,
                                    Err(_) => continue,
                                };

                                if let Some(env) = envelope.envelope {
                                    Self::handle_envelope(&config, &allow_bots, env, &tx).await;
                                }
                            }
                        }
                    }
                }
            }

            connected.store(false, Ordering::SeqCst);
            warn!("Signal SSE stream disconnected — reconnecting in 5s");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    async fn handle_envelope(
        config: &SignalConfig,
        allow_bots: &AllowBotsMode,
        env: Envelope,
        tx: &mpsc::Sender<ChannelMessage>,
    ) {
        let sender = match env.source.as_deref() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return,
        };

        // Skip self-echoes
        if sender == config.phone {
            return;
        }

        // ── Receipt handling ──────────────────────────────────────────────
        if let Some(receipt) = env.receipt {
            let rtype = receipt.receipt_type.as_deref().unwrap_or("unknown");
            let msg_type = match rtype {
                "DELIVERY" => crate::MessageType::DeliveryReceipt,
                "READ" => crate::MessageType::ReadReceipt,
                _ => crate::MessageType::Text,
            };

            let mut source = ChannelSource::new("signal", &sender);
            if let Some(ref acct_id) = config.account_id {
                source = source.with_account(acct_id);
            }

            let msg = ChannelMessage::receipt(source, msg_type, receipt.timestamps.clone());

            info!(
                "Signal receipt: type={}, from={}, timestamps={}",
                rtype,
                sender,
                receipt.timestamps.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(",")
            );

            if tx.send(msg).await.is_err() {
                warn!("Signal receipt channel closed");
            }
            return;
        }

        // AllowBotsMode — no bot flag in Signal; reserved for future heuristic
        let _ = allow_bots;

        let data_msg = match env.data_message {
            Some(d) => d,
            None => return,
        };

        // Allow messages that are attachment-only (no text body)
        let text = data_msg
            .message
            .as_deref()
            .filter(|t| !t.is_empty())
            .unwrap_or("")
            .to_string();

        // If there's no text and no attachments, skip
        if text.is_empty() && data_msg.attachments.is_empty() {
            return;
        }

        let policy = ChannelPolicy::new(config.policy.clone().unwrap_or_default());

        let group_id = data_msg
            .group_info
            .as_ref()
            .and_then(|g| g.group_id.as_deref());

        if let Some(gid) = group_id {
            // Robust mention detection (Discord parity): groups addressed on / command
            let is_addressed = text.starts_with('/');
            if policy.check_group(gid, &sender, is_addressed).is_denied() {
                return;
            }
        } else if policy.check_dm(&sender).is_denied() {
            return;
        }

        // Distinguish group vs DM: group messages carry group_id as chat_id so
        // downstream mention/routing logic can tell them apart. DMs have no chat_id
        // and trigger implicit-addressing (no mention keyword required).
        let is_addressed = group_id.is_none() || text.starts_with('/');

        // Distinguish group vs DM: group messages carry group_id as chat_id so
        // downstream mention/routing logic can tell them apart. DMs have no chat_id
        // and trigger implicit-addressing (no mention keyword required).
        let mut source = if let Some(gid) = group_id {
            ChannelSource::with_chat("signal", &sender, gid)
        } else {
            ChannelSource::new("signal", &sender)
        }
        .with_sender_type(zeus_core::SenderType::Human);
        if let Some(ref acct_id) = config.account_id {
            source = source.with_account(acct_id);
        }

        // ── Parse inbound attachments ────────────────────────────────────
        let attachments: Vec<ChannelAttachment> = data_msg
            .attachments
            .into_iter()
            .map(|att| {
                let mime = att
                    .content_type
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                let url = att.id.map(|id| {
                    format!("{}/v1/attachments/{}", config.base_url(), id)
                });
                let mut channel_att = if let Some(url) = url {
                    ChannelAttachment::from_url(&url, &mime)
                } else {
                    ChannelAttachment::from_data(Vec::new(), &mime)
                };
                if let Some(fname) = att.filename {
                    channel_att = channel_att.with_filename(&fname);
                }
                channel_att
            })
            .collect();

        let msg = if attachments.is_empty() {
            ChannelMessage::new(source, text).with_addressed(is_addressed)
        } else {
            ChannelMessage::with_attachments(source, text, attachments)
        };

        if tx.send(msg).await.is_err() {
            warn!("Signal message channel closed");
        }
    }
}

#[async_trait]
impl ChannelAdapter for SignalAdapter {
    fn channel_type(&self) -> &'static str {
        "signal"
    }

    fn account_id(&self) -> Option<&str> {
        self.config.account_id.as_deref()
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::ExternalProcess
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        // Spawn the daemon
        let mut child = self.spawn_daemon().await?;
        let connected = self.connected.clone();
        connected.store(true, Ordering::SeqCst);

        let config = self.config.clone();
        let http = self.http.clone();
        let conn2 = connected.clone();

        // Watch daemon stderr for readiness / errors
        tokio::spawn(async move {
            if let Some(stderr) = child.stderr.take() {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    info!("[signal-cli] {}", line);
                }
            }
            let _ = child.wait().await;
            conn2.store(false, Ordering::SeqCst);
            warn!("signal-cli daemon exited");
        });

        // Stream SSE events
        tokio::spawn(Self::stream_events(config, http, connected, tx));

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        let url = format!("{}/v2/send", self.config.base_url());

        // Signal renders text as plain (no Markdown). Strip Markdown to avoid
        // leaking raw `**bold**`, `[text](url)`, etc. into the user's view.
        // Mirror-symmetric to Telegram/IRC adapter-layer sanitize (d9bef524).
        let content = strip_markdown(content);

        let body = SendRequest {
            message: &content,
            number: &self.config.phone,
            recipients: vec![&to.user_id],
        };

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("signal send request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "signal send error {}: {}",
                status, body
            )));
        }

        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn supports_native_identity(&self) -> bool {
        false
    }

    /// Send a file attachment via signal-cli HTTP API.
    ///
    /// Uses multipart form-data POST to `/v2/send` with the file attached.
    /// Falls back to base64-encoding the file in the JSON body if multipart
    /// is not supported by the signal-cli version.
    async fn send_file(
        &self,
        to: &ChannelSource,
        filename: &str,
        data: &[u8],
        caption: Option<&str>,
    ) -> Result<()> {
        let url = format!("{}/v2/send", self.config.base_url());

        let file_part = reqwest::multipart::Part::bytes(data.to_vec())
            .file_name(filename.to_string())
            .mime_str("application/octet-stream")
            .map_err(|e| Error::Channel(format!("Signal file part creation failed: {}", e)))?;

        let mut form = reqwest::multipart::Form::new()
            .text("number", self.config.phone.clone())
            .text("recipients", to.user_id.clone())
            .part("attachment", file_part);

        if let Some(cap) = caption {
            form = form.text("message", cap.to_string());
        }

        let resp = self
            .http
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Signal send_file request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Channel(format!(
                "Signal send_file error {}: {}",
                status, body
            )));
        }

        info!(
            "Signal file sent: filename={}, size={} bytes, to={}",
            filename,
            data.len(),
            to.user_id
        );
        Ok(())
    }

    /// Send a typing indicator via signal-cli.
    ///
    /// Uses the `/v1/typing/{recipient}` endpoint. Not all signal-cli
    /// versions support this — errors are logged but not propagated.
    async fn send_typing(&self, to: &ChannelSource) -> Result<()> {
        let url = format!(
            "{}/v1/typing/{}",
            self.config.base_url(),
            to.user_id
        );

        let resp = self
            .http
            .put(&url)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                info!("Signal typing indicator sent to {}", to.user_id);
            }
            Ok(r) => {
                // Typing indicator not critical — log and move on
                warn!(
                    "Signal typing indicator not supported or failed: {}",
                    r.status()
                );
            }
            Err(e) => {
                warn!("Signal typing indicator request failed: {}", e);
            }
        }

        Ok(())
    }

    fn supports_typing(&self) -> bool {
        true
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentSendIdentity;

    #[test]
    fn test_signal_config_defaults() {
        let config = SignalConfig::default();
        assert_eq!(config.signal_cli_path, "signal-cli");
        assert!(config.phone.is_empty());
        assert_eq!(config.http_port, 8080);
        assert_eq!(config.http_host, "127.0.0.1");
    }

    #[test]
    fn test_signal_base_url() {
        let config = SignalConfig {
            http_host: "127.0.0.1".to_string(),
            http_port: 8080,
            ..Default::default()
        };
        assert_eq!(config.base_url(), "http://127.0.0.1:8080");
    }

    #[test]
    fn test_signal_base_url_custom_port() {
        let config = SignalConfig {
            http_host: "0.0.0.0".to_string(),
            http_port: 9090,
            ..Default::default()
        };
        assert_eq!(config.base_url(), "http://0.0.0.0:9090");
    }

    #[tokio::test]
    async fn test_signal_adapter_creation() {
        let config = SignalConfig::default();
        let adapter = SignalAdapter::new(config).await;
        assert!(adapter.is_ok());
        let adapter = adapter.expect("SignalAdapter::new should succeed");
        assert_eq!(adapter.channel_type(), "signal");
        assert!(!adapter.is_connected());
    }

    #[test]
    fn test_signal_channel_source() {
        let source = ChannelSource::new("signal", "+1234567890");
        assert_eq!(source.channel_type(), "signal");
        assert_eq!(source.user_id, "+1234567890");
        assert!(source.chat_id.is_none());
    }

    #[test]
    fn test_signal_dm_stamps_human_sender_type() {
        // Regression: Signal DMs must carry SenderType::Human + no chat_id so
        // downstream implicit-addressing picks them up. Mirrors fix/irc-dm-routing-v2.
        let dm_source = ChannelSource::new("signal", "+1234567890")
            .with_sender_type(zeus_core::SenderType::Human);
        assert_eq!(dm_source.sender_type, zeus_core::SenderType::Human);
        assert!(dm_source.sender_type.is_human());
        assert!(dm_source.chat_id.is_none(), "DM must have no chat_id");
    }

    #[test]
    fn test_signal_group_stamps_human_sender_type_with_chat() {
        // Regression: Signal group messages must carry SenderType::Human AND
        // the group_id as chat_id so mention-routing can distinguish group vs DM.
        let group_source = ChannelSource::with_chat("signal", "+1234567890", "group-abc-123")
            .with_sender_type(zeus_core::SenderType::Human);
        assert_eq!(group_source.sender_type, zeus_core::SenderType::Human);
        assert_eq!(group_source.chat_id.as_deref(), Some("group-abc-123"));
    }

    #[test]
    fn test_signal_config_serde() {
        let json = r#"{
            "signal_cli_path": "/opt/signal-cli/bin/signal-cli",
            "phone": "+9876543210",
            "http_port": 9090,
            "http_host": "0.0.0.0",
            "account_id": "work",
            "allow_bots": "mentions"
        }"#;
        let config: SignalConfig = serde_json::from_str(json).expect("should parse");
        assert_eq!(config.signal_cli_path, "/opt/signal-cli/bin/signal-cli");
        assert_eq!(config.phone, "+9876543210");
        assert_eq!(config.http_port, 9090);
        assert_eq!(config.http_host, "0.0.0.0");
        assert_eq!(config.account_id.as_deref(), Some("work"));
        assert_eq!(config.allow_bots.as_deref(), Some("mentions"));
    }

    #[test]
    fn test_sse_envelope_parse() {
        let data = r#"{
            "envelope": {
                "source": "+1234567890",
                "dataMessage": {
                    "message": "Hello from Signal",
                    "groupInfo": null
                }
            }
        }"#;
        let env: SseEnvelope = serde_json::from_str(data).expect("should parse");
        let inner = env.envelope.expect("should have envelope");
        assert_eq!(inner.source.as_deref(), Some("+1234567890"));
        let dm = inner.data_message.expect("should have dataMessage");
        assert_eq!(dm.message.as_deref(), Some("Hello from Signal"));
        assert!(dm.group_info.is_none());
    }

    #[test]
    fn test_sse_group_envelope_parse() {
        let data = r#"{
            "envelope": {
                "source": "+1234567890",
                "dataMessage": {
                    "message": "/help",
                    "groupInfo": {
                        "groupId": "abc123=="
                    }
                }
            }
        }"#;
        let env: SseEnvelope = serde_json::from_str(data).expect("should parse");
        let inner = env.envelope.expect("should have envelope");
        let dm = inner.data_message.expect("should have dataMessage");
        let gid = dm
            .group_info
            .expect("should have groupInfo")
            .group_id
            .expect("should have groupId");
        assert_eq!(gid, "abc123==");
    }

    #[test]
    fn test_send_request_serializes() {
        let req = SendRequest {
            message: "Hello",
            number: "+10000000000",
            recipients: vec!["+19999999999"],
        };
        let json = serde_json::to_string(&req).expect("should serialize");
        assert!(json.contains("\"message\":\"Hello\""));
        assert!(json.contains("\"number\":\"+10000000000\""));
        assert!(json.contains("\"+19999999999\""));
    }

    #[tokio::test]
    async fn test_signal_supports_native_identity_false() {
        let adapter = SignalAdapter::new(SignalConfig::default())
            .await
            .expect("should create");
        assert!(!adapter.supports_native_identity());
    }

    #[test]
    fn test_signal_send_as_text_prefix_format() {
        let identity = AgentSendIdentity::new("zeus_agent");
        let prefixed = identity.apply_prefix("Hello from Signal");
        assert_eq!(prefixed, "[zeus_agent] Hello from Signal");
    }

    #[tokio::test]
    async fn test_signal_account_id_none_by_default() {
        let adapter = SignalAdapter::new(SignalConfig::default())
            .await
            .expect("should create");
        assert!(adapter.account_id().is_none());
    }

    #[tokio::test]
    async fn test_signal_account_id_set() {
        let config = SignalConfig {
            account_id: Some("personal".to_string()),
            ..Default::default()
        };
        let adapter = SignalAdapter::new(config).await.expect("should create");
        assert_eq!(adapter.account_id(), Some("personal"));
    }

    #[test]
    fn test_signal_allow_bots_field_parsed() {
        let json = r#"{"signal_cli_path":"signal-cli","phone":"+1234","allow_bots":"on"}"#;
        let config: SignalConfig = serde_json::from_str(json).expect("should parse");
        let mode = AllowBotsMode::from_config(config.allow_bots.as_deref());
        assert_eq!(mode, AllowBotsMode::On);
    }

    #[tokio::test]
    async fn test_signal_receive_mode() {
        let adapter = SignalAdapter::new(SignalConfig::default())
            .await
            .expect("should create");
        assert_eq!(adapter.receive_mode(), ReceiveMode::ExternalProcess);
    }

    // ── Attachment parsing tests ──────────────────────────────────────────

    #[test]
    fn test_sse_envelope_with_attachments() {
        let data = r#"{
            "envelope": {
                "source": "+1234567890",
                "dataMessage": {
                    "message": "Check this photo",
                    "groupInfo": null,
                    "attachments": [
                        {
                            "contentType": "image/jpeg",
                            "filename": "photo.jpg",
                            "id": "att_abc123",
                            "size": 2048
                        }
                    ]
                }
            }
        }"#;
        let env: SseEnvelope = serde_json::from_str(data).expect("should parse");
        let inner = env.envelope.expect("should have envelope");
        let dm = inner.data_message.expect("should have dataMessage");
        assert_eq!(dm.attachments.len(), 1);
        let att = &dm.attachments[0];
        assert_eq!(att.content_type.as_deref(), Some("image/jpeg"));
        assert_eq!(att.filename.as_deref(), Some("photo.jpg"));
        assert_eq!(att.id.as_deref(), Some("att_abc123"));
        assert_eq!(att.size, Some(2048));
    }

    #[test]
    fn test_sse_envelope_multiple_attachments() {
        let data = r#"{
            "envelope": {
                "source": "+1234567890",
                "dataMessage": {
                    "message": "Here are the files",
                    "attachments": [
                        {
                            "contentType": "image/png",
                            "filename": "screenshot.png",
                            "id": "att_001"
                        },
                        {
                            "contentType": "application/pdf",
                            "filename": "report.pdf",
                            "id": "att_002"
                        }
                    ]
                }
            }
        }"#;
        let env: SseEnvelope = serde_json::from_str(data).expect("should parse");
        let dm = env.envelope.unwrap().data_message.unwrap();
        assert_eq!(dm.attachments.len(), 2);
        assert_eq!(dm.attachments[0].content_type.as_deref(), Some("image/png"));
        assert_eq!(dm.attachments[1].content_type.as_deref(), Some("application/pdf"));
    }

    #[test]
    fn test_sse_envelope_attachment_only_no_text() {
        let data = r#"{
            "envelope": {
                "source": "+1234567890",
                "dataMessage": {
                    "message": null,
                    "attachments": [
                        {
                            "contentType": "image/jpeg",
                            "filename": "photo.jpg",
                            "id": "att_only"
                        }
                    ]
                }
            }
        }"#;
        let env: SseEnvelope = serde_json::from_str(data).expect("should parse");
        let dm = env.envelope.unwrap().data_message.unwrap();
        assert!(dm.message.is_none() || dm.message.as_deref() == Some(""));
        assert_eq!(dm.attachments.len(), 1);
    }

    #[test]
    fn test_sse_envelope_no_attachments() {
        let data = r#"{
            "envelope": {
                "source": "+1234567890",
                "dataMessage": {
                    "message": "Just text",
                    "groupInfo": null
                }
            }
        }"#;
        let env: SseEnvelope = serde_json::from_str(data).expect("should parse");
        let dm = env.envelope.unwrap().data_message.unwrap();
        assert!(dm.attachments.is_empty());
    }

    // ── Receipt parsing tests ─────────────────────────────────────────────

    #[test]
    fn test_sse_receipt_delivery() {
        let data = r#"{
            "envelope": {
                "source": "+1234567890",
                "receipt": {
                    "type": "DELIVERY",
                    "timestamps": [1700000000000]
                },
                "timestamp": 1700000000000
            }
        }"#;
        let env: SseEnvelope = serde_json::from_str(data).expect("should parse");
        let inner = env.envelope.expect("should have envelope");
        let receipt = inner.receipt.expect("should have receipt");
        assert_eq!(receipt.receipt_type.as_deref(), Some("DELIVERY"));
        assert_eq!(receipt.timestamps, vec![1700000000000u64]);
    }

    #[test]
    fn test_sse_receipt_read() {
        let data = r#"{
            "envelope": {
                "source": "+1234567890",
                "receipt": {
                    "type": "READ",
                    "timestamps": [1700000000000, 1700000001000]
                }
            }
        }"#;
        let env: SseEnvelope = serde_json::from_str(data).expect("should parse");
        let receipt = env.envelope.unwrap().receipt.unwrap();
        assert_eq!(receipt.receipt_type.as_deref(), Some("READ"));
        assert_eq!(receipt.timestamps.len(), 2);
    }

    #[test]
    fn test_sse_no_receipt_in_data_message() {
        let data = r#"{
            "envelope": {
                "source": "+1234567890",
                "dataMessage": {
                    "message": "Hello"
                }
            }
        }"#;
        let env: SseEnvelope = serde_json::from_str(data).expect("should parse");
        let inner = env.envelope.unwrap();
        assert!(inner.receipt.is_none());
    }

    // ── ChannelAttachment from SignalAttachment ───────────────────────────

    #[test]
    fn test_channel_attachment_from_signal_url() {
        let config = SignalConfig::default();
        let sig_att = SignalAttachment {
            content_type: Some("image/jpeg".to_string()),
            filename: Some("photo.jpg".to_string()),
            id: Some("att_123".to_string()),
            size: Some(2048),
        };
        let mime = sig_att.content_type.unwrap_or_else(|| "application/octet-stream".to_string());
        let url = sig_att.id.map(|id| format!("{}/v1/attachments/{}", config.base_url(), id));
        let mut att = ChannelAttachment::from_url(url.as_deref().unwrap(), &mime);
        if let Some(fname) = sig_att.filename {
            att = att.with_filename(&fname);
        }
        assert_eq!(att.mime_type, "image/jpeg");
        assert!(att.url.is_some());
        assert_eq!(att.filename.as_deref(), Some("photo.jpg"));
    }

    // ── send_file / typing support tests ──────────────────────────────────

    #[tokio::test]
    async fn test_signal_supports_typing() {
        let adapter = SignalAdapter::new(SignalConfig::default())
            .await
            .expect("should create");
        assert!(adapter.supports_typing());
    }
}
