//! Telegram channel adapter using grammers (MTProto)

use crate::filters::AllowBotsMode;
use crate::policy::ChannelPolicy;
use crate::{ChannelAdapter, ChannelAttachment, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use grammers_client::types::{Chat, Media};
use grammers_client::{Client, Config, InitParams, SignInError, Update};
use grammers_session::{PackedType, Session};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, RwLock, mpsc};
use zeus_core::{Error, Result};

/// Telegram channel adapter using grammers (MTProto)
pub struct TelegramAdapter {
    client: Arc<RwLock<Option<Client>>>,
    config: TelegramConfig,
    connected: Arc<AtomicBool>,
    shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    allow_bots: AllowBotsMode,
    policy: ChannelPolicy,
    /// Shared HTTP client for Bot API calls (connection pooling)
    http: reqwest::Client,
    /// Layer 0: Message dedup — track recently seen message IDs to prevent
    /// duplicate processing on MTProto reconnect. Cleared when > 1000 entries.
    seen_messages: Arc<Mutex<HashSet<i32>>>,
}

impl TelegramAdapter {
    /// Create a new Telegram adapter
    pub async fn new(config: TelegramConfig) -> Result<Self> {
        tracing::info!("Creating Telegram adapter");
        let allow_bots = AllowBotsMode::from_config(config.allow_bots.as_deref());

        Ok(Self {
            client: Arc::new(RwLock::new(None)),
            allow_bots,
            policy: ChannelPolicy::default(),
            config,
            connected: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(Mutex::new(None)),
            http: reqwest::Client::new(),
            seen_messages: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    /// Connect to Telegram
    async fn connect(&self) -> Result<Client> {
        let session_path = self.config.session_path.clone().unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("zeus")
                .join("telegram.session")
                .to_string_lossy()
                .to_string()
        });

        // Ensure directory exists (non-blocking)
        if let Some(parent) = std::path::Path::new(&session_path).parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }

        // Load or create session
        let session = Session::load_file_or_create(&session_path)
            .map_err(|e| Error::channel(format!("Failed to load Telegram session: {}", e)))?;

        let client = Client::connect(Config {
            session,
            api_id: self.config.api_id,
            api_hash: self.config.api_hash.clone(),
            params: InitParams {
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                device_model: "Zeus AI Assistant".to_string(),
                system_version: std::env::consts::OS.to_string(),
                ..Default::default()
            },
        })
        .await
        .map_err(|e| Error::channel(format!("Failed to connect to Telegram: {}", e)))?;

        // Sign in if not authorized
        if !client.is_authorized().await.unwrap_or(false) {
            self.sign_in(&client).await?;
        }

        // Save session after successful connection
        client
            .session()
            .save_to_file(&session_path)
            .map_err(|e| Error::channel(format!("Failed to save session: {}", e)))?;

        tracing::info!("Connected to Telegram");
        Ok(client)
    }

    /// Sign in to Telegram (bot or user mode)
    async fn sign_in(&self, client: &Client) -> Result<()> {
        if let Some(ref bot_token) = self.config.bot_token {
            // Bot mode - simpler
            tracing::info!("Signing in as bot");
            client
                .bot_sign_in(bot_token)
                .await
                .map_err(|e| Error::channel(format!("Bot sign-in failed: {}", e)))?;
        } else if let Some(ref phone) = self.config.phone {
            // User mode - requires code verification
            tracing::info!("Signing in as user");
            let token = client
                .request_login_code(phone)
                .await
                .map_err(|e| Error::channel(format!("Failed to request code: {}", e)))?;

            // In a real implementation, we'd need to get the code from the user
            // For now, check if there's a code file or environment variable
            let code = std::env::var("TELEGRAM_CODE").map_err(|_| {
                Error::channel(
                    "User mode requires TELEGRAM_CODE environment variable for initial login",
                )
            })?;

            match client.sign_in(&token, &code).await {
                Ok(_) => {}
                Err(SignInError::PasswordRequired(password_token)) => {
                    // 2FA enabled
                    let password = std::env::var("TELEGRAM_2FA_PASSWORD").map_err(|_| {
                        Error::channel("2FA requires TELEGRAM_2FA_PASSWORD environment variable")
                    })?;
                    client
                        .check_password(password_token, password)
                        .await
                        .map_err(|e| Error::channel(format!("2FA verification failed: {}", e)))?;
                }
                Err(e) => {
                    return Err(Error::channel(format!("Sign-in failed: {}", e)));
                }
            }
        } else {
            return Err(Error::channel(
                "Telegram requires either bot_token or phone for authentication",
            ));
        }

        Ok(())
    }

    /// Process incoming updates
    #[allow(clippy::too_many_arguments)]
    async fn process_updates(
        client: Client,
        tx: mpsc::Sender<ChannelMessage>,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
        connected: Arc<AtomicBool>,
        bot_user_id: i64,
        allow_bots: AllowBotsMode,
        bot_username: Option<String>,
        policy: ChannelPolicy,
        seen_messages: Arc<Mutex<HashSet<i32>>>,
    ) {
        use tokio::select;

        let mut shutdown_rx = shutdown_rx;

        loop {
            select! {
                _ = &mut shutdown_rx => {
                    tracing::info!("Telegram update loop shutting down");
                    break;
                }
                update = client.next_update() => {
                    match update {
                        Ok(Some(update)) => {
                            if let Err(e) = Self::handle_update(&client, update, &tx, bot_user_id, allow_bots, &bot_username, &policy, &seen_messages).await {
                                tracing::error!("Error handling Telegram update: {}", e);
                            }
                        }
                        Ok(None) => {
                            // No update available, continue
                        }
                        Err(e) => {
                            tracing::error!("Error receiving Telegram update: {}", e);
                            if !connected.load(Ordering::SeqCst) {
                                break;
                            }
                            // Brief delay before retry
                            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                        }
                    }
                }
            }
        }
    }

    /// Handle a single update
    async fn handle_update(
        _client: &Client,
        update: Update,
        tx: &mpsc::Sender<ChannelMessage>,
        bot_user_id: i64,
        allow_bots: AllowBotsMode,
        bot_username: &Option<String>,
        policy: &ChannelPolicy,
        seen_messages: &Arc<Mutex<HashSet<i32>>>,
    ) -> Result<()> {
        match update {
            Update::NewMessage(message) => {
                // Layer 0: Message dedup — skip if we've already processed this message ID.
                // MTProto can redeliver updates on reconnect; this prevents double-processing.
                let msg_id = message.id();
                {
                    let mut seen = seen_messages.lock().await;
                    if !seen.insert(msg_id) {
                        tracing::debug!(message_id = msg_id, "Telegram dedup: skipping duplicate message");
                        return Ok(());
                    }
                    if seen.len() > 1000 {
                        seen.clear();
                    }
                }

                let sender = message.sender();

                // Layer 1: Self-echo — drop messages originating from our own account
                if sender.as_ref().is_some_and(|s| s.id() == bot_user_id) {
                    return Ok(());
                }

                // Layer 2: Bot message filter (mention-completion parity with relay path)
                // Uses grammers-native `message.mentioned()` which covers:
                //   - @username entity mentions
                //   - text mentions (no username)
                //   - reply-chain implicit mentions (replying to our message)
                // This is server-side truth — more reliable than text-only matching.
                let sender_is_bot = match &sender {
                    Some(Chat::User(u)) => u.is_bot(),
                    _ => false,
                };
                if sender_is_bot {
                    match allow_bots {
                        AllowBotsMode::Off => return Ok(()),
                        AllowBotsMode::Mentions => {
                            // grammers `mentioned()` is the server-side flag that covers
                            // entity mentions + reply-chain implicit mentions + text mentions.
                            // Fallback to text-based @username check for edge cases.
                            let mentioned = message.mentioned()
                                || bot_username
                                    .as_deref()
                                    .is_some_and(|uname| message.text().contains(&format!("@{uname}")));
                            if !mentioned {
                                return Ok(());
                            }
                        }
                        AllowBotsMode::On => {}
                    }
                }

                // Layer 3: Policy check (DM vs group)
                let chat = message.chat();
                let chat_id = chat.id();
                let sender_id_str = sender
                    .as_ref()
                    .map(|s| s.id().to_string())
                    .unwrap_or_else(|| chat_id.to_string());

                let is_group = matches!(&chat, Chat::Group(_) | Chat::Channel(_));
                if is_group {
                    let is_mention = message.mentioned()
                        || bot_username
                            .as_deref()
                            .is_some_and(|uname| message.text().contains(&format!("@{uname}")))
                        || message.text().starts_with('/');
                    if policy
                        .check_group(&chat_id.to_string(), &sender_id_str, is_mention)
                        .is_denied()
                    {
                        return Ok(());
                    }
                } else if policy.check_dm(&sender_id_str).is_denied() {
                    return Ok(());
                }

                // Layer 3: Robust mention detection (Discord parity)
                // Handle: @username, /command prefix, implicit DM addressing,
                // and grammers-native `mentioned()` (entity + reply-chain mentions).
                let text = message.text();
                let is_direct_message = !is_group;
                let is_addressed = is_group
                    && (message.mentioned()
                        || bot_username.as_deref().is_some_and(|uname| text.contains(&format!("@{uname}")))
                        || text.starts_with('/'))
                    || is_direct_message;

                // Layer 4: Emit message
                let user_id = sender.as_ref().map(|s| s.id()).unwrap_or(chat_id);
                // Classify sender type (S52 T3)
                let tg_sender_type = if sender_is_bot {
                    zeus_core::SenderType::Bot
                } else {
                    zeus_core::SenderType::Human
                };
                let source = ChannelSource::with_chat(
                    "telegram",
                    &user_id.to_string(),
                    &chat_id.to_string(),
                )
                .with_sender_type(tg_sender_type);

                let text = message.text();

                // Layer 4a: Attachment extraction (MTProto).
                // grammers downloads media to a local file path (not bytes-in-memory like
                // Bot HTTP), so we download to a tempfile, read it back, and build a
                // data-based ChannelAttachment for parity with the Discord/Bot-HTTP path.
                let mut attachments: Vec<ChannelAttachment> = Vec::new();
                if let Some(media) = message.media() {
                    let (filename, mime_type) = match &media {
                        Media::Document(doc) => {
                            let name = doc.name();
                            let fname = if name.is_empty() { None } else { Some(name.to_string()) };
                            // P0 file-type support: when Telegram doesn't set mime_type
                            // (or sets it to the generic octet-stream catch-all), infer
                            // from filename extension so process_attachments routes the
                            // attachment correctly (text/* → prompt prefix, Office MIMEs
                            // → text-extraction pipeline, application/pdf → vision LLM).
                            let mime = doc
                                .mime_type()
                                .filter(|m| !m.is_empty() && *m != "application/octet-stream")
                                .map(|m| m.to_string())
                                .or_else(|| {
                                    fname
                                        .as_deref()
                                        .and_then(crate::media::infer_mime_from_extension)
                                        .map(|m| m.to_string())
                                })
                                .unwrap_or_else(|| "application/octet-stream".to_string());
                            (fname, mime)
                        }
                        Media::Photo(_) => (
                            Some(format!("photo_{}.jpg", message.id())),
                            "image/jpeg".to_string(),
                        ),
                        Media::Sticker(_) => (
                            Some(format!("sticker_{}.webp", message.id())),
                            "image/webp".to_string(),
                        ),
                        // Contact/Poll/Geo/Dice/Venue/GeoLive/WebPage: non-file media, skip.
                        _ => (None, String::new()),
                    };

                    if !mime_type.is_empty() {
                        match tempfile::NamedTempFile::new() {
                            Ok(tmp) => {
                                // Persist temp file beyond this scope so download_media
                                // and fs::read both have a valid path. NamedTempFile
                                // auto-deletes on Drop — we keep() after read succeeds.
                                let path = tmp.into_temp_path();
                                let path_buf = path.to_path_buf();
                                match message.download_media(&path_buf).await {
                                    Ok(true) => match tokio::fs::read(&path_buf).await {
                                        Ok(bytes) => {
                                            // File read complete — safe to delete now
                                            let _ = path.close();
                                            let byte_count = bytes.len();
                                            let mut att = ChannelAttachment::from_data(bytes, &mime_type);
                                            if let Some(name) = filename.as_deref() {
                                                att = att.with_filename(name);
                                            }
                                            attachments.push(att);
                                            tracing::info!(
                                                chat_id = chat_id,
                                                filename = ?filename,
                                                mime = %mime_type,
                                                bytes = byte_count,
                                                "Downloaded Telegram MTProto attachment"
                                            );
                                        }
                                        Err(e) => {
                                            let _ = path.close();
                                            tracing::warn!(error = %e, path = %path_buf.display(), "Failed to read downloaded Telegram media");
                                        }
                                    },
                                    Ok(false) => {
                                        let _ = path.close();
                                        tracing::warn!(
                                            chat_id = chat_id,
                                            message_id = msg_id,
                                            filename = ?filename,
                                            mime = %mime_type,
                                            "Telegram message.download_media returned false — attachment skipped (file reference expired or bot lacks media rights)"
                                        );
                                    }
                                    Err(e) => {
                                        let _ = path.close();
                                        tracing::warn!(error = %e, "Failed to download Telegram media");
                                    }
                                }
                            }
                            Err(e) => tracing::warn!(error = %e, "Failed to create tempfile for Telegram media"),
                        }
                    }
                }

                // Emit if there's text OR attachments (file-only messages must propagate).
                if !text.is_empty() || !attachments.is_empty() {
                    let channel_message = if attachments.is_empty() {
                        ChannelMessage::new(source, text.to_string()).with_addressed(is_addressed)
                    } else {
                        ChannelMessage::with_attachments(source, text.to_string(), attachments)
                            .with_addressed(is_addressed)
                    };
                    tracing::debug!(
                        chat_id = chat_id,
                        text = text,
                        is_addressed = is_addressed,
                        attachments = channel_message.attachments.len(),
                        "Received Telegram message"
                    );
                    if tx.send(channel_message).await.is_err() {
                        tracing::warn!("Message receiver dropped");
                    }
                }
            }
            _ => {
                // Ignore other update types for now
            }
        }

        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn channel_type(&self) -> &'static str {
        "telegram"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Native // MTProto protocol
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        // Connect to Telegram
        let client = self.connect().await?;

        // Retrieve bot identity to enable self-echo filtering
        let me = client
            .get_me()
            .await
            .map_err(|e| Error::channel(format!("Failed to get bot identity: {}", e)))?;
        let bot_user_id = me.id();
        let bot_username = me.username().map(|s| s.to_string());

        // Store the client
        {
            let mut client_guard = self.client.write().await;
            *client_guard = Some(client.clone());
        }

        self.connected.store(true, Ordering::SeqCst);

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        {
            let mut shutdown_guard = self.shutdown.lock().await;
            *shutdown_guard = Some(shutdown_tx);
        }

        // Spawn update processing task
        let connected = self.connected.clone();
        let allow_bots = self.allow_bots;
        let policy = self.policy.clone();
        let seen_messages = self.seen_messages.clone();
        tokio::spawn(async move {
            Self::process_updates(
                client,
                tx,
                shutdown_rx,
                connected,
                bot_user_id,
                allow_bots,
                bot_username,
                policy,
                seen_messages,
            )
            .await;
        });

        tracing::info!("Telegram adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);

        // Signal shutdown
        {
            let mut shutdown_guard = self.shutdown.lock().await;
            if let Some(tx) = shutdown_guard.take() {
                let _ = tx.send(());
            }
        }

        // Clear client
        {
            let mut client_guard = self.client.write().await;
            *client_guard = None;
        }

        tracing::info!("Telegram adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "telegram" {
            return Err(Error::channel("Invalid channel source for Telegram"));
        }

        let chat_id_str = to
            .chat_id
            .as_deref()
            .ok_or_else(|| Error::channel("Telegram send requires a chat_id"))?;
        let chat_id: i64 = chat_id_str
            .parse()
            .map_err(|_| Error::channel(format!("Invalid Telegram chat_id: {}", chat_id_str)))?;

        let client_guard = self.client.read().await;
        let client = client_guard
            .as_ref()
            .ok_or_else(|| Error::channel("Telegram not connected"))?;

        // Determine chat type from ID
        let packed = grammers_client::types::PackedChat {
            ty: if chat_id > 0 {
                PackedType::User
            } else if chat_id > -1000000000000 {
                PackedType::Chat
            } else {
                PackedType::Megagroup
            },
            id: chat_id.unsigned_abs() as i64,
            access_hash: None,
        };

        let chat = client
            .unpack_chat(packed)
            .await
            .map_err(|e| Error::channel(format!("Failed to resolve chat {}: {}", chat_id, e)))?;

        client
            .send_message(&chat, content)
            .await
            .map_err(|e| Error::channel(format!("Failed to send message: {}", e)))?;

        tracing::debug!(chat_id = chat_id, "Sent Telegram message");
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn supports_native_identity(&self) -> bool {
        false
    }

    async fn send_typing(&self, to: &ChannelSource) -> Result<()> {
        // Telegram Bot API sendChatAction for typing indicator
        if let Some(ref token) = self.config.bot_token {
            let chat_id_str = to.chat_id.as_deref().unwrap_or(&to.user_id);
            let url = format!("https://api.telegram.org/bot{}/sendChatAction", token);
            let client = &self.http;
            let _ = client
                .post(&url)
                .json(&serde_json::json!({
                    "chat_id": chat_id_str,
                    "action": "typing"
                }))
                .send()
                .await;
        }
        Ok(())
    }

    fn supports_typing(&self) -> bool {
        true
    }

    async fn send_file(
        &self,
        to: &ChannelSource,
        filename: &str,
        data: &[u8],
        caption: Option<&str>,
    ) -> Result<()> {
        let token = self
            .config
            .bot_token
            .as_ref()
            .ok_or_else(|| Error::channel("Telegram send_file requires bot_token"))?;
        let chat_id_str = to
            .chat_id
            .as_deref()
            .ok_or_else(|| Error::channel("Telegram send_file requires a chat_id"))?;

        let url = format!("https://api.telegram.org/bot{}/sendDocument", token);

        let file_part = reqwest::multipart::Part::bytes(data.to_vec())
            .file_name(filename.to_string())
            .mime_str("application/octet-stream")
            .map_err(|e| Error::channel(format!("Failed to create file part: {}", e)))?;

        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id_str.to_string())
            .part("document", file_part);

        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }

        let response = self
            .http
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::channel(format!("Failed to send file: {}", e)))?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(Error::channel(format!("Telegram sendDocument failed: {}", body)));
        }

        tracing::debug!(chat_id = chat_id_str, filename, "Sent Telegram file");
        Ok(())
    }

    // ── Rich-response support (#85 Cut 3-E) ─────────────────────────────
    //
    // Telegram caps: markdown text + inline images + file attachments.
    // No native embed/threading layer like Discord/Slack, so we project
    // a `RichResponse` onto:
    //   1. sendPhoto per image URL (carries optional caption on first photo)
    //   2. sendMessage with parse_mode=Markdown for the rendered text body
    //   3. sendDocument per inline file blob (reuses send_file pattern)
    //
    // Any sub-step failure falls back to the trait-default text-degradation
    // path (`self.send(to, &response.to_text())`) — no partial-send loss.

    fn capabilities(&self) -> crate::rich::ChannelCapabilities {
        crate::rich::ChannelCapabilities {
            rich_content: false,
            inline_images: true,
            file_attachments: true,
            markdown: true,
            tables: true,
            threading: false,
        }
    }

    async fn send_rich(
        &self,
        to: &ChannelSource,
        response: &crate::rich::RichResponse,
    ) -> Result<()> {
        // Pre-flight: telegram source + bot_token + chat_id.
        if to.channel_type() != "telegram" {
            return Err(Error::channel("Invalid channel source for Telegram"));
        }
        let token = match self.config.bot_token.as_ref() {
            Some(t) => t.clone(),
            None => {
                // No bot token → degrade to MTProto plain send via to_text().
                return self.send(to, &response.to_text()).await;
            }
        };
        let chat_id_str = match to.chat_id.as_deref() {
            Some(c) => c.to_string(),
            None => {
                return Err(Error::channel(
                    "Telegram send_rich requires chat_id",
                ));
            }
        };

        let rendered = crate::rich_render::render_telegram(response);

        // Step 1: sendPhoto per image URL.
        // First photo carries the markdown caption inline (telegram convention);
        // subsequent photos are bare. If caption-on-first fails to deliver, the
        // text body is still sent via sendMessage in step 2.
        let mut first_image = true;
        let caption_for_first = if rendered.image_urls.len() == 1 && !rendered.text.is_empty() {
            // Single-image case: caption inline, skip step-2 sendMessage.
            Some(rendered.text.clone())
        } else {
            None
        };
        for url in &rendered.image_urls {
            let endpoint = format!("https://api.telegram.org/bot{}/sendPhoto", token);
            let mut form = vec![
                ("chat_id", chat_id_str.clone()),
                ("photo", url.clone()),
            ];
            if first_image {
                if let Some(ref cap) = caption_for_first {
                    form.push(("caption", cap.clone()));
                    form.push(("parse_mode", "Markdown".to_string()));
                }
                first_image = false;
            }
            let resp = self.http.post(&endpoint).form(&form).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => {
                    let body = r.text().await.unwrap_or_default();
                    tracing::warn!(url, body=%body, "Telegram sendPhoto failed; falling back");
                    return self.send(to, &response.to_text()).await;
                }
                Err(e) => {
                    tracing::warn!(url, error=%e, "Telegram sendPhoto network error; falling back");
                    return self.send(to, &response.to_text()).await;
                }
            }
        }

        // Step 2: sendMessage with markdown body (unless single-image w/ inline caption).
        if caption_for_first.is_none() && !rendered.text.is_empty() {
            let endpoint = format!("https://api.telegram.org/bot{}/sendMessage", token);
            let form = vec![
                ("chat_id", chat_id_str.clone()),
                ("text", rendered.text.clone()),
                ("parse_mode", "Markdown".to_string()),
            ];
            let resp = self.http.post(&endpoint).form(&form).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => {
                    let body = r.text().await.unwrap_or_default();
                    tracing::warn!(body=%body, "Telegram sendMessage(Markdown) failed; falling back to plain send");
                    return self.send(to, &response.to_text()).await;
                }
                Err(e) => {
                    tracing::warn!(error=%e, "Telegram sendMessage network error; falling back");
                    return self.send(to, &response.to_text()).await;
                }
            }
        }

        // Step 3: inline file blobs via existing send_file().
        for (filename, bytes, caption) in &rendered.files {
            if let Err(e) = self
                .send_file(to, filename, bytes, caption.as_deref())
                .await
            {
                tracing::warn!(filename, error=%e, "Telegram send_file failed in send_rich; continuing");
                // Don't fully abort; text + images already shipped.
            }
        }

        Ok(())
    }
}

/// Poll configuration for Telegram
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TelegramPoll {
    pub question: String,
    pub options: Vec<String>,
    #[serde(default)]
    pub is_anonymous: bool,
    #[serde(default)]
    pub allows_multiple_answers: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correct_option_id: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
}

/// Poll result from Telegram
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TelegramPollResult {
    pub poll_id: String,
    pub question: String,
    pub options: Vec<PollOption>,
    pub total_voter_count: u32,
    pub is_closed: bool,
    pub is_anonymous: bool,
    pub allows_multiple_answers: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PollOption {
    pub text: String,
    pub voter_count: u32,
}

impl TelegramAdapter {
    /// Send a poll to a Telegram chat (Bot API only)
    ///
    /// # Arguments
    /// * `to` - Channel source with chat_id
    /// * `poll` - Poll configuration
    ///
    /// # Returns
    /// Message ID of the sent poll
    pub async fn send_poll(&self, to: &ChannelSource, poll: &TelegramPoll) -> Result<i64> {
        let bot_token = self
            .config
            .bot_token
            .as_ref()
            .ok_or_else(|| Error::channel("Telegram polls require bot_token (Bot API)"))?;

        if poll.options.len() < 2 {
            return Err(Error::channel("Poll must have at least 2 options"));
        }
        if poll.options.len() > 10 {
            return Err(Error::channel("Poll can have at most 10 options"));
        }

        let chat_id_str = to
            .chat_id
            .as_deref()
            .ok_or_else(|| Error::channel("Telegram poll send requires a chat_id"))?;

        let url = format!("https://api.telegram.org/bot{}/sendPoll", bot_token);
        let client = reqwest::Client::new();

        let mut payload = serde_json::json!({
            "chat_id": chat_id_str,
            "question": poll.question,
            "options": poll.options,
            "is_anonymous": poll.is_anonymous,
            "allows_multiple_answers": poll.allows_multiple_answers,
        });

        // Add quiz-specific fields if provided
        if let Some(correct_id) = poll.correct_option_id {
            payload["type"] = serde_json::json!("quiz");
            payload["correct_option_id"] = serde_json::json!(correct_id);
            if let Some(ref explanation) = poll.explanation {
                payload["explanation"] = serde_json::json!(explanation);
            }
        }

        let response = client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::channel(format!("Failed to send poll request: {}", e)))?;

        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| Error::channel(format!("Failed to parse poll response: {}", e)))?;

        if !status.is_success() {
            let error_msg = body["description"].as_str().unwrap_or("Unknown error");
            return Err(Error::channel(format!("Telegram API error: {}", error_msg)));
        }

        let message_id = body["result"]["message_id"]
            .as_i64()
            .ok_or_else(|| Error::channel("Failed to extract message_id from poll response"))?;

        tracing::info!(
            chat_id = chat_id_str,
            message_id = message_id,
            "Sent Telegram poll"
        );

        Ok(message_id)
    }

    /// Stop a poll (Bot API only)
    ///
    /// # Arguments
    /// * `chat_id` - Chat ID where the poll was sent
    /// * `message_id` - Message ID of the poll
    ///
    /// # Returns
    /// Final poll results
    pub async fn stop_poll(&self, chat_id: &str, message_id: i64) -> Result<TelegramPollResult> {
        let bot_token = self
            .config
            .bot_token
            .as_ref()
            .ok_or_else(|| Error::channel("Telegram polls require bot_token (Bot API)"))?;

        let url = format!("https://api.telegram.org/bot{}/stopPoll", bot_token);
        let client = reqwest::Client::new();

        let response = client
            .post(&url)
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "message_id": message_id,
            }))
            .send()
            .await
            .map_err(|e| Error::channel(format!("Failed to stop poll request: {}", e)))?;

        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| Error::channel(format!("Failed to parse stop poll response: {}", e)))?;

        if !status.is_success() {
            let error_msg = body["description"].as_str().unwrap_or("Unknown error");
            return Err(Error::channel(format!("Telegram API error: {}", error_msg)));
        }

        Self::parse_poll_result(&body["result"])
    }

    /// Parse poll result from Telegram API response
    fn parse_poll_result(poll_data: &serde_json::Value) -> Result<TelegramPollResult> {
        let poll_id = poll_data["id"]
            .as_str()
            .ok_or_else(|| Error::channel("Missing poll id"))?
            .to_string();

        let question = poll_data["question"]
            .as_str()
            .ok_or_else(|| Error::channel("Missing poll question"))?
            .to_string();

        let options_array = poll_data["options"]
            .as_array()
            .ok_or_else(|| Error::channel("Missing poll options"))?;

        let options = options_array
            .iter()
            .map(|opt| {
                Ok(PollOption {
                    text: opt["text"]
                        .as_str()
                        .ok_or_else(|| Error::channel("Missing option text"))?
                        .to_string(),
                    voter_count: opt["voter_count"]
                        .as_u64()
                        .ok_or_else(|| Error::channel("Missing voter_count"))?
                        as u32,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let total_voter_count = poll_data["total_voter_count"]
            .as_u64()
            .ok_or_else(|| Error::channel("Missing total_voter_count"))?
            as u32;

        let is_closed = poll_data["is_closed"]
            .as_bool()
            .ok_or_else(|| Error::channel("Missing is_closed"))?;

        let is_anonymous = poll_data["is_anonymous"].as_bool().unwrap_or(true);

        let allows_multiple_answers = poll_data["allows_multiple_answers"]
            .as_bool()
            .unwrap_or(false);

        Ok(TelegramPollResult {
            poll_id,
            question,
            options,
            total_voter_count,
            is_closed,
            is_anonymous,
            allows_multiple_answers,
        })
    }

    /// Register bot commands in the Telegram menu (Bot API only)
    ///
    /// This sets up the command menu that appears when users type / in the chat.
    /// Commands should be lowercase and contain only a-z, 0-9, and underscores.
    ///
    /// # Arguments
    /// * `commands` - List of (command, description) tuples
    ///
    /// # Example
    /// ```no_run
    /// # use zeus_channels::TelegramAdapter;
    /// # async fn example(adapter: &TelegramAdapter) {
    /// adapter.register_bot_menu(&[
    ///     ("compact", "Enable compact mode"),
    ///     ("status", "Show system status"),
    ///     ("help", "Show help message"),
    /// ]).await.unwrap();
    /// # }
    /// ```
    pub async fn register_bot_menu(&self, commands: &[(&str, &str)]) -> Result<()> {
        let bot_token = self
            .config
            .bot_token
            .as_ref()
            .ok_or_else(|| Error::channel("Telegram bot menu requires bot_token (Bot API)"))?;

        if commands.is_empty() {
            return Err(Error::channel("At least one command must be provided"));
        }

        if commands.len() > 100 {
            return Err(Error::channel("Maximum 100 commands allowed"));
        }

        // Validate command format
        for (cmd, desc) in commands {
            if cmd.is_empty() || cmd.len() > 32 {
                return Err(Error::channel(format!(
                    "Command '{}' must be 1-32 characters",
                    cmd
                )));
            }

            if !cmd
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            {
                return Err(Error::channel(format!(
                    "Command '{}' must contain only lowercase letters, digits, and underscores",
                    cmd
                )));
            }

            if desc.is_empty() || desc.len() > 256 {
                return Err(Error::channel(format!(
                    "Description for '{}' must be 1-256 characters",
                    cmd
                )));
            }
        }

        let url = format!("https://api.telegram.org/bot{}/setMyCommands", bot_token);
        let client = reqwest::Client::new();

        let commands_json: Vec<serde_json::Value> = commands
            .iter()
            .map(|(cmd, desc)| {
                serde_json::json!({
                    "command": cmd,
                    "description": desc,
                })
            })
            .collect();

        let response = client
            .post(&url)
            .json(&serde_json::json!({
                "commands": commands_json,
            }))
            .send()
            .await
            .map_err(|e| Error::channel(format!("Failed to register bot menu: {}", e)))?;

        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| Error::channel(format!("Failed to parse bot menu response: {}", e)))?;

        if !status.is_success() {
            let error_msg = body["description"].as_str().unwrap_or("Unknown error");
            return Err(Error::channel(format!("Telegram API error: {}", error_msg)));
        }

        let ok = body["ok"].as_bool().unwrap_or(false);
        if !ok {
            let error_msg = body["description"]
                .as_str()
                .unwrap_or("Registration failed");
            return Err(Error::channel(format!(
                "Bot menu registration failed: {}",
                error_msg
            )));
        }

        tracing::info!("Successfully registered {} bot commands", commands.len());
        Ok(())
    }
}

/// Telegram configuration
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct TelegramConfig {
    /// API ID from my.telegram.org
    pub api_id: i32,
    /// API hash from my.telegram.org
    #[serde(skip_serializing)]
    pub api_hash: String,
    /// Bot token (if using bot mode)
    #[serde(default, skip_serializing)]
    pub bot_token: Option<String>,
    /// Phone number (if using user mode)
    pub phone: Option<String>,
    /// Session file path
    pub session_path: Option<String>,
    /// Bot message filter: "off" | "mentions" (default) | "on"
    #[serde(default)]
    pub allow_bots: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentSendIdentity;

    #[test]
    fn test_telegram_config_default() {
        let config = TelegramConfig::default();
        assert_eq!(config.api_id, 0);
        assert!(config.bot_token.is_none());
    }

    #[tokio::test]
    async fn test_adapter_creation() {
        let config = TelegramConfig::default();
        let adapter = TelegramAdapter::new(config).await;
        assert!(adapter.is_ok());
    }

    #[test]
    fn test_telegram_poll_creation() {
        let poll = TelegramPoll {
            question: "What's your favorite color?".to_string(),
            options: vec!["Red".to_string(), "Blue".to_string(), "Green".to_string()],
            is_anonymous: true,
            allows_multiple_answers: false,
            correct_option_id: None,
            explanation: None,
        };

        assert_eq!(poll.question, "What's your favorite color?");
        assert_eq!(poll.options.len(), 3);
        assert!(poll.is_anonymous);
        assert!(!poll.allows_multiple_answers);
    }

    #[test]
    fn test_telegram_poll_quiz() {
        let poll = TelegramPoll {
            question: "What is 2 + 2?".to_string(),
            options: vec!["3".to_string(), "4".to_string(), "5".to_string()],
            is_anonymous: false,
            allows_multiple_answers: false,
            correct_option_id: Some(1), // Index 1 = "4"
            explanation: Some("Simple math!".to_string()),
        };

        assert_eq!(poll.correct_option_id, Some(1));
        assert!(poll.explanation.is_some());
    }

    #[test]
    fn test_parse_poll_result() {
        let json_data = serde_json::json!({
            "id": "5319694702468239360",
            "question": "What's your favorite color?",
            "options": [
                {"text": "Red", "voter_count": 3},
                {"text": "Blue", "voter_count": 5},
                {"text": "Green", "voter_count": 2}
            ],
            "total_voter_count": 10,
            "is_closed": true,
            "is_anonymous": true,
            "allows_multiple_answers": false
        });

        let result =
            TelegramAdapter::parse_poll_result(&json_data).expect("should parse successfully");

        assert_eq!(result.poll_id, "5319694702468239360");
        assert_eq!(result.question, "What's your favorite color?");
        assert_eq!(result.options.len(), 3);
        assert_eq!(result.options[0].text, "Red");
        assert_eq!(result.options[0].voter_count, 3);
        assert_eq!(result.options[1].voter_count, 5);
        assert_eq!(result.total_voter_count, 10);
        assert!(result.is_closed);
        assert!(result.is_anonymous);
        assert!(!result.allows_multiple_answers);
    }

    #[test]
    fn test_parse_poll_result_missing_id() {
        let json_data = serde_json::json!({
            "question": "Test?",
            "options": []
        });

        let result = TelegramAdapter::parse_poll_result(&json_data);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_send_poll_requires_bot_token() {
        let config = TelegramConfig {
            api_id: 12345,
            api_hash: "test_hash".to_string(),
            bot_token: None, // No bot token
            phone: Some("+1234567890".to_string()),
            session_path: None,
            allow_bots: None,
        };

        let adapter = TelegramAdapter::new(config)
            .await
            .expect("TelegramAdapter::new should succeed");
        let source = ChannelSource::with_chat("telegram", "123", "123");
        let poll = TelegramPoll {
            question: "Test?".to_string(),
            options: vec!["A".to_string(), "B".to_string()],
            is_anonymous: true,
            allows_multiple_answers: false,
            correct_option_id: None,
            explanation: None,
        };

        let result = adapter.send_poll(&source, &poll).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("bot_token"));
    }

    #[tokio::test]
    async fn test_send_poll_validates_option_count() {
        let config = TelegramConfig {
            api_id: 12345,
            api_hash: "test_hash".to_string(),
            bot_token: Some("fake_token".to_string()),
            phone: None,
            session_path: None,
            allow_bots: None,
        };

        let adapter = TelegramAdapter::new(config)
            .await
            .expect("TelegramAdapter::new should succeed");
        let source = ChannelSource::with_chat("telegram", "123", "123");

        // Too few options
        let poll = TelegramPoll {
            question: "Test?".to_string(),
            options: vec!["A".to_string()], // Only 1 option
            is_anonymous: true,
            allows_multiple_answers: false,
            correct_option_id: None,
            explanation: None,
        };

        let result = adapter.send_poll(&source, &poll).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("at least 2"));

        // Too many options
        let poll = TelegramPoll {
            question: "Test?".to_string(),
            options: (0..11).map(|i| format!("Option {}", i)).collect(), // 11 options
            is_anonymous: true,
            allows_multiple_answers: false,
            correct_option_id: None,
            explanation: None,
        };

        let result = adapter.send_poll(&source, &poll).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("at most 10"));
    }

    #[test]
    fn test_poll_option_serialization() {
        let option = PollOption {
            text: "Test Option".to_string(),
            voter_count: 42,
        };

        let json = serde_json::to_string(&option).expect("should serialize to JSON");
        assert!(json.contains("Test Option"));
        assert!(json.contains("42"));
    }

    #[test]
    fn test_poll_result_serialization() {
        let result = TelegramPollResult {
            poll_id: "test123".to_string(),
            question: "Test?".to_string(),
            options: vec![
                PollOption {
                    text: "Yes".to_string(),
                    voter_count: 10,
                },
                PollOption {
                    text: "No".to_string(),
                    voter_count: 5,
                },
            ],
            total_voter_count: 15,
            is_closed: false,
            is_anonymous: true,
            allows_multiple_answers: false,
        };

        let json = serde_json::to_value(&result).expect("should serialize to JSON");
        assert_eq!(json["poll_id"], "test123");
        assert_eq!(json["total_voter_count"], 15);
        assert_eq!(json["options"][0]["voter_count"], 10);
    }

    #[tokio::test]
    async fn test_register_bot_menu_validates_bot_token() {
        let config = TelegramConfig {
            api_id: 12345,
            api_hash: "test_hash".to_string(),
            bot_token: None, // No bot token
            phone: None,
            session_path: None,
            allow_bots: None,
        };

        let adapter = TelegramAdapter::new(config)
            .await
            .expect("TelegramAdapter::new should succeed");
        let result = adapter
            .register_bot_menu(&[("start", "Start the bot")])
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("bot_token"));
    }

    #[tokio::test]
    async fn test_register_bot_menu_validates_command_count() {
        let config = TelegramConfig {
            api_id: 12345,
            api_hash: "test_hash".to_string(),
            bot_token: Some("fake_token".to_string()),
            phone: None,
            session_path: None,
            allow_bots: None,
        };

        let adapter = TelegramAdapter::new(config)
            .await
            .expect("TelegramAdapter::new should succeed");

        // Empty commands
        let result = adapter.register_bot_menu(&[]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("At least one"));
    }

    #[tokio::test]
    async fn test_register_bot_menu_validates_command_format() {
        let config = TelegramConfig {
            api_id: 12345,
            api_hash: "test_hash".to_string(),
            bot_token: Some("fake_token".to_string()),
            phone: None,
            session_path: None,
            allow_bots: None,
        };

        let adapter = TelegramAdapter::new(config)
            .await
            .expect("TelegramAdapter::new should succeed");

        // Invalid command with uppercase
        let result = adapter
            .register_bot_menu(&[("Start", "Start the bot")])
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("lowercase"));

        // Invalid command with special chars
        let result = adapter
            .register_bot_menu(&[("start!", "Start the bot")])
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("lowercase"));

        // Command too long
        let result = adapter
            .register_bot_menu(&[("a".repeat(33).as_str(), "Test")])
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("1-32"));

        // Description too long
        let result = adapter
            .register_bot_menu(&[("test", &"a".repeat(257))])
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("1-256"));
    }

    #[test]
    fn test_bot_menu_command_validation() {
        // Valid commands
        assert!(
            "start"
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        );
        assert!(
            "help_me"
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        );
        assert!(
            "cmd123"
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        );

        // Invalid commands
        assert!(
            !"Start"
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        );
        assert!(
            !"help-me"
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        );
        assert!(
            !"help!"
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        );
    }

    // ── S43: allow_bots filter ───────────────────────────────────────────────

    #[test]
    fn test_allow_bots_mode_from_config() {
        assert_eq!(
            AllowBotsMode::from_config(None),
            AllowBotsMode::Mentions,
            "None → Mentions (default, OpenClaw parity)"
        );
        assert_eq!(AllowBotsMode::from_config(Some("on")), AllowBotsMode::On);
        assert_eq!(AllowBotsMode::from_config(Some("true")), AllowBotsMode::On);
        assert_eq!(
            AllowBotsMode::from_config(Some("mentions")),
            AllowBotsMode::Mentions
        );
        assert_eq!(AllowBotsMode::from_config(Some("off")), AllowBotsMode::Off);
    }

    #[tokio::test]
    async fn test_telegram_allow_bots_field_parsed() {
        let config = TelegramConfig {
            allow_bots: Some("mentions".to_string()),
            ..Default::default()
        };
        // new() should not panic and should parse allow_bots correctly
        let adapter = TelegramAdapter::new(config)
            .await
            .expect("TelegramAdapter::new should succeed");
        assert_eq!(adapter.allow_bots, AllowBotsMode::Mentions);
    }

    // ── S33 Track D: Tier 2 identity tests ──────────────────────────────────

    #[tokio::test]
    async fn test_telegram_supports_native_identity_false() {
        let config = TelegramConfig::default();
        let adapter = TelegramAdapter::new(config)
            .await
            .expect("TelegramAdapter::new should succeed");
        assert!(!adapter.supports_native_identity());
    }

    #[test]
    fn test_telegram_send_as_text_prefix_format() {
        let identity = AgentSendIdentity::new("zeus_agent");
        let prefixed = identity.apply_prefix("Hello from Telegram");
        assert_eq!(prefixed, "[zeus_agent] Hello from Telegram");
    }

    // ── #85 Cut 3-E: send_rich override surface tests ────────────────────

    #[tokio::test]
    async fn test_telegram_capabilities_advertises_markdown_and_images() {
        // Pure capability surface assertion — no I/O.
        let config = TelegramConfig {
            bot_token: Some("fake_token".to_string()),
            ..Default::default()
        };
        let adapter = TelegramAdapter::new(config)
            .await
            .expect("TelegramAdapter::new should succeed");
        let caps = adapter.capabilities();
        assert!(caps.markdown, "telegram must advertise markdown");
        assert!(caps.inline_images, "telegram must advertise inline images");
        assert!(caps.file_attachments, "telegram must advertise file attachments");
        assert!(caps.tables, "telegram markdown supports table glyphs");
        assert!(!caps.rich_content, "telegram has no native embed layer");
        assert!(!caps.threading, "telegram threading not modeled here");
    }

    #[tokio::test]
    async fn test_telegram_send_rich_rejects_non_telegram_source() {
        let config = TelegramConfig {
            bot_token: Some("fake_token".to_string()),
            ..Default::default()
        };
        let adapter = TelegramAdapter::new(config)
            .await
            .expect("TelegramAdapter::new should succeed");
        let wrong_source = ChannelSource::with_chat("discord", "123", "c");
        let rich = crate::rich::RichResponse::new().title("t");
        let result = adapter.send_rich(&wrong_source, &rich).await;
        assert!(result.is_err(), "must reject non-telegram source");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Invalid channel source for Telegram"),
            "expected channel-mismatch error, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_telegram_send_rich_without_bot_token_degrades_via_mtproto_send() {
        // No bot_token + no auth means MTProto send() will return Err
        // (no authenticated client). The crucial assertion is that send_rich
        // does NOT return the "requires bot_token" channel error and instead
        // falls through to self.send() — i.e., follows the documented
        // degradation contract.
        let config = TelegramConfig {
            bot_token: None,
            ..Default::default()
        };
        let adapter = TelegramAdapter::new(config)
            .await
            .expect("TelegramAdapter::new should succeed");
        let to = ChannelSource::with_chat("telegram", "42", "42");
        let rich = crate::rich::RichResponse::new().title("hello");
        let result = adapter.send_rich(&to, &rich).await;
        // We don't assert Ok here — MTProto send will fail without auth —
        // but the error must NOT be the bot_token gate (which would mean
        // we short-circuited instead of degrading).
        if let Err(e) = result {
            let msg = e.to_string();
            assert!(
                !msg.contains("send_rich requires bot_token"),
                "expected degradation through self.send(), got bot_token gate: {msg}"
            );
        }
    }
}
