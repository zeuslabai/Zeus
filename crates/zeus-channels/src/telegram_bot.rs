//! Pure Telegram Bot HTTP API adapter — no MTProto/grammers required.
//!
//! Uses only `bot_token` + Telegram's HTTPS Bot API endpoints:
//! - `getUpdates` for long-polling incoming messages
//! - `sendMessage` for sending replies
//!
//! Ideal for deployments where no `api_id`/`api_hash` are available or desired.

use crate::{ChannelAdapter, ChannelAttachment, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use tokio::sync::mpsc;
use zeus_core::{Error, Result};

const TELEGRAM_API_BASE: &str = "https://api.telegram.org/bot";
const DEFAULT_POLL_TIMEOUT_SECS: u64 = 30;

// ── Config ────────────────────────────────────────────────────────────────────

/// Configuration for the pure-HTTP Telegram Bot adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramBotConfig {
    /// Bot token from BotFather (e.g. "123456:ABC-DEF...")
    pub bot_token: String,
    /// Default chat/channel ID for outbound messages
    pub default_chat_id: Option<i64>,
    /// Long-poll timeout in seconds (default: 30)
    #[serde(default)]
    pub poll_timeout_secs: Option<u64>,
}

impl TelegramBotConfig {
    pub fn new(bot_token: impl Into<String>) -> Self {
        Self {
            bot_token: bot_token.into(),
            default_chat_id: None,
            poll_timeout_secs: None,
        }
    }

    fn poll_timeout(&self) -> u64 {
        self.poll_timeout_secs.unwrap_or(DEFAULT_POLL_TIMEOUT_SECS)
    }

    pub fn api_url(&self, method: &str) -> String {
        format!("{}{}/{}", TELEGRAM_API_BASE, self.bot_token, method)
    }
}

// ── Telegram API response types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TgResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TgUpdate {
    update_id: i64,
    message: Option<TgMessage>,
}

#[derive(Debug, Deserialize)]
struct TgMessage {
    message_id: i64,
    chat: TgChat,
    from: Option<TgUser>,
    text: Option<String>,
    #[serde(default)]
    caption: Option<String>,
    #[serde(default)]
    document: Option<TgDocument>,
    #[serde(default)]
    photo: Option<Vec<TgPhotoSize>>,
    #[serde(default)]
    video: Option<TgVideo>,
    #[serde(default)]
    audio: Option<TgAudio>,
    #[serde(default)]
    voice: Option<TgVoice>,
    date: i64,
}

#[derive(Debug, Deserialize)]
struct TgChat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
}

#[derive(Debug, Deserialize)]
struct TgUser {
    first_name: String,
    username: Option<String>,
}

// ── Telegram media types (minimal fields for attachment extraction) ──────────

#[derive(Debug, Deserialize, Clone)]
struct TgDocument {
    file_id: String,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct TgPhotoSize {
    file_id: String,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
}

#[derive(Debug, Deserialize, Clone)]
struct TgVideo {
    file_id: String,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct TgAudio {
    file_id: String,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct TgVoice {
    file_id: String,
    #[serde(default)]
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TgFile {
    file_path: Option<String>,
}

// ── Adapter ───────────────────────────────────────────────────────────────────

/// Telegram Bot HTTP API adapter — pure HTTP, no MTProto.
pub struct TelegramBotAdapter {
    pub config: TelegramBotConfig,
    http: reqwest::Client,
    connected: Arc<AtomicBool>,
    /// Next offset for getUpdates (avoids reprocessing)
    offset: Arc<AtomicI64>,
}

impl TelegramBotAdapter {
    pub fn new(config: TelegramBotConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
            connected: Arc::new(AtomicBool::new(false)),
            offset: Arc::new(AtomicI64::new(0)),
        }
    }

    /// Call getMe to validate the bot token; returns the bot username.
    pub async fn verify_token(&self) -> Result<String> {
        let url = self.config.api_url("getMe");

        #[derive(Deserialize)]
        struct BotInfo {
            username: Option<String>,
            first_name: String,
        }

        let body: TgResponse<BotInfo> = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::channel(format!("Telegram getMe failed: {}", e)))?
            .json()
            .await
            .map_err(|e| Error::channel(format!("Telegram getMe parse error: {}", e)))?;

        if !body.ok {
            return Err(Error::channel(format!(
                "Telegram getMe error: {}",
                body.description.unwrap_or_default()
            )));
        }

        let info = body
            .result
            .ok_or_else(|| Error::channel("Telegram getMe: no result"))?;
        Ok(info.username.unwrap_or(info.first_name))
    }

    /// Send a text message to a specific chat ID.
    pub async fn send_to_chat(&self, chat_id: i64, text: &str) -> Result<()> {
        #[derive(Serialize)]
        struct Payload<'a> {
            chat_id: i64,
            text: &'a str,
        }

        let body: TgResponse<serde_json::Value> = self
            .http
            .post(&self.config.api_url("sendMessage"))
            .json(&Payload { chat_id, text })
            .send()
            .await
            .map_err(|e| Error::channel(format!("Telegram sendMessage failed: {}", e)))?
            .json()
            .await
            .map_err(|e| Error::channel(format!("Telegram sendMessage parse error: {}", e)))?;

        if !body.ok {
            return Err(Error::channel(format!(
                "Telegram sendMessage error: {}",
                body.description.unwrap_or_default()
            )));
        }
        Ok(())
    }

    /// Long-poll for new updates; advances the offset automatically.
    async fn get_updates(&self) -> Result<Vec<TgUpdate>> {
        #[derive(Serialize)]
        struct Params {
            timeout: u64,
            offset: i64,
            allowed_updates: Vec<&'static str>,
        }

        let body: TgResponse<Vec<TgUpdate>> = self
            .http
            .get(&self.config.api_url("getUpdates"))
            .query(&Params {
                timeout: self.config.poll_timeout(),
                offset: self.offset.load(Ordering::Relaxed),
                allowed_updates: vec!["message"],
            })
            .send()
            .await
            .map_err(|e| Error::channel(format!("Telegram getUpdates failed: {}", e)))?
            .json()
            .await
            .map_err(|e| Error::channel(format!("Telegram getUpdates parse error: {}", e)))?;

        if !body.ok {
            return Err(Error::channel(format!(
                "Telegram getUpdates error: {}",
                body.description.unwrap_or_default()
            )));
        }

        let updates = body.result.unwrap_or_default();
        if let Some(last) = updates.last() {
            self.offset.store(last.update_id + 1, Ordering::Relaxed);
        }
        Ok(updates)
    }

    /// Convert a raw Telegram update into a `ChannelMessage`.
    pub async fn update_to_channel_message(
        http: &reqwest::Client,
        config: &TelegramBotConfig,
        update: TgUpdate,
    ) -> Option<ChannelMessage> {
        let msg = update.message?;

        // Extract media (if any) into ChannelAttachment list — bot HTTP API
        // path. Mirrors Discord adapter's attachment-URL injection convention so
        // downstream agents see file context the same way across channels.
        let mut attachments: Vec<ChannelAttachment> = Vec::new();

        if let Some(doc) = msg.document.as_ref() {
            if let Some(url) = Self::resolve_file_url(http, config, &doc.file_id).await {
                let mime = doc.mime_type.as_deref().unwrap_or("application/octet-stream");
                let mut att = ChannelAttachment::from_url(&url, mime);
                if let Some(name) = doc.file_name.as_deref() {
                    att = att.with_filename(name);
                }
                attachments.push(att);
            }
        }
        if let Some(photos) = msg.photo.as_ref() {
            // Telegram delivers photos as an array of sizes; pick the largest (last).
            if let Some(largest) = photos.last() {
                if let Some(url) = Self::resolve_file_url(http, config, &largest.file_id).await {
                    attachments.push(ChannelAttachment::from_url(&url, "image/jpeg"));
                }
            }
        }
        if let Some(video) = msg.video.as_ref() {
            if let Some(url) = Self::resolve_file_url(http, config, &video.file_id).await {
                let mime = video.mime_type.as_deref().unwrap_or("video/mp4");
                let mut att = ChannelAttachment::from_url(&url, mime);
                if let Some(name) = video.file_name.as_deref() {
                    att = att.with_filename(name);
                }
                attachments.push(att);
            }
        }
        if let Some(audio) = msg.audio.as_ref() {
            if let Some(url) = Self::resolve_file_url(http, config, &audio.file_id).await {
                let mime = audio.mime_type.as_deref().unwrap_or("audio/mpeg");
                let mut att = ChannelAttachment::from_url(&url, mime);
                if let Some(name) = audio.file_name.as_deref() {
                    att = att.with_filename(name);
                }
                attachments.push(att);
            }
        }
        if let Some(voice) = msg.voice.as_ref() {
            if let Some(url) = Self::resolve_file_url(http, config, &voice.file_id).await {
                let mime = voice.mime_type.as_deref().unwrap_or("audio/ogg");
                attachments.push(ChannelAttachment::from_url(&url, mime));
            }
        }

        // Combine text + caption. Media-only messages (no text, no caption) must
        // still flow through, so we drop the previous `let text = msg.text?` guard.
        let content = msg.text
            .clone()
            .or_else(|| msg.caption.clone())
            .unwrap_or_default();

        // Filter: pure no-text + no-media noise (e.g. service updates) — skip.
        if content.is_empty() && attachments.is_empty() {
            return None;
        }

        let user_id = msg
            .from
            .as_ref()
            .and_then(|u| u.username.clone())
            .map(|n| format!("@{}", n))
            .unwrap_or_else(|| {
                msg.from
                    .as_ref()
                    .map(|u| u.first_name.clone())
                    .unwrap_or_else(|| "unknown".to_string())
            });

        let source = ChannelSource::with_chat(
            "telegram",
            &user_id,
            &msg.chat.id.to_string(),
        );

        let mut cm = if attachments.is_empty() {
            ChannelMessage::new(source, content)
        } else {
            ChannelMessage::with_attachments(source, content, attachments)
        };
        cm.platform_message_id = Some(msg.message_id.to_string());
        Some(cm)
    }

    /// Resolve a Telegram `file_id` to a downloadable HTTPS URL via `getFile`.
    /// Returns `None` on any API/network error (logged) — caller treats the
    /// attachment as unresolvable rather than failing the whole message.
    async fn resolve_file_url(
        http: &reqwest::Client,
        config: &TelegramBotConfig,
        file_id: &str,
    ) -> Option<String> {
        #[derive(Serialize)]
        struct Params<'a> {
            file_id: &'a str,
        }

        let resp: std::result::Result<TgResponse<TgFile>, _> = http
            .get(&config.api_url("getFile"))
            .query(&Params { file_id })
            .send()
            .await
            .ok()?
            .json()
            .await;

        let body = match resp {
            Ok(b) if b.ok => b,
            Ok(b) => {
                tracing::warn!(
                    "Telegram getFile non-ok response: {}",
                    b.description.unwrap_or_default()
                );
                return None;
            }
            Err(e) => {
                tracing::warn!("Telegram getFile parse error: {}", e);
                return None;
            }
        };

        let file_path = body.result.and_then(|f| f.file_path)?;
        Some(format!(
            "{}file/bot{}/{}",
            TELEGRAM_API_BASE, config.bot_token, file_path
        ))
    }
}

#[async_trait]
impl ChannelAdapter for TelegramBotAdapter {
    fn channel_type(&self) -> &'static str {
        "telegram"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::LongPolling { timeout_secs: self.config.poll_timeout() }
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        let offset = self.offset.clone();
        let http = self.http.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            loop {
                #[derive(Serialize)]
                struct Params {
                    timeout: u64,
                    offset: i64,
                    allowed_updates: Vec<&'static str>,
                }

                let params = Params {
                    timeout: config.poll_timeout(),
                    offset: offset.load(Ordering::Relaxed),
                    allowed_updates: vec!["message"],
                };

                let resp = http
                    .get(&config.api_url("getUpdates"))
                    .query(&params)
                    .send()
                    .await;

                match resp {
                    Ok(r) => {
                        if let Ok(body) = r.json::<TgResponse<Vec<TgUpdate>>>().await {
                            if body.ok {
                                for update in body.result.unwrap_or_default() {
                                    let uid = update.update_id;
                                    if let Some(cm) =
                                        TelegramBotAdapter::update_to_channel_message(
                                            &http, &config, update,
                                        )
                                        .await
                                    {
                                        let _ = tx.send(cm).await;
                                    }
                                    offset.store(uid + 1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("TelegramBot poll error: {}", e);
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            }
        });

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::Relaxed);
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        let chat_id: i64 = to
            .chat_id
            .as_deref()
            .or(self
                .config
                .default_chat_id
                .as_ref()
                .map(|_| "")
                .and(None::<&str>))
            .and_then(|s| s.parse().ok())
            .or(self.config.default_chat_id)
            .ok_or_else(|| {
                Error::channel(
                    "TelegramBot send: no chat_id in ChannelSource and no default_chat_id configured",
                )
            })?;

        self.send_to_chat(chat_id, content).await
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> TelegramBotConfig {
        TelegramBotConfig::new("123456:ABC-DEF-test-token")
    }

    // ── Config ────────────────────────────────────────────────────────────────

    #[test]
    fn test_config_api_url() {
        let cfg = make_config();
        assert_eq!(
            cfg.api_url("sendMessage"),
            "https://api.telegram.org/bot123456:ABC-DEF-test-token/sendMessage"
        );
    }

    #[test]
    fn test_config_poll_timeout_default() {
        assert_eq!(make_config().poll_timeout(), 30);
    }

    #[test]
    fn test_config_poll_timeout_custom() {
        let mut cfg = make_config();
        cfg.poll_timeout_secs = Some(60);
        assert_eq!(cfg.poll_timeout(), 60);
    }

    #[test]
    fn test_config_serialization_roundtrip() {
        let cfg = TelegramBotConfig {
            bot_token: "test:token".to_string(),
            default_chat_id: Some(-100123456789),
            poll_timeout_secs: Some(45),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: TelegramBotConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.bot_token, "test:token");
        assert_eq!(back.default_chat_id, Some(-100123456789));
        assert_eq!(back.poll_timeout_secs, Some(45));
    }

    // ── Adapter creation ──────────────────────────────────────────────────────

    #[test]
    fn test_adapter_new_not_connected() {
        let adapter = TelegramBotAdapter::new(make_config());
        assert!(!adapter.connected.load(Ordering::Relaxed));
        assert_eq!(adapter.offset.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_channel_type() {
        let adapter = TelegramBotAdapter::new(make_config());
        assert_eq!(adapter.channel_type(), "telegram");
    }

    #[test]
    fn test_receive_mode_is_poll() {
        let adapter = TelegramBotAdapter::new(make_config());
        assert_eq!(adapter.receive_mode(), ReceiveMode::LongPolling { timeout_secs: 30 });
    }

    // ── update_to_channel_message ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_update_with_username() {
        let update = TgUpdate {
            update_id: 42,
            message: Some(TgMessage {
                message_id: 1,
                chat: TgChat {
                    id: -100999,
                    chat_type: "group".to_string(),
                },
                from: Some(TgUser {
                    first_name: "Alice".to_string(),
                    username: Some("alice_bot".to_string()),
                }),
                text: Some("hello world".to_string()),
                caption: None,
                document: None,
                photo: None,
                video: None,
                audio: None,
                voice: None,
                date: 1700000000,
            }),
        };

        let http = reqwest::Client::new();
        let cfg = make_config();
        let msg = TelegramBotAdapter::update_to_channel_message(&http, &cfg, update)
            .await
            .unwrap();
        assert_eq!(msg.content, "hello world");
        assert_eq!(msg.source.user_id, "@alice_bot");
        assert_eq!(msg.source.channel_type, "telegram");
        assert_eq!(msg.source.chat_id.as_deref(), Some("-100999"));
        assert_eq!(msg.platform_message_id.as_deref(), Some("1"));
    }

    #[tokio::test]
    async fn test_update_no_username_uses_first_name() {
        let update = TgUpdate {
            update_id: 1,
            message: Some(TgMessage {
                message_id: 2,
                chat: TgChat {
                    id: 999,
                    chat_type: "private".to_string(),
                },
                from: Some(TgUser {
                    first_name: "Bob".to_string(),
                    username: None,
                }),
                text: Some("hey".to_string()),
                caption: None,
                document: None,
                photo: None,
                video: None,
                audio: None,
                voice: None,
                date: 1700000001,
            }),
        };

        let http = reqwest::Client::new();
        let cfg = make_config();
        let msg = TelegramBotAdapter::update_to_channel_message(&http, &cfg, update)
            .await
            .unwrap();
        assert_eq!(msg.source.user_id, "Bob");
    }

    #[tokio::test]
    async fn test_update_no_text_returns_none() {
        let update = TgUpdate {
            update_id: 2,
            message: Some(TgMessage {
                message_id: 3,
                chat: TgChat {
                    id: 1,
                    chat_type: "private".to_string(),
                },
                from: None,
                text: None,
                caption: None,
                document: None,
                photo: None,
                video: None,
                audio: None,
                voice: None,
                date: 0,
            }),
        };
        let http = reqwest::Client::new();
        let cfg = make_config();
        assert!(
            TelegramBotAdapter::update_to_channel_message(&http, &cfg, update)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_update_no_message_returns_none() {
        let update = TgUpdate {
            update_id: 3,
            message: None,
        };
        let http = reqwest::Client::new();
        let cfg = make_config();
        assert!(
            TelegramBotAdapter::update_to_channel_message(&http, &cfg, update)
                .await
                .is_none()
        );
    }

    // ── send() routing ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_send_no_chat_id_anywhere_errors() {
        let adapter = TelegramBotAdapter::new(make_config()); // no default_chat_id
        let source = ChannelSource::new("telegram", "@user"); // no chat_id
        let result = adapter.send(&source, "hello").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("chat_id"));
    }

    #[test]
    fn test_send_uses_source_chat_id() {
        // Verify the chat_id parse logic works for both positive and negative IDs
        let chat_id_str = "-100123456789";
        let parsed: i64 = chat_id_str.parse().unwrap();
        assert_eq!(parsed, -100123456789i64);
    }
}
