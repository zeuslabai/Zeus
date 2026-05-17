//! Channel relay — background polling + tmux forwarding for Claude Code sessions.
//!
//! Polls messaging channels (Telegram, Discord, Slack, etc.) for incoming messages
//! and forwards them into the active Claude Code tmux session via `tmux send-keys`.
//!
//! This is the mechanism that allows agents to receive commands from coordinators
//! and users via Telegram/Discord/etc. without leaving their Claude Code session.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::process::Command;
use zeus_core::{Result, ToolSchema};

const DISCORD_API: &str = "https://discord.com/api/v10";

/// Simple base64 decode (standard alphabet, no padding required).
fn base64_decode(input: &str) -> Vec<u8> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &b in input.as_bytes() {
        if b == b'=' {
            break;
        }
        let val = TABLE.iter().position(|&c| c == b);
        if let Some(v) = val {
            buf = (buf << 6) | v as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
                buf &= (1 << bits) - 1;
            }
        }
    }
    out
}

// Simple logging macros (zeus-talos doesn't depend on tracing)
macro_rules! relay_info { ($($arg:tt)*) => { eprintln!("[zeus-relay] {}", format!($($arg)*)); }; }
macro_rules! relay_debug { ($($arg:tt)*) => { #[cfg(debug_assertions)] eprintln!("[zeus-relay:debug] {}", format!($($arg)*)); }; }
macro_rules! relay_warn { ($($arg:tt)*) => { eprintln!("[zeus-relay:warn] {}", format!($($arg)*)); }; }

// ---------------------------------------------------------------------------
// Incoming message from any channel
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RelayMessage {
    pub channel: String, // "telegram", "discord", "slack", etc.
    pub sender: String,  // Display name
    pub username: Option<String>,
    pub text: String,
    pub chat_id: i64,
    pub message_id: i64,
    pub timestamp: i64,
    pub file_path: Option<String>,
    pub file_type: Option<String>,
}

// ---------------------------------------------------------------------------
// Tmux session detection + forwarding
// ---------------------------------------------------------------------------

/// Detect the current tmux session name (if running inside tmux).
fn detect_tmux_session() -> Option<String> {
    if std::env::var("TMUX").is_err() {
        return None;
    }
    let output = std::process::Command::new("tmux")
        .args(["display-message", "-p", "#S"])
        .output()
        .ok()?;
    if output.status.success() {
        String::from_utf8(output.stdout)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    } else {
        None
    }
}

/// Auto-detect the active tmux session by listing sessions and picking the first attached one.
/// Works even when the relay process was NOT launched from inside tmux.
async fn detect_active_tmux_session() -> Option<String> {
    let tmux = if std::path::Path::new("/opt/homebrew/bin/tmux").exists() {
        "/opt/homebrew/bin/tmux"
    } else if std::path::Path::new("/usr/local/bin/tmux").exists() {
        "/usr/local/bin/tmux"
    } else {
        "tmux"
    };

    let output = Command::new(tmux)
        .args(["list-sessions", "-F", "#{session_name}:#{session_attached}"])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut first_session = None;

    for line in stdout.lines() {
        if let Some((name, attached)) = line.rsplit_once(':') {
            if first_session.is_none() {
                first_session = Some(name.to_string());
            }
            if attached == "1" {
                return Some(name.to_string());
            }
        }
    }

    first_session
}

/// Forward a message into a tmux session by typing it + pressing Enter.
async fn forward_to_tmux(session: &str, text: &str) {
    let escaped = text.replace('\'', "'\\''");
    let escaped_session = session.replace('\'', "'\\''");
    // Type the message (quote both session name and text to prevent injection)
    let type_cmd = format!("tmux send-keys -t '{}' -l '{}'", escaped_session, escaped);
    let _ = Command::new("sh").arg("-c").arg(&type_cmd).output().await;
    // Small delay so Claude Code can process
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    // Press Enter
    let enter_cmd = format!("tmux send-keys -t '{}' C-m", escaped_session);
    let _ = Command::new("sh").arg("-c").arg(&enter_cmd).output().await;
    relay_debug!("Forwarded to tmux {}: {}", session, text);
}

/// Download a Telegram file by file_id and save to /tmp/telegram_files/.
async fn download_telegram_file(
    client: &reqwest::Client,
    tg_api: &str,
    file_id: &str,
    file_type: &str,
    original_name: Option<&str>,
) -> std::result::Result<String, String> {
    let resp = client
        .get(format!("{}/getFile", tg_api))
        .query(&[("file_id", file_id)])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("getFile failed: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("parse error: {}", e))?;
    let remote_path = body["result"]["file_path"]
        .as_str()
        .ok_or("no file_path in response")?;

    let token = tg_api
        .strip_prefix("https://api.telegram.org/bot")
        .unwrap_or("");
    let url = format!("https://api.telegram.org/file/bot{}/{}", token, remote_path);

    let bytes = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("download failed: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("read bytes failed: {}", e))?;

    let dir = "/tmp/telegram_files";
    let _ = std::fs::create_dir_all(dir);

    let ext = remote_path.rsplit('.').next().unwrap_or(file_type);
    let filename = if let Some(name) = original_name {
        name.to_string()
    } else {
        format!(
            "{}_{}.{}",
            file_type,
            chrono::Utc::now().format("%H%M%S"),
            ext
        )
    };
    let path = format!("{}/{}", dir, filename);
    std::fs::write(&path, &bytes).map_err(|e| format!("write failed: {}", e))?;
    Ok(path)
}

/// Download a Discord attachment by direct CDN URL and save to /tmp/discord_files/.
async fn download_discord_file(
    client: &reqwest::Client,
    url: &str,
    filename: &str,
) -> std::result::Result<String, String> {
    let bytes = client
        .get(url)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("download failed: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("read bytes failed: {}", e))?;

    let dir = "/tmp/discord_files";
    let _ = std::fs::create_dir_all(dir);

    let path = format!("{}/{}", dir, filename);
    std::fs::write(&path, &bytes).map_err(|e| format!("write failed: {}", e))?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// TelegramRelay — background poller
// ---------------------------------------------------------------------------

pub struct TelegramRelay {
    client: reqwest::Client,
    messages: Arc<Mutex<VecDeque<RelayMessage>>>,
    running: Arc<AtomicBool>,
    last_update_id: Arc<AtomicI64>,
    last_poll: Arc<Mutex<Option<chrono::DateTime<chrono::Utc>>>>,
    poll_handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    allowed_users: Vec<String>,
    max_queue: usize,
    target_session: Option<String>,
}

impl Default for TelegramRelay {
    fn default() -> Self {
        Self::new()
    }
}

impl TelegramRelay {
    pub fn new() -> Self {
        let allowed = std::env::var("TELEGRAM_ALLOWED_USERS")
            .unwrap_or_else(|_| "mpalenciac".to_string())
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();

        Self {
            client: reqwest::Client::new(),
            messages: Arc::new(Mutex::new(VecDeque::new())),
            running: Arc::new(AtomicBool::new(false)),
            last_update_id: Arc::new(AtomicI64::new(0)),
            last_poll: Arc::new(Mutex::new(None)),
            poll_handle: tokio::sync::Mutex::new(None),
            allowed_users: allowed,
            max_queue: 100,
            target_session: detect_tmux_session(),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn queued_count(&self) -> usize {
        self.messages.lock().unwrap().len()
    }

    pub fn last_poll_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        *self.last_poll.lock().unwrap()
    }

    /// Drain up to `limit` messages from the queue.
    pub fn drain_messages(&self, limit: usize) -> Vec<RelayMessage> {
        let mut queue = self.messages.lock().unwrap();
        let n = limit.min(queue.len());
        queue.drain(..n).collect()
    }

    /// Start the background polling loop.
    pub async fn start(&self) -> std::result::Result<(), String> {
        if self.is_running() {
            return Err("Relay is already running".to_string());
        }

        let bot_token = std::env::var("TELEGRAM_BOT_TOKEN")
            .map_err(|_| "TELEGRAM_BOT_TOKEN not set".to_string())?;

        self.running.store(true, Ordering::SeqCst);

        let running = self.running.clone();
        let messages = self.messages.clone();
        let last_update_id = self.last_update_id.clone();
        let last_poll = self.last_poll.clone();
        let client = self.client.clone();
        let tg_api = format!("https://api.telegram.org/bot{}", bot_token);
        let max_queue = self.max_queue;
        let allowed_users = self.allowed_users.clone();
        let target_session = self
            .target_session
            .clone()
            .or_else(|| std::env::var("TELEGRAM_RELAY_SESSION").ok());
        let target_session = match target_session {
            Some(s) => s,
            None => detect_active_tmux_session()
                .await
                .unwrap_or_else(|| "zeus-0".to_string()),
        };

        relay_info!("Telegram relay starting (session: {})", target_session);

        let handle = tokio::spawn(async move {
            let mut seen: HashSet<i64> = HashSet::new();

            while running.load(Ordering::SeqCst) {
                let offset = last_update_id.load(Ordering::SeqCst) + 1;

                let poll_result = client
                    .post(format!("{}/getUpdates", tg_api))
                    .json(&json!({
                        "offset": offset,
                        "timeout": 2,
                        "allowed_updates": ["message", "callback_query"]
                    }))
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await;

                // Update last poll time
                {
                    let mut lp = last_poll.lock().unwrap();
                    *lp = Some(chrono::Utc::now());
                }

                match poll_result {
                    Ok(response) => {
                        if let Ok(body) = response.json::<Value>().await
                            && body["ok"].as_bool() == Some(true)
                            && let Some(updates) = body["result"].as_array()
                        {
                            for update in updates {
                                let uid = update["update_id"].as_i64().unwrap_or(0);
                                last_update_id.store(uid, Ordering::SeqCst);

                                if !seen.insert(uid) {
                                    continue;
                                }
                                if seen.len() > 1000 {
                                    seen.clear();
                                }

                                if let Some(msg) = update.get("message") {
                                    let chat_id = msg["chat"]["id"].as_i64().unwrap_or(0);
                                    let _user_id = msg["from"]["id"].as_i64().unwrap_or(0);
                                    let msg_id = msg["message_id"].as_i64().unwrap_or(0);
                                    let username =
                                        msg["from"]["username"].as_str().map(|s| s.to_string());
                                    let first_name = msg["from"]["first_name"]
                                        .as_str()
                                        .unwrap_or("User")
                                        .to_string();
                                    let timestamp = msg["date"].as_i64().unwrap_or(0);

                                    // Check allowed users
                                    if !allowed_users.is_empty() {
                                        let uname = username
                                            .as_deref()
                                            .map(|s| s.to_lowercase())
                                            .unwrap_or_default();
                                        if !allowed_users.iter().any(|u| u == &uname) {
                                            relay_debug!("Ignoring unauthorized user: {}", uname);
                                            continue;
                                        }
                                    }

                                    let text = msg["text"].as_str().unwrap_or("").to_string();

                                    // Handle text messages
                                    if !text.is_empty() {
                                        let incoming = RelayMessage {
                                            channel: "telegram".to_string(),
                                            sender: first_name.clone(),
                                            username: username.clone(),
                                            text: text.clone(),
                                            chat_id,
                                            message_id: msg_id,
                                            timestamp,
                                            file_path: None,
                                            file_type: None,
                                        };
                                        {
                                            let mut q = messages.lock().unwrap();
                                            if q.len() >= max_queue {
                                                q.pop_front();
                                            }
                                            q.push_back(incoming);
                                        }
                                        let session = target_session.clone();
                                        let fwd =
                                            format!("(Telegram from {}) {}", first_name, text);
                                        tokio::spawn(async move {
                                            forward_to_tmux(&session, &fwd).await;
                                        });
                                    }

                                    // Handle voice messages (transcription via Whisper)
                                    if let Some(voice) = msg.get("voice") {
                                        let file_id = voice["file_id"].as_str().unwrap_or("");
                                        let duration = voice["duration"].as_i64().unwrap_or(0);
                                        if file_id.is_empty() {
                                            continue;
                                        }

                                        let whisper_url =
                                            std::env::var("ZEUS_WHISPER_URL").unwrap_or_default();

                                        let voice_text = if !whisper_url.is_empty() {
                                            match download_telegram_file(
                                                &client, &tg_api, file_id, "voice", None,
                                            )
                                            .await
                                            {
                                                Ok(path) => {
                                                    match transcribe_voice(
                                                        &client,
                                                        &whisper_url,
                                                        &path,
                                                    )
                                                    .await
                                                    {
                                                        Ok(t) => {
                                                            format!("[Voice {}s] {}", duration, t)
                                                        }
                                                        Err(e) => format!(
                                                            "[Voice {}s - transcription failed: {}]",
                                                            duration, e
                                                        ),
                                                    }
                                                }
                                                Err(e) => format!(
                                                    "[Voice {}s - download failed: {}]",
                                                    duration, e
                                                ),
                                            }
                                        } else {
                                            format!("[Voice {}s - no whisper configured]", duration)
                                        };

                                        let incoming = RelayMessage {
                                            channel: "telegram".to_string(),
                                            sender: first_name.clone(),
                                            username: username.clone(),
                                            text: voice_text.clone(),
                                            chat_id,
                                            message_id: msg_id,
                                            timestamp,
                                            file_path: None,
                                            file_type: Some("voice".to_string()),
                                        };
                                        {
                                            let mut q = messages.lock().unwrap();
                                            if q.len() >= max_queue {
                                                q.pop_front();
                                            }
                                            q.push_back(incoming);
                                        }
                                        let session = target_session.clone();
                                        let fwd = format!(
                                            "(Telegram from {}) {}",
                                            first_name, voice_text
                                        );
                                        tokio::spawn(async move {
                                            forward_to_tmux(&session, &fwd).await;
                                        });
                                    }

                                    // Handle photos
                                    if let Some(photos) =
                                        msg.get("photo").and_then(|p| p.as_array())
                                        && let Some(largest) = photos.last()
                                    {
                                        let file_id = largest["file_id"].as_str().unwrap_or("");
                                        if !file_id.is_empty() {
                                            let caption =
                                                msg["caption"].as_str().unwrap_or("").to_string();
                                            match download_telegram_file(
                                                &client, &tg_api, file_id, "photo", None,
                                            )
                                            .await
                                            {
                                                Ok(path) => {
                                                    let photo_text = if caption.is_empty() {
                                                        format!("{} [Photo]", path)
                                                    } else {
                                                        format!("{} [Photo] {}", path, caption)
                                                    };
                                                    let incoming = RelayMessage {
                                                        channel: "telegram".to_string(),
                                                        sender: first_name.clone(),
                                                        username: username.clone(),
                                                        text: photo_text.clone(),
                                                        chat_id,
                                                        message_id: msg_id,
                                                        timestamp,
                                                        file_path: Some(path),
                                                        file_type: Some("photo".to_string()),
                                                    };
                                                    {
                                                        let mut q = messages.lock().unwrap();
                                                        if q.len() >= max_queue {
                                                            q.pop_front();
                                                        }
                                                        q.push_back(incoming);
                                                    }
                                                    let session = target_session.clone();
                                                    let fwd = format!(
                                                        "(Telegram file from {}) {}",
                                                        first_name, photo_text
                                                    );
                                                    tokio::spawn(async move {
                                                        forward_to_tmux(&session, &fwd).await;
                                                    });
                                                }
                                                Err(e) => {
                                                    relay_warn!("Photo download failed: {}", e);
                                                }
                                            }
                                        }
                                    }

                                    // Handle documents
                                    if let Some(doc) = msg.get("document") {
                                        let file_id = doc["file_id"].as_str().unwrap_or("");
                                        let file_name = doc["file_name"].as_str();
                                        if !file_id.is_empty() {
                                            let caption =
                                                msg["caption"].as_str().unwrap_or("").to_string();
                                            match download_telegram_file(
                                                &client, &tg_api, file_id, "document", file_name,
                                            )
                                            .await
                                            {
                                                Ok(path) => {
                                                    let doc_text = if caption.is_empty() {
                                                        format!("{} [Document]", path)
                                                    } else {
                                                        format!("{} [Document] {}", path, caption)
                                                    };
                                                    let incoming = RelayMessage {
                                                        channel: "telegram".to_string(),
                                                        sender: first_name.clone(),
                                                        username: username.clone(),
                                                        text: doc_text.clone(),
                                                        chat_id,
                                                        message_id: msg_id,
                                                        timestamp,
                                                        file_path: Some(path),
                                                        file_type: Some("document".to_string()),
                                                    };
                                                    {
                                                        let mut q = messages.lock().unwrap();
                                                        if q.len() >= max_queue {
                                                            q.pop_front();
                                                        }
                                                        q.push_back(incoming);
                                                    }
                                                    let session = target_session.clone();
                                                    let fwd = format!(
                                                        "(Telegram file from {}) {}",
                                                        first_name, doc_text
                                                    );
                                                    tokio::spawn(async move {
                                                        forward_to_tmux(&session, &fwd).await;
                                                    });
                                                }
                                                Err(e) => {
                                                    relay_warn!("Document download failed: {}", e);
                                                }
                                            }
                                        }
                                    }
                                }

                                // Handle callback queries (inline button presses)
                                if let Some(cb) = update.get("callback_query") {
                                    let data = cb["data"].as_str().unwrap_or("");
                                    let from = cb["from"]["first_name"].as_str().unwrap_or("User");
                                    if !data.is_empty() {
                                        let session = target_session.clone();
                                        let fwd =
                                            format!("(Telegram callback from {}) {}", from, data);
                                        tokio::spawn(async move {
                                            forward_to_tmux(&session, &fwd).await;
                                        });
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        relay_debug!("Poll error: {} — retrying in 5s", e);
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }

                // Short sleep between polls
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            relay_info!("Telegram relay stopped");
        });

        *self.poll_handle.lock().await = Some(handle);
        Ok(())
    }

    /// Stop the relay.
    pub async fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.poll_handle.lock().await.take() {
            handle.abort();
        }
    }
}

/// Transcribe a voice file via Whisper endpoint.
/// Supports both whisper.cpp servers (URL ending in /inference) and
/// OpenAI-compatible APIs (appends /v1/audio/transcriptions if no path given).
/// OGG files are auto-converted to WAV via ffmpeg before sending.
async fn transcribe_voice(
    client: &reqwest::Client,
    whisper_url: &str,
    file_path: &str,
) -> std::result::Result<String, String> {
    // Convert OGG/OPUS to WAV if needed (whisper.cpp doesn't accept OGG)
    let (actual_path, _tmp_wav) = if file_path.ends_with(".ogg") || file_path.ends_with(".opus") {
        let wav_path = format!(
            "{}.wav",
            file_path.trim_end_matches(".ogg").trim_end_matches(".opus")
        );
        let status = Command::new("ffmpeg")
            .args(["-i", file_path, "-ar", "16000", "-ac", "1", "-y", &wav_path])
            .output()
            .await
            .map_err(|e| format!("ffmpeg not found: {}", e))?;
        if !status.status.success() {
            return Err(format!(
                "ffmpeg conversion failed: {}",
                String::from_utf8_lossy(&status.stderr)
            ));
        }
        (wav_path.clone(), Some(wav_path))
    } else {
        (file_path.to_string(), None)
    };

    let bytes = std::fs::read(&actual_path).map_err(|e| format!("read file: {}", e))?;
    let filename = std::path::Path::new(&actual_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio.wav")
        .to_string();

    let mime = if filename.ends_with(".wav") {
        "audio/wav"
    } else {
        "audio/ogg"
    };
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(filename)
        .mime_str(mime)
        .map_err(|e| format!("mime: {}", e))?;
    let form = reqwest::multipart::Form::new().part("file", part);

    // Use URL as-is — set ZEUS_WHISPER_URL to the full endpoint URL:
    // whisper.cpp: http://localhost:8080/inference
    // OpenAI-compat: https://api.openai.com/v1/audio/transcriptions
    let url = whisper_url.trim_end_matches('/').to_string();

    let resp = client
        .post(&url)
        .multipart(form)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("whisper request: {}", e))?;

    let body: Value = resp.json().await.map_err(|e| format!("parse: {}", e))?;
    body["text"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "no text in whisper response".to_string())
}

// ---------------------------------------------------------------------------
// DiscordRelay — background poller
// ---------------------------------------------------------------------------

pub struct DiscordRelay {
    client: reqwest::Client,
    messages: Arc<Mutex<VecDeque<RelayMessage>>>,
    running: Arc<AtomicBool>,
    last_message_ids: Arc<Mutex<HashMap<String, String>>>, // per-channel high-water mark
    last_poll: Arc<Mutex<Option<chrono::DateTime<chrono::Utc>>>>,
    poll_handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    allowed_users: Vec<String>,
    channel_ids: Vec<String>,
    max_queue: usize,
    target_session: Option<String>,
}

impl Default for DiscordRelay {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscordRelay {
    pub fn new() -> Self {
        let allowed = std::env::var("DISCORD_ALLOWED_USERS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();

        // Read channel IDs from config.toml (SSoT), env var as last resort
        let channel_ids: Vec<String> = {
            // Primary: read from config.toml bindings (channel_id fields)
            let from_config: Vec<String> = zeus_core::Config::load()
                .ok()
                .map(|cfg| {
                    cfg.bindings.iter()
                        .filter(|b| b.channel_id.is_some())
                        .filter_map(|b| b.channel_id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if !from_config.is_empty() {
                from_config
            } else {
                // Fallback: env var (legacy, should be removed eventually)
                std::env::var("DISCORD_RELAY_CHANNEL_IDS")
                    .unwrap_or_default()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
        };

        Self {
            client: reqwest::Client::new(),
            messages: Arc::new(Mutex::new(VecDeque::new())),
            running: Arc::new(AtomicBool::new(false)),
            last_message_ids: Arc::new(Mutex::new(HashMap::new())),
            last_poll: Arc::new(Mutex::new(None)),
            poll_handle: tokio::sync::Mutex::new(None),
            allowed_users: allowed,
            channel_ids,
            max_queue: 100,
            target_session: detect_tmux_session(),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn queued_count(&self) -> usize {
        self.messages.lock().unwrap().len()
    }

    pub fn last_poll_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        *self.last_poll.lock().unwrap()
    }

    pub fn channel_count(&self) -> usize {
        self.channel_ids.len()
    }

    /// Drain up to `limit` messages from the queue.
    pub fn drain_messages(&self, limit: usize) -> Vec<RelayMessage> {
        let mut queue = self.messages.lock().unwrap();
        let n = limit.min(queue.len());
        queue.drain(..n).collect()
    }

    /// Start the background polling loop.
    pub async fn start(&self) -> std::result::Result<(), String> {
        if self.is_running() {
            return Err("Discord relay is already running".to_string());
        }

        let bot_token = zeus_core::resolve_discord_token()
            .ok_or_else(|| "No Discord bot token found in config.toml or DISCORD_BOT_TOKEN env var".to_string())?;

        if self.channel_ids.is_empty() {
            return Err("DISCORD_RELAY_CHANNEL_IDS not set or empty".to_string());
        }

        self.running.store(true, Ordering::SeqCst);

        let running = self.running.clone();
        let messages = self.messages.clone();
        let last_message_ids = self.last_message_ids.clone();
        let last_poll = self.last_poll.clone();
        let client = self.client.clone();
        let max_queue = self.max_queue;
        let allowed_users = self.allowed_users.clone();
        let channel_ids = self.channel_ids.clone();
        let target_session = self
            .target_session
            .clone()
            .or_else(|| std::env::var("DISCORD_RELAY_SESSION").ok())
            .or_else(|| std::env::var("TELEGRAM_RELAY_SESSION").ok());
        let target_session = match target_session {
            Some(s) => s,
            None => detect_active_tmux_session()
                .await
                .unwrap_or_else(|| "zeus-0".to_string()),
        };

        relay_info!(
            "Discord relay starting (session: {}, channels: {})",
            target_session,
            channel_ids.len()
        );

        let handle = tokio::spawn(async move {
            // Prime: fetch 1 message per channel to set high-water marks
            for ch_id in &channel_ids {
                let url = format!("{}/channels/{}/messages?limit=1", DISCORD_API, ch_id);
                if let Ok(resp) = client
                    .get(&url)
                    .header("Authorization", format!("Bot {}", bot_token))
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await
                    && let Ok(msgs) = resp.json::<Vec<Value>>().await
                    && let Some(first) = msgs.first()
                    && let Some(id) = first["id"].as_str()
                {
                    let mut ids = last_message_ids.lock().unwrap();
                    ids.insert(ch_id.clone(), id.to_string());
                    relay_debug!("Discord primed channel {} at message {}", ch_id, id);
                }
            }

            while running.load(Ordering::SeqCst) {
                for ch_id in &channel_ids {
                    let after = {
                        let ids = last_message_ids.lock().unwrap();
                        ids.get(ch_id).cloned()
                    };

                    let url = if let Some(ref after_id) = after {
                        format!(
                            "{}/channels/{}/messages?limit=50&after={}",
                            DISCORD_API, ch_id, after_id
                        )
                    } else {
                        format!("{}/channels/{}/messages?limit=1", DISCORD_API, ch_id)
                    };

                    let poll_result = client
                        .get(&url)
                        .header("Authorization", format!("Bot {}", bot_token))
                        .timeout(std::time::Duration::from_secs(10))
                        .send()
                        .await;

                    match poll_result {
                        Ok(response) => {
                            if let Ok(mut msgs) = response.json::<Vec<Value>>().await {
                                // Discord returns newest first — reverse for chronological order
                                msgs.reverse();

                                for msg in &msgs {
                                    // Update high-water mark
                                    if let Some(msg_id) = msg["id"].as_str() {
                                        let mut ids = last_message_ids.lock().unwrap();
                                        ids.insert(ch_id.clone(), msg_id.to_string());
                                    }

                                    // Decode our bot ID from the token for self-echo detection
                                    let our_bot_id_token = bot_token.split('.').next().unwrap_or("");
                                    let our_decoded_id =
                                        String::from_utf8(base64_decode(our_bot_id_token))
                                            .unwrap_or_default();

                                    let is_bot_author = msg["author"]["bot"].as_bool() == Some(true);
                                    let author_id = msg["author"]["id"].as_str().unwrap_or("");

                                    // Skip our own bot messages (self-echo prevention)
                                    if is_bot_author && author_id == our_decoded_id {
                                        continue;
                                    }

                                    let username = msg["author"]["username"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string();
                                    let display_name = msg["author"]["global_name"]
                                        .as_str()
                                        .or_else(|| msg["author"]["username"].as_str())
                                        .unwrap_or("User")
                                        .to_string();

                                    // Check allowed users — bots bypass this filter entirely
                                    // (agent-to-agent comms must work, self-echo already blocked above)
                                    if !allowed_users.is_empty()
                                        && !is_bot_author
                                        && !allowed_users
                                            .iter()
                                            .any(|u| u == &username.to_lowercase())
                                    {
                                        relay_debug!(
                                            "Ignoring unauthorized Discord user: {}",
                                            username
                                        );
                                        continue;
                                    }

                                    let content = msg["content"].as_str().unwrap_or("").to_string();
                                    let msg_id_str = msg["id"].as_str().unwrap_or("0");
                                    // Truncate snowflake to i64 (informational only)
                                    let msg_id_i64 = msg_id_str.parse::<i64>().unwrap_or(0);
                                    let ch_id_i64 = ch_id.parse::<i64>().unwrap_or(0);
                                    let timestamp = msg["timestamp"]
                                        .as_str()
                                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                                        .map(|dt| dt.timestamp())
                                        .unwrap_or(0);

                                    // Handle text content
                                    if !content.is_empty() {
                                        let incoming = RelayMessage {
                                            channel: "discord".to_string(),
                                            sender: display_name.clone(),
                                            username: Some(username.clone()),
                                            text: content.clone(),
                                            chat_id: ch_id_i64,
                                            message_id: msg_id_i64,
                                            timestamp,
                                            file_path: None,
                                            file_type: None,
                                        };
                                        {
                                            let mut q = messages.lock().unwrap();
                                            if q.len() >= max_queue {
                                                q.pop_front();
                                            }
                                            q.push_back(incoming);
                                        }
                                        let session = target_session.clone();
                                        let fwd =
                                            format!("(Discord from {}) {}", display_name, content);
                                        tokio::spawn(async move {
                                            forward_to_tmux(&session, &fwd).await;
                                        });
                                    }

                                    // Handle attachments
                                    if let Some(attachments) = msg["attachments"].as_array() {
                                        for att in attachments {
                                            let att_url = att["url"].as_str().unwrap_or("");
                                            let att_name =
                                                att["filename"].as_str().unwrap_or("file");
                                            let att_type =
                                                att["content_type"].as_str().unwrap_or("unknown");

                                            if att_url.is_empty() {
                                                continue;
                                            }

                                            // Prefix filename with message_id to avoid collisions
                                            let safe_name = format!("{}_{}", msg_id_str, att_name);

                                            match download_discord_file(
                                                &client, att_url, &safe_name,
                                            )
                                            .await
                                            {
                                                Ok(path) => {
                                                    let file_text = format!(
                                                        "{} [{}] {}",
                                                        path, att_type, att_name
                                                    );
                                                    let incoming = RelayMessage {
                                                        channel: "discord".to_string(),
                                                        sender: display_name.clone(),
                                                        username: Some(username.clone()),
                                                        text: file_text.clone(),
                                                        chat_id: ch_id_i64,
                                                        message_id: msg_id_i64,
                                                        timestamp,
                                                        file_path: Some(path),
                                                        file_type: Some(att_type.to_string()),
                                                    };
                                                    {
                                                        let mut q = messages.lock().unwrap();
                                                        if q.len() >= max_queue {
                                                            q.pop_front();
                                                        }
                                                        q.push_back(incoming);
                                                    }
                                                    let session = target_session.clone();
                                                    let fwd = format!(
                                                        "(Discord file from {}) {}",
                                                        display_name, file_text
                                                    );
                                                    tokio::spawn(async move {
                                                        forward_to_tmux(&session, &fwd).await;
                                                    });
                                                }
                                                Err(e) => {
                                                    relay_warn!(
                                                        "Discord attachment download failed: {}",
                                                        e
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            relay_debug!("Discord poll error: {} — retrying", e);
                        }
                    }
                }

                // Update last poll time
                {
                    let mut lp = last_poll.lock().unwrap();
                    *lp = Some(chrono::Utc::now());
                }

                // 2-second poll interval
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            relay_info!("Discord relay stopped");
        });

        *self.poll_handle.lock().await = Some(handle);
        Ok(())
    }

    /// Stop the relay.
    pub async fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.poll_handle.lock().await.take() {
            handle.abort();
        }
    }
}

// ---------------------------------------------------------------------------
// Global relay instances (lazy-initialized)
// ---------------------------------------------------------------------------

use std::sync::OnceLock;
static TELEGRAM_RELAY: OnceLock<TelegramRelay> = OnceLock::new();
static DISCORD_RELAY: OnceLock<DiscordRelay> = OnceLock::new();

fn get_relay() -> &'static TelegramRelay {
    TELEGRAM_RELAY.get_or_init(TelegramRelay::new)
}

fn get_discord_relay() -> &'static DiscordRelay {
    DISCORD_RELAY.get_or_init(DiscordRelay::new)
}

/// Read `enable_telegram_relay` from config.toml.
/// Returns `true` (enabled) when the flag is absent or `[telegram_relay]` section is missing.
/// Returns `false` only when explicitly set to `false`.
fn telegram_relay_enabled() -> bool {
    match zeus_core::Config::load() {
        Ok(cfg) => cfg
            .telegram_relay
            .map(|r| r.enable_telegram_relay)
            .unwrap_or(true),
        Err(_) => true, // default to enabled if config can't be read
    }
}

// ---------------------------------------------------------------------------
// Talos tools for relay control
// ---------------------------------------------------------------------------

pub struct AutoStartRelayTool;
pub struct TelegramStartRelayTool;
pub struct TelegramStopRelayTool;
pub struct TelegramRelayStatusTool;
pub struct DiscordStartRelayTool;
pub struct DiscordStopRelayTool;
pub struct DiscordRelayStatusTool;

#[async_trait]
impl TalosTool for AutoStartRelayTool {
    fn name(&self) -> &'static str {
        "auto_start_relay"
    }
    fn description(&self) -> &'static str {
        "Auto-start all configured channel relays"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "auto_start_relay".to_string(),
            description: "Auto-start all configured channel relays (Telegram + Discord). Call this on session boot to begin receiving messages.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let mut results = Vec::new();
        let session = detect_tmux_session().unwrap_or_else(|| "unknown".to_string());

        // Start Telegram relay — honour per-node enable_telegram_relay toggle
        let tg_relay = get_relay();
        if tg_relay.is_running() {
            results.push("Telegram relay: already running".to_string());
        } else if telegram_relay_enabled() {
            match tg_relay.start().await {
                Ok(()) => results.push(format!("Telegram relay: started (session: {})", session)),
                Err(e) => results.push(format!("Telegram relay: {}", e)),
            }
        } else {
            results.push(
                "Telegram relay: disabled (enable_telegram_relay = false in config)".to_string(),
            );
        }

        // Start Discord relay
        let dc_relay = get_discord_relay();
        if dc_relay.is_running() {
            results.push("Discord relay: already running".to_string());
        } else if zeus_core::resolve_discord_token().is_some()
            && !dc_relay.channel_ids.is_empty()
        {
            match dc_relay.start().await {
                Ok(()) => results.push(format!(
                    "Discord relay: started ({} channels, session: {})",
                    dc_relay.channel_count(),
                    session
                )),
                Err(e) => results.push(format!("Discord relay: {}", e)),
            }
        } else {
            results.push(
                "Discord relay: skipped (no Discord token in config.toml or channel_ids empty)"
                    .to_string(),
            );
        }

        Ok(results.join("\n"))
    }
}

#[async_trait]
impl TalosTool for TelegramStartRelayTool {
    fn name(&self) -> &'static str {
        "telegram_start_relay"
    }
    fn description(&self) -> &'static str {
        "Start the Telegram message relay"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "telegram_start_relay".to_string(),
            description: "Start the Telegram relay. Polls for new messages and forwards them into your Claude Code tmux session.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        if !telegram_relay_enabled() {
            return Ok(
                "Telegram relay disabled by config (enable_telegram_relay = false).".to_string(),
            );
        }
        let relay = get_relay();
        if relay.is_running() {
            return Ok("Telegram relay is already running.".to_string());
        }
        match relay.start().await {
            Ok(()) => Ok(
                "Telegram relay started. Messages will be forwarded to your session.".to_string(),
            ),
            Err(e) => Ok(format!("Failed to start: {}", e)),
        }
    }
}

#[async_trait]
impl TalosTool for TelegramStopRelayTool {
    fn name(&self) -> &'static str {
        "telegram_stop_relay"
    }
    fn description(&self) -> &'static str {
        "Stop the Telegram message relay"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "telegram_stop_relay".to_string(),
            description: "Stop the background Telegram message polling relay.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let relay = get_relay();
        relay.stop().await;
        Ok("Telegram relay stopped.".to_string())
    }
}

#[async_trait]
impl TalosTool for TelegramRelayStatusTool {
    fn name(&self) -> &'static str {
        "telegram_relay_status"
    }
    fn description(&self) -> &'static str {
        "Check Telegram relay status"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "telegram_relay_status".to_string(),
            description: "Check Telegram relay status: whether it is running, queued messages, and last poll time.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let relay = get_relay();
        let running = relay.is_running();
        let queued = relay.queued_count();
        let last_poll = relay
            .last_poll_time()
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "never".to_string());
        let session = detect_tmux_session().unwrap_or_else(|| "not in tmux".to_string());

        Ok(format!(
            "Telegram relay: {}\nQueued messages: {}\nLast poll: {}\nTmux session: {}",
            if running { "running" } else { "stopped" },
            queued,
            last_poll,
            session,
        ))
    }
}

#[async_trait]
impl TalosTool for DiscordStartRelayTool {
    fn name(&self) -> &'static str {
        "discord_start_relay"
    }
    fn description(&self) -> &'static str {
        "Start the Discord message relay"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "discord_start_relay".to_string(),
            description: "Start the Discord relay. Polls configured channels for new messages and forwards them into your Claude Code tmux session. Requires DISCORD_BOT_TOKEN and DISCORD_RELAY_CHANNEL_IDS.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let relay = get_discord_relay();
        if relay.is_running() {
            return Ok("Discord relay is already running.".to_string());
        }
        match relay.start().await {
            Ok(()) => Ok(format!(
                "Discord relay started ({} channels). Messages will be forwarded to your session.",
                relay.channel_count()
            )),
            Err(e) => Ok(format!("Failed to start: {}", e)),
        }
    }
}

#[async_trait]
impl TalosTool for DiscordStopRelayTool {
    fn name(&self) -> &'static str {
        "discord_stop_relay"
    }
    fn description(&self) -> &'static str {
        "Stop the Discord message relay"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "discord_stop_relay".to_string(),
            description: "Stop the background Discord message polling relay.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let relay = get_discord_relay();
        relay.stop().await;
        Ok("Discord relay stopped.".to_string())
    }
}

#[async_trait]
impl TalosTool for DiscordRelayStatusTool {
    fn name(&self) -> &'static str {
        "discord_relay_status"
    }
    fn description(&self) -> &'static str {
        "Check Discord relay status"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "discord_relay_status".to_string(),
            description: "Check Discord relay status: whether it is running, channels monitored, queued messages, and last poll time.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let relay = get_discord_relay();
        let running = relay.is_running();
        let channels = relay.channel_count();
        let queued = relay.queued_count();
        let last_poll = relay
            .last_poll_time()
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "never".to_string());
        let session = detect_tmux_session().unwrap_or_else(|| "not in tmux".to_string());

        Ok(format!(
            "Discord relay: {}\nChannels monitored: {}\nQueued messages: {}\nLast poll: {}\nTmux session: {}",
            if running { "running" } else { "stopped" },
            channels,
            queued,
            last_poll,
            session,
        ))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_tmux_outside_tmux() {
        // Outside tmux, should return None
        // (may return Some if test runner is in tmux, that's fine)
        let _ = detect_tmux_session();
    }

    #[test]
    fn test_relay_new() {
        let relay = TelegramRelay::new();
        assert!(!relay.is_running());
        assert_eq!(relay.queued_count(), 0);
        assert!(relay.last_poll_time().is_none());
    }

    #[test]
    fn test_relay_drain_empty() {
        let relay = TelegramRelay::new();
        let drained = relay.drain_messages(10);
        assert!(drained.is_empty());
    }

    #[test]
    fn test_auto_start_relay_schema() {
        let tool = AutoStartRelayTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "auto_start_relay");
    }

    #[test]
    fn test_telegram_start_relay_schema() {
        let tool = TelegramStartRelayTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "telegram_start_relay");
    }

    #[test]
    fn test_telegram_stop_relay_schema() {
        let tool = TelegramStopRelayTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "telegram_stop_relay");
    }

    #[test]
    fn test_telegram_relay_status_schema() {
        let tool = TelegramRelayStatusTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "telegram_relay_status");
    }

    #[test]
    fn test_discord_relay_new() {
        let relay = DiscordRelay::new();
        assert!(!relay.is_running());
        assert_eq!(relay.queued_count(), 0);
        assert!(relay.last_poll_time().is_none());
    }

    #[test]
    fn test_discord_relay_drain_empty() {
        let relay = DiscordRelay::new();
        let drained = relay.drain_messages(10);
        assert!(drained.is_empty());
    }

    #[test]
    fn test_discord_start_relay_schema() {
        let tool = DiscordStartRelayTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "discord_start_relay");
    }

    #[test]
    fn test_discord_stop_relay_schema() {
        let tool = DiscordStopRelayTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "discord_stop_relay");
    }

    #[test]
    fn test_discord_relay_status_schema() {
        let tool = DiscordRelayStatusTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "discord_relay_status");
    }
}
