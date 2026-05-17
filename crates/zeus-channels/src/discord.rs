//! Discord channel adapter using serenity
//!
//! Full-featured Discord bot with:
//! - Gateway message receive/send
//! - Slash command registration and handling
//! - Rich embeds (title, description, fields, color, footer, thumbnail)
//! - Reaction add/remove listening
//! - Thread creation and reply
//! - File attachment send
//! - Bot status/presence setting

use base64::Engine as _;
use crate::filters::AllowBotsMode;
use crate::policy::ChannelPolicy;
use crate::{ChannelAdapter, ChannelAttachment, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use serenity::Client as SerenityClient;
use serenity::all::{
    ChannelId, ChannelType, Command, CommandOptionType, Context, CreateAttachment, CreateCommand,
    CreateCommandOption, CreateEmbed, CreateEmbedAuthor, CreateEmbedFooter, CreateMessage,
    CreateThread, EditMessage, EventHandler, GatewayIntents, Interaction,
    Message as DiscordMessage, MessageId, MessageReference, ReactionType, Ready,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, RwLock, mpsc};
use zeus_core::{Error, Result};

// ── Helpers ─────────────────────────────────────────────────────────────

/// Pre-decode the bot user ID from the token's first segment so the self-echo
/// filter works immediately — before the `ready` event fires.
///
/// Discord bot token format: `base64(bot_user_id).timestamp.hmac`
/// The first segment is the numeric user ID encoded as standard base64
/// (no padding). If decoding fails for any reason we return `None` and fall
/// back to the `ready`-event path.
pub fn decode_bot_id_from_token(token: &str) -> Option<u64> {
    let first = token.split('.').next()?;
    // Re-add padding stripped by Discord's token format
    let padded = match first.len() % 4 {
        2 => format!("{}==", first),
        3 => format!("{}=", first),
        _ => first.to_string(),
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&padded)
        .ok()?;
    std::str::from_utf8(&bytes).ok()?.parse().ok()
}

// ── Types ───────────────────────────────────────────────────────────────

/// Discord embed for rich messages
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DiscordEmbed {
    pub title: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub color: Option<u32>,
    pub fields: Vec<EmbedField>,
    pub footer: Option<String>,
    pub thumbnail_url: Option<String>,
    pub image_url: Option<String>,
    pub author_name: Option<String>,
    pub author_icon_url: Option<String>,
    pub timestamp: Option<String>,
}

impl DiscordEmbed {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    pub fn color(mut self, color: u32) -> Self {
        self.color = Some(color);
        self
    }

    pub fn field(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
        inline: bool,
    ) -> Self {
        self.fields.push(EmbedField {
            name: name.into(),
            value: value.into(),
            inline,
        });
        self
    }

    pub fn footer(mut self, text: impl Into<String>) -> Self {
        self.footer = Some(text.into());
        self
    }

    pub fn thumbnail(mut self, url: impl Into<String>) -> Self {
        self.thumbnail_url = Some(url.into());
        self
    }

    pub fn image(mut self, url: impl Into<String>) -> Self {
        self.image_url = Some(url.into());
        self
    }

    pub fn author(mut self, name: impl Into<String>) -> Self {
        self.author_name = Some(name.into());
        self
    }

    pub fn author_icon(mut self, url: impl Into<String>) -> Self {
        self.author_icon_url = Some(url.into());
        self
    }

    /// Convert to serenity CreateEmbed
    fn to_serenity(&self) -> CreateEmbed {
        let mut embed = CreateEmbed::new();
        if let Some(ref title) = self.title {
            embed = embed.title(title);
        }
        if let Some(ref desc) = self.description {
            embed = embed.description(desc);
        }
        if let Some(ref url) = self.url {
            embed = embed.url(url);
        }
        if let Some(color) = self.color {
            embed = embed.colour(color);
        }
        for field in &self.fields {
            embed = embed.field(&field.name, &field.value, field.inline);
        }
        if let Some(ref footer) = self.footer {
            embed = embed.footer(CreateEmbedFooter::new(footer));
        }
        if let Some(ref thumb) = self.thumbnail_url {
            embed = embed.thumbnail(thumb);
        }
        if let Some(ref img) = self.image_url {
            embed = embed.image(img);
        }
        if let Some(ref author_name) = self.author_name {
            let mut author = CreateEmbedAuthor::new(author_name);
            if let Some(ref icon) = self.author_icon_url {
                author = author.icon_url(icon);
            }
            embed = embed.author(author);
        }
        embed
    }
}

/// Embed field
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

/// Slash command definition
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub options: Vec<SlashCommandOption>,
}

/// Slash command option
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SlashCommandOption {
    pub name: String,
    pub description: String,
    pub kind: SlashOptionKind,
    pub required: bool,
}

/// Slash command option type
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SlashOptionKind {
    String,
    Integer,
    Boolean,
    User,
    Channel,
    Role,
}

impl SlashOptionKind {
    fn to_serenity(&self) -> CommandOptionType {
        match self {
            Self::String => CommandOptionType::String,
            Self::Integer => CommandOptionType::Integer,
            Self::Boolean => CommandOptionType::Boolean,
            Self::User => CommandOptionType::User,
            Self::Channel => CommandOptionType::Channel,
            Self::Role => CommandOptionType::Role,
        }
    }
}

/// Incoming slash command invocation
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SlashCommandInvocation {
    pub command_name: String,
    pub user_id: String,
    pub channel_id: String,
    pub guild_id: Option<String>,
    pub options: HashMap<String, String>,
    pub interaction_token: String,
}

/// Reaction event
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReactionEvent {
    pub user_id: String,
    pub channel_id: String,
    pub message_id: String,
    pub emoji: String,
    pub added: bool,
}

/// Discord bot presence/status
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum BotPresence {
    Playing(String),
    Listening(String),
    Watching(String),
    Competing(String),
    Custom(String),
}

// ── Handler ─────────────────────────────────────────────────────────────

/// Handler for Discord gateway events
struct Handler {
    tx: mpsc::Sender<ChannelMessage>,
    connected: Arc<AtomicBool>,
    allowed_guilds: Vec<u64>,
    slash_tx: Option<mpsc::Sender<SlashCommandInvocation>>,
    reaction_tx: Option<mpsc::Sender<ReactionEvent>>,
    policy: ChannelPolicy,
    bot_user_id: Arc<tokio::sync::RwLock<Option<u64>>>,
    /// Account ID for multi-bot routing (S35). Tags inbound messages
    /// so the gateway can dispatch to the correct agent session.
    account_id: Option<String>,
    /// Bot message filter mode (OpenClaw `allowBots` parity)
    allow_bots: AllowBotsMode,
    /// Role IDs this agent belongs to — for role mention matching
    role_ids: Vec<u64>,
    /// Bot username for text-based mention fallback detection
    bot_username: Arc<tokio::sync::RwLock<Option<String>>>,
    /// Message dedup: track recently seen message IDs to prevent duplicate
    /// processing when the same message arrives via multiple event paths.
    /// Stores (message_id, timestamp) pairs; entries older than 5s are pruned.
    seen_messages: Arc<Mutex<HashSet<u64>>>,
    seen_timestamps: Arc<Mutex<Vec<(u64, std::time::Instant)>>>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        tracing::info!(
            "Discord bot connected as {}#{}",
            ready.user.name,
            ready
                .user
                .discriminator
                .map(|d| d.to_string())
                .unwrap_or_default()
        );
        // Store our bot user ID and username for mention detection
        *self.bot_user_id.write().await = Some(ready.user.id.get());
        *self.bot_username.write().await = Some(ready.user.name.clone());
        self.connected.store(true, Ordering::SeqCst);
    }

    async fn message(&self, _ctx: Context, msg: DiscordMessage) {
        // Layer 0: Message dedup — skip if we've seen this message ID in the last 5 seconds.
        // Prevents duplicate processing when the same Discord event arrives via
        // multiple paths (e.g. real-time relay + context refresh).
        {
            let msg_id = msg.id.get();
            let now = std::time::Instant::now();
            let mut seen = self.seen_messages.lock().await;
            let mut timestamps = self.seen_timestamps.lock().await;

            // Prune entries older than 5 seconds
            let cutoff = now - std::time::Duration::from_secs(5);
            timestamps.retain(|(id, ts)| {
                if *ts < cutoff {
                    seen.remove(id);
                    false
                } else {
                    true
                }
            });

            // Check if already seen
            if !seen.insert(msg_id) {
                tracing::debug!(message_id = msg_id, "Discord dedup: skipping duplicate message");
                return;
            }
            timestamps.push((msg_id, now));
        }

        // Layer 1: Always ignore our own messages (prevents self-echo loops)
        if let Some(our_id) = *self.bot_user_id.read().await
            && msg.author.id.get() == our_id
        {
            return;
        }

        // Layer 2: Bot message filter (OpenClaw `allowBots` parity)
        if msg.author.bot {
            // @everyone/@here from bots always passes through — fleet broadcasts
            // need to reach all agents regardless of allow_bots setting.
            if !msg.mention_everyone {
                match self.allow_bots {
                    AllowBotsMode::Off => return,
                    AllowBotsMode::Mentions => {
                        let bot_id = *self.bot_user_id.read().await;
                        // Check 1: Discord structured mentions array (OpenClaw parity)
                        let is_mentioned = msg
                            .mentions
                            .iter()
                            .any(|u| bot_id.is_some_and(|id| u.id.get() == id));
                        // Check 2: reply-chain counts as implicit mention
                        let implicit_mention = msg
                            .referenced_message
                            .as_ref()
                            .map(|ref_msg| bot_id.is_some_and(|id| ref_msg.author.id.get() == id))
                            .unwrap_or(false);
                        // Text-based fallback: check if msg content contains our
                        // bot ID mention string or username (handles cases where
                        // Discord doesn't populate the mentions array for bot-to-bot)
                        let text_mention = {
                            let content_lower = msg.content.to_lowercase();
                            let id_match = bot_id
                                .map(|id| {
                                    content_lower.contains(&format!("<@{}>", id))
                                        || content_lower.contains(&format!("<@!{}>", id))
                                })
                                .unwrap_or(false);
                            let name_match = if let Some(ref name) = *self.bot_username.read().await {
                                content_lower.contains(&format!("@{}", name.to_lowercase()))
                            } else {
                                false
                            };
                            id_match || name_match
                        };
                        // Role mentions — only match if agent has configured role IDs
                        let role_mention = if self.role_ids.is_empty() {
                            false // no role IDs configured, ignore all role mentions
                        } else {
                            msg.mention_roles.iter().any(|rid| self.role_ids.contains(&rid.get()))
                        };
                        if !is_mentioned && !implicit_mention && !text_mention && !role_mention {
                            return;
                        }
                    }
                    AllowBotsMode::On => {} // allow all bot messages through
                }
            }
        }

        // Check guild allowlist if configured
        if !self.allowed_guilds.is_empty()
            && let Some(guild_id) = msg.guild_id
            && !self.allowed_guilds.contains(&guild_id.get())
        {
            return;
        }

        // Policy check: group vs DM filtering
        let is_group = msg.guild_id.is_some();
        // #66-L1: compute is_addressed once, plumb to ChannelMessage downstream.
        // DM is always addressed; in groups, @mention or slash-command counts.
        let is_dm = !is_group;
        let is_mention = if is_group {
            let bot_id = *self.bot_user_id.read().await;
            msg.mentions
                .iter()
                .any(|u| bot_id.is_some_and(|id| u.id.get() == id))
                || msg.content.starts_with('/')
        } else {
            false
        };
        let is_addressed = is_dm || is_mention;
        if is_group {
            let result = self.policy.check_group(
                &msg.channel_id.get().to_string(),
                &msg.author.id.get().to_string(),
                is_mention,
            );
            if result.is_denied() {
                return;
            }
        } else {
            let result = self.policy.check_dm(&msg.author.id.get().to_string());
            if result.is_denied() {
                return;
            }
        }

        // Detect thread context via guild channel lookup (requires GUILDS intent)
        let is_thread = if let Some(guild_id) = msg.guild_id {
            _ctx.cache
                .guild(guild_id)
                .and_then(|g| g.channels.get(&msg.channel_id).cloned())
                .map(|ch| {
                    matches!(
                        ch.kind,
                        ChannelType::PublicThread
                            | ChannelType::PrivateThread
                            | ChannelType::NewsThread
                    )
                })
                .unwrap_or(false)
        } else {
            false
        };

        let mut source = if is_thread {
            // Thread messages: set thread_id so gateway can route to thread-bound agent
            ChannelSource::with_chat(
                "discord",
                &msg.author.id.get().to_string(),
                &msg.channel_id.get().to_string(),
            )
            .with_thread(&msg.channel_id.get().to_string())
        } else {
            ChannelSource::with_chat(
                "discord",
                &msg.author.id.get().to_string(),
                &msg.channel_id.get().to_string(),
            )
        };

        // Classify sender type (S52 T3) — prevents bot-to-bot loops at classification level
        let sender_type = if msg.author.bot {
            zeus_core::SenderType::Bot
        } else if msg.webhook_id.is_some() {
            zeus_core::SenderType::System
        } else {
            zeus_core::SenderType::Human
        };
        source = source.with_sender_type(sender_type);

        // Collect attachments
        let attachments: Vec<ChannelAttachment> = msg
            .attachments
            .iter()
            .map(|a| {
                let mut att = ChannelAttachment::from_url(
                    &a.url,
                    a.content_type
                        .as_deref()
                        .unwrap_or("application/octet-stream"),
                );
                att.filename = Some(a.filename.clone());
                att
            })
            .collect();

        // Tag with account_id for multi-bot routing (S35)
        if let Some(ref acct_id) = self.account_id {
            source = source.with_account(acct_id);
        }

        // Resolve Discord mention IDs to readable usernames (S43)
        // <@123456> → @Username, so agents understand who is being addressed
        let resolved_content = {
            let mut content = msg.content.clone();
            for user in &msg.mentions {
                let mention_raw = format!("<@{}>", user.id.get());
                let mention_nick = format!("<@!{}>", user.id.get()); // nickname variant
                let replacement = format!("@{}", user.name);
                content = content.replace(&mention_raw, &replacement);
                content = content.replace(&mention_nick, &replacement);
            }
            // Prefix with author name so agents know who sent the message
            format!("[{}]: {}", msg.author.name, content)
        };

        let platform_msg_id = msg.id.get().to_string();
        let channel_message = if attachments.is_empty() {
            ChannelMessage::new(source, resolved_content)
                .with_platform_message_id(&platform_msg_id)
                .with_addressed(is_addressed)
        } else {
            ChannelMessage::with_attachments(source, resolved_content, attachments)
                .with_platform_message_id(&platform_msg_id)
                .with_addressed(is_addressed)
        };

        tracing::debug!(
            channel_id = msg.channel_id.get(),
            author = msg.author.name,
            content = msg.content,
            attachment_count = msg.attachments.len(),
            "Received Discord message"
        );

        if self.tx.send(channel_message).await.is_err() {
            tracing::warn!("Message receiver dropped");
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(cmd) = interaction {
            let invocation = SlashCommandInvocation {
                command_name: cmd.data.name.clone(),
                user_id: cmd.user.id.get().to_string(),
                channel_id: cmd.channel_id.get().to_string(),
                guild_id: cmd.guild_id.map(|g| g.get().to_string()),
                options: cmd
                    .data
                    .options
                    .iter()
                    .map(|o| (o.name.clone(), format!("{:?}", o.value)))
                    .collect(),
                interaction_token: cmd.token.clone(),
            };

            tracing::debug!(
                command = %invocation.command_name,
                user = %invocation.user_id,
                "Received slash command"
            );

            // Forward to slash command channel if configured
            if let Some(ref slash_tx) = self.slash_tx {
                let _ = slash_tx.send(invocation.clone()).await;
            }

            // Also forward as a regular message so the agent loop can process it
            let source =
                ChannelSource::with_chat("discord", &invocation.user_id, &invocation.channel_id);
            let content = format!(
                "/{} {}",
                invocation.command_name,
                invocation
                    .options
                    .iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join(" ")
            );

            // Slash commands are always explicitly addressed to the bot.
            let channel_message = ChannelMessage::new(source, content).with_addressed(true);
            let _ = self.tx.send(channel_message).await;

            // Send deferred response
            if let Err(e) = cmd
                .create_response(
                    &ctx.http,
                    serenity::all::CreateInteractionResponse::Defer(
                        serenity::all::CreateInteractionResponseMessage::new(),
                    ),
                )
                .await
            {
                tracing::error!(error = %e, "Failed to defer interaction response");
            }
        }
    }

    async fn reaction_add(&self, _ctx: Context, reaction: serenity::all::Reaction) {
        if let Some(ref reaction_tx) = self.reaction_tx {
            let event = ReactionEvent {
                user_id: reaction
                    .user_id
                    .map(|u| u.get().to_string())
                    .unwrap_or_default(),
                channel_id: reaction.channel_id.get().to_string(),
                message_id: reaction.message_id.get().to_string(),
                emoji: reaction.emoji.to_string(),
                added: true,
            };
            let _ = reaction_tx.send(event).await;
        }
    }

    async fn reaction_remove(&self, _ctx: Context, reaction: serenity::all::Reaction) {
        if let Some(ref reaction_tx) = self.reaction_tx {
            let event = ReactionEvent {
                user_id: reaction
                    .user_id
                    .map(|u| u.get().to_string())
                    .unwrap_or_default(),
                channel_id: reaction.channel_id.get().to_string(),
                message_id: reaction.message_id.get().to_string(),
                emoji: reaction.emoji.to_string(),
                added: false,
            };
            let _ = reaction_tx.send(event).await;
        }
    }
}

// ── Adapter ─────────────────────────────────────────────────────────────

/// Discord channel adapter using serenity
pub struct DiscordAdapter {
    config: DiscordConfig,
    http: Arc<RwLock<Option<Arc<serenity::http::Http>>>>,
    connected: Arc<AtomicBool>,
    shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    slash_commands: Arc<RwLock<Vec<SlashCommand>>>,
    slash_rx: Arc<Mutex<Option<mpsc::Receiver<SlashCommandInvocation>>>>,
    reaction_rx: Arc<Mutex<Option<mpsc::Receiver<ReactionEvent>>>>,
    slash_tx: Arc<Mutex<Option<mpsc::Sender<SlashCommandInvocation>>>>,
    reaction_tx: Arc<Mutex<Option<mpsc::Sender<ReactionEvent>>>>,
    /// Optional songbird voice manager for voice channel support
    songbird: Arc<RwLock<Option<Arc<songbird::Songbird>>>>,
}

impl DiscordAdapter {
    /// Create a new Discord adapter
    pub async fn new(config: DiscordConfig) -> Result<Self> {
        tracing::info!("Creating Discord adapter");

        let (slash_tx, slash_rx) = mpsc::channel(100);
        let (reaction_tx, reaction_rx) = mpsc::channel(100);

        Ok(Self {
            config,
            http: Arc::new(RwLock::new(None)),
            connected: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(Mutex::new(None)),
            slash_commands: Arc::new(RwLock::new(Vec::new())),
            songbird: Arc::new(RwLock::new(None)),
            slash_rx: Arc::new(Mutex::new(Some(slash_rx))),
            reaction_rx: Arc::new(Mutex::new(Some(reaction_rx))),
            slash_tx: Arc::new(Mutex::new(Some(slash_tx))),
            reaction_tx: Arc::new(Mutex::new(Some(reaction_tx))),
        })
    }

    /// Register a slash command (call before start())
    pub async fn register_slash_command(&self, cmd: SlashCommand) {
        self.slash_commands.write().await.push(cmd);
    }

    /// Take the slash command receiver (for processing commands externally)
    pub async fn take_slash_receiver(&self) -> Option<mpsc::Receiver<SlashCommandInvocation>> {
        self.slash_rx.lock().await.take()
    }

    /// Take the reaction event receiver
    pub async fn take_reaction_receiver(&self) -> Option<mpsc::Receiver<ReactionEvent>> {
        self.reaction_rx.lock().await.take()
    }

    /// Register all pending slash commands with Discord
    async fn register_commands_with_discord(&self) -> Result<()> {
        let http_guard = self.http.read().await;
        let http = match http_guard.as_ref() {
            Some(h) => h,
            None => return Ok(()), // Not connected yet
        };

        let commands = self.slash_commands.read().await;
        if commands.is_empty() {
            return Ok(());
        }

        let mut create_commands = Vec::new();
        for cmd in commands.iter() {
            let mut create_cmd = CreateCommand::new(&cmd.name).description(&cmd.description);
            for opt in &cmd.options {
                create_cmd = create_cmd.add_option(
                    CreateCommandOption::new(opt.kind.to_serenity(), &opt.name, &opt.description)
                        .required(opt.required),
                );
            }
            create_commands.push(create_cmd);
        }

        Command::set_global_commands(http.as_ref(), create_commands)
            .await
            .map_err(|e| Error::channel(format!("Failed to register slash commands: {}", e)))?;

        tracing::info!(count = commands.len(), "Registered Discord slash commands");
        Ok(())
    }

    /// Send a rich embed message
    pub async fn send_embed(
        &self,
        channel_id: u64,
        content: Option<&str>,
        embed: &DiscordEmbed,
    ) -> Result<()> {
        let http_guard = self.http.read().await;
        let http = http_guard
            .as_ref()
            .ok_or_else(|| Error::channel("Discord not connected"))?;

        let channel = ChannelId::new(channel_id);
        let mut msg = CreateMessage::new().embed(embed.to_serenity());
        if let Some(text) = content {
            msg = msg.content(text);
        }

        channel
            .send_message(http.as_ref(), msg)
            .await
            .map_err(|e| Error::channel(format!("Failed to send embed: {}", e)))?;

        tracing::debug!(channel_id, "Sent Discord embed");
        Ok(())
    }

    /// Convert audio data to OGG/Opus via ffmpeg for Discord voice messages
    async fn convert_to_ogg(data: &[u8], input_ext: &str) -> Result<Vec<u8>> {
        use tokio::process::Command;

        let input_path = format!("/tmp/zeus_voice_in.{}", input_ext);
        let output_path = "/tmp/zeus_voice_out.ogg";

        tokio::fs::write(&input_path, data)
            .await
            .map_err(|e| Error::channel(format!("Failed to write temp audio: {}", e)))?;

        // Try multiple ffmpeg paths (non-login shells may lack /opt/homebrew/bin)
        let ffmpeg = [
            "/opt/homebrew/bin/ffmpeg",
            "/usr/local/bin/ffmpeg",
            "ffmpeg",
        ]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .copied()
        .unwrap_or("ffmpeg");

        let output = Command::new(ffmpeg)
            .args([
                "-y",
                "-i",
                &input_path,
                "-c:a",
                "libopus",
                "-b:a",
                "64k",
                "-ar",
                "48000",
                "-ac",
                "1",
                output_path,
            ])
            .output()
            .await
            .map_err(|e| Error::channel(format!("ffmpeg not found or failed: {}", e)))?;

        let _ = tokio::fs::remove_file(&input_path).await;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = tokio::fs::remove_file(output_path).await;
            return Err(Error::channel(format!("ffmpeg conversion failed: {}", stderr)));
        }

        let ogg_data = tokio::fs::read(output_path)
            .await
            .map_err(|e| Error::channel(format!("Failed to read converted OGG: {}", e)))?;
        let _ = tokio::fs::remove_file(output_path).await;

        tracing::debug!(
            input_size = data.len(),
            output_size = ogg_data.len(),
            "Converted audio to OGG/Opus"
        );
        Ok(ogg_data)
    }

    /// Send a message with file attachments
    pub async fn send_with_attachments(
        &self,
        channel_id: u64,
        content: &str,
        files: Vec<(String, Vec<u8>)>,
    ) -> Result<()> {
        let http_guard = self.http.read().await;
        let http = http_guard
            .as_ref()
            .ok_or_else(|| Error::channel("Discord not connected"))?;

        let channel = ChannelId::new(channel_id);
        let mut msg = CreateMessage::new().content(content);

        for (filename, data) in files {
            msg = msg.add_file(CreateAttachment::bytes(data, filename));
        }

        channel
            .send_message(http.as_ref(), msg)
            .await
            .map_err(|e| Error::channel(format!("Failed to send with attachments: {}", e)))?;

        tracing::debug!(channel_id, "Sent Discord message with attachments");
        Ok(())
    }

    /// Create a thread from a message
    pub async fn create_thread(
        &self,
        channel_id: u64,
        name: &str,
        auto_archive_minutes: Option<u16>,
    ) -> Result<u64> {
        let http_guard = self.http.read().await;
        let http = http_guard
            .as_ref()
            .ok_or_else(|| Error::channel("Discord not connected"))?;

        let channel = ChannelId::new(channel_id);
        let mut builder = CreateThread::new(name);
        if let Some(mins) = auto_archive_minutes {
            builder = builder.auto_archive_duration(serenity::all::AutoArchiveDuration::from(mins));
        }

        // Create a public thread in the channel
        let thread = channel
            .create_thread(http.as_ref(), builder)
            .await
            .map_err(|e| Error::channel(format!("Failed to create thread: {}", e)))?;

        let thread_id = thread.id.get();
        tracing::debug!(channel_id, thread_id, name, "Created Discord thread");
        Ok(thread_id)
    }

    /// Send a message in a thread
    pub async fn send_in_thread(&self, thread_id: u64, content: &str) -> Result<()> {
        // Threads are just channels, so send normally
        let http_guard = self.http.read().await;
        let http = http_guard
            .as_ref()
            .ok_or_else(|| Error::channel("Discord not connected"))?;

        let channel = ChannelId::new(thread_id);
        channel
            .send_message(http.as_ref(), CreateMessage::new().content(content))
            .await
            .map_err(|e| Error::channel(format!("Failed to send in thread: {}", e)))?;

        tracing::debug!(thread_id, "Sent message in Discord thread");
        Ok(())
    }

    /// Reply to a specific message
    pub async fn reply_to_message(
        &self,
        channel_id: u64,
        message_id: u64,
        content: &str,
    ) -> Result<()> {
        let http_guard = self.http.read().await;
        let http = http_guard
            .as_ref()
            .ok_or_else(|| Error::channel("Discord not connected"))?;

        let channel = ChannelId::new(channel_id);
        let msg = CreateMessage::new()
            .content(content)
            .reference_message(MessageReference::from((
                channel,
                MessageId::new(message_id),
            )));

        channel
            .send_message(http.as_ref(), msg)
            .await
            .map_err(|e| Error::channel(format!("Failed to reply: {}", e)))?;

        tracing::debug!(channel_id, message_id, "Replied to Discord message");
        Ok(())
    }

    /// Add a reaction emoji to a message
    pub async fn react(&self, channel_id: u64, message_id: u64, emoji: &str) -> Result<()> {
        let http_guard = self.http.read().await;
        let http = http_guard
            .as_ref()
            .ok_or_else(|| Error::channel("Discord not connected"))?;

        let channel = ChannelId::new(channel_id);
        let msg_id = MessageId::new(message_id);
        let reaction = ReactionType::Unicode(emoji.to_string());

        channel
            .create_reaction(http.as_ref(), msg_id, reaction)
            .await
            .map_err(|e| Error::channel(format!("Failed to add reaction: {}", e)))?;

        tracing::debug!(channel_id, message_id, emoji, "Added Discord reaction");
        Ok(())
    }

    /// Remove a reaction emoji from a message (removes bot's own reaction)
    pub async fn unreact(&self, channel_id: u64, message_id: u64, emoji: &str) -> Result<()> {
        let http_guard = self.http.read().await;
        let http = http_guard
            .as_ref()
            .ok_or_else(|| Error::channel("Discord not connected"))?;

        let channel = ChannelId::new(channel_id);
        let msg_id = MessageId::new(message_id);
        let reaction = ReactionType::Unicode(emoji.to_string());

        channel
            .delete_reaction_emoji(http.as_ref(), msg_id, reaction)
            .await
            .map_err(|e| Error::channel(format!("Failed to remove reaction: {}", e)))?;

        tracing::debug!(channel_id, message_id, emoji, "Removed Discord reaction");
        Ok(())
    }

    /// Send a message and return the message ID (for later editing)
    pub async fn send_returning_id(&self, channel_id: u64, content: &str) -> Result<u64> {
        let http_guard = self.http.read().await;
        let http = http_guard
            .as_ref()
            .ok_or_else(|| Error::channel("Discord not connected"))?;

        let channel = ChannelId::new(channel_id);
        let msg = channel
            .send_message(http.as_ref(), CreateMessage::new().content(content))
            .await
            .map_err(|e| Error::channel(format!("Failed to send message: {}", e)))?;

        Ok(msg.id.get())
    }

    /// Edit an existing message by ID
    pub async fn edit_message_by_id(
        &self,
        channel_id: u64,
        message_id: u64,
        content: &str,
    ) -> Result<()> {
        let http_guard = self.http.read().await;
        let http = http_guard
            .as_ref()
            .ok_or_else(|| Error::channel("Discord not connected"))?;

        let channel = ChannelId::new(channel_id);
        let msg_id = MessageId::new(message_id);
        channel
            .edit_message(http.as_ref(), msg_id, EditMessage::new().content(content))
            .await
            .map_err(|e| Error::channel(format!("Failed to edit message: {}", e)))?;

        tracing::debug!(channel_id, message_id, "Edited Discord message");
        Ok(())
    }

    /// Get HTTP client reference (for advanced usage)
    pub async fn http(&self) -> Option<Arc<serenity::http::Http>> {
        self.http.read().await.clone()
    }

    /// Register a songbird voice manager for voice channel support.
    ///
    /// Must be called before `start()`. The songbird instance will be
    /// registered with the serenity client builder for gateway voice events.
    pub async fn set_songbird(&self, songbird: Arc<songbird::Songbird>) {
        *self.songbird.write().await = Some(songbird);
    }

    /// Get the songbird voice manager, if registered.
    pub async fn songbird(&self) -> Option<Arc<songbird::Songbird>> {
        self.songbird.read().await.clone()
    }
}

/// Split a message into chunks that fit Discord's 2000-char limit.
fn chunk_message(content: &str, max_len: usize) -> Vec<String> {
    if content.len() <= max_len {
        return vec![content.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = content;
    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }
        // Find a valid UTF-8 boundary at or before max_len
        let safe_max = {
            let mut i = max_len.min(remaining.len());
            while i > 0 && !remaining.is_char_boundary(i) {
                i -= 1;
            }
            i
        };
        if safe_max == 0 {
            // Shouldn't happen, but prevent infinite loop
            chunks.push(remaining.to_string());
            break;
        }
        let split_at = remaining[..safe_max]
            .rfind('\n')
            .or_else(|| remaining[..safe_max].rfind(' '))
            .unwrap_or(safe_max);
        let (chunk, rest) = remaining.split_at(split_at);
        chunks.push(chunk.to_string());
        remaining = rest.trim_start_matches('\n').trim_start();
    }
    chunks
}

#[async_trait]
impl ChannelAdapter for DiscordAdapter {
    fn channel_type(&self) -> &'static str {
        "discord"
    }

    fn account_id(&self) -> Option<&str> {
        self.config.account_id.as_deref()
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::WebSocket // Discord Gateway
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        let token = &self.config.bot_token;
        if token.is_empty() {
            return Err(Error::channel("Discord bot token is required"));
        }

        // Set up intents - full bot intents
        let intents = GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILD_MESSAGE_REACTIONS
            | GatewayIntents::DIRECT_MESSAGE_REACTIONS
            | GatewayIntents::GUILD_VOICE_STATES;

        // Take the senders for the handler
        let slash_tx = self.slash_tx.lock().await.take();
        let reaction_tx = self.reaction_tx.lock().await.take();

        let policy = ChannelPolicy::new(self.config.policy.clone().unwrap_or_default());
        let allow_bots = AllowBotsMode::from_config(self.config.allow_bots.as_deref());

        // Pre-decode bot user ID from the token so Layer 1 (self-echo filter)
        // is active immediately — before the `ready` event fires (OpenClaw parity).
        let pre_decoded_bot_id = decode_bot_id_from_token(token);
        if let Some(id) = pre_decoded_bot_id {
            tracing::debug!("Discord: pre-decoded bot_user_id={} from token", id);
        }

        // ── Owned-clones-before-loop (β-shape for outer reconnect loop) ────
        // `client.start()` consumes `Client`, so the outer loop must rebuild
        // the client + handler each iteration. All inputs to handler/builder
        // construction are cloned here so the spawn task can `async move`
        // without borrowing `&self`.
        let token_owned: String = token.clone();
        let tx_owned = tx;
        let slash_tx_owned = slash_tx;
        let reaction_tx_owned = reaction_tx;
        let connected = self.connected.clone();
        let allowed_guilds = self.config.allowed_guilds.clone();
        let account_id = self.config.account_id.clone();
        let role_ids: Vec<u64> = self
            .config
            .role_ids
            .iter()
            .filter_map(|s| s.parse::<u64>().ok())
            .collect();
        let songbird_clone = self.songbird.read().await.as_ref().cloned();
        let http_arc = self.http.clone();

        // ── First-iter setup outside loop (idempotent ops) ─────────────────
        // We need to populate self.http with a client.http BEFORE
        // register_commands_with_discord() can hit the Discord HTTP API.
        // Build a throwaway HTTP client for command registration (cheap —
        // serenity::http::Http is a thin reqwest wrapper with auth header).
        {
            let bootstrap_http = Arc::new(serenity::http::Http::new(&token_owned));
            let mut http_guard = self.http.write().await;
            *http_guard = Some(bootstrap_http);
        }

        // Register slash commands once (idempotent on Discord side).
        self.register_commands_with_discord().await?;

        // Create shutdown channel
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        {
            let mut shutdown_guard = self.shutdown.lock().await;
            *shutdown_guard = Some(shutdown_tx);
        }

        // ── Reconnect loop spawned in background task ──────────────────────
        // Each iteration:
        //   1. Build fresh Handler (mpsc::Sender clones; non-clone state is
        //      fresh per-connection by design — bot_user_id, seen_messages,
        //      seen_timestamps reset on reconnect, which is correct).
        //   2. Build fresh Client (consumes builder).
        //   3. Update self.http with new client.http for outbound sends.
        //   4. select! on client.start() vs shutdown_rx.
        //   5. On disconnect: exponential backoff 1s → 60s, then continue.
        //   6. On shutdown: shard_manager.shutdown_all() and break.
        tokio::spawn(async move {
            const BACKOFF_INITIAL_MS: u64 = 1_000;
            const BACKOFF_MAX_MS: u64 = 60_000;
            // A connection is considered "stable" if client.start() ran for at
            // least this long before returning. Stable connections reset the
            // backoff schedule on disconnect, so a healthy session followed by
            // a transient drop reconnects fast (1s) rather than inheriting an
            // accumulated backoff from earlier failures. Threshold chosen so
            // that gateway READY (typically 1-3s after socket open) plus some
            // operational margin counts as "actually connected."
            const STABLE_CONNECTION_THRESHOLD: std::time::Duration =
                std::time::Duration::from_secs(30);
            let mut backoff_ms = BACKOFF_INITIAL_MS;

            loop {
                // Rebuild handler for this connection iteration.
                let handler = Handler {
                    tx: tx_owned.clone(),
                    connected: connected.clone(),
                    allowed_guilds: allowed_guilds.clone(),
                    slash_tx: slash_tx_owned.clone(),
                    reaction_tx: reaction_tx_owned.clone(),
                    policy: policy.clone(),
                    bot_user_id: Arc::new(tokio::sync::RwLock::new(pre_decoded_bot_id)),
                    account_id: account_id.clone(),
                    allow_bots,
                    role_ids: role_ids.clone(),
                    bot_username: Arc::new(tokio::sync::RwLock::new(None)),
                    seen_messages: Arc::new(Mutex::new(HashSet::new())),
                    seen_timestamps: Arc::new(Mutex::new(Vec::new())),
                };

                // Build client (with optional songbird voice support).
                let mut builder =
                    SerenityClient::builder(&token_owned, intents).event_handler(handler);
                if let Some(songbird) = songbird_clone.as_ref() {
                    builder = builder.voice_manager_arc(songbird.clone());
                }

                let mut client = match builder.await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            backoff_ms,
                            "Discord client build failed; backing off before retry"
                        );
                        tokio::select! {
                            _ = tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)) => {}
                            _ = &mut shutdown_rx => {
                                tracing::info!("Discord shutdown received during build-backoff");
                                connected.store(false, Ordering::SeqCst);
                                return;
                            }
                        }
                        backoff_ms = (backoff_ms.saturating_mul(2)).min(BACKOFF_MAX_MS);
                        continue;
                    }
                };

                // Refresh self.http with the new client's HTTP for outbound sends.
                {
                    let mut http_guard = http_arc.write().await;
                    *http_guard = Some(client.http.clone());
                }

                tracing::info!("Discord client connecting (gateway start)");
                let connect_started_at = std::time::Instant::now();

                tokio::select! {
                    result = client.start() => {
                        connected.store(false, Ordering::SeqCst);
                        let connection_duration = connect_started_at.elapsed();
                        let was_stable = connection_duration >= STABLE_CONNECTION_THRESHOLD;
                        match result {
                            Ok(()) => {
                                tracing::warn!(
                                    backoff_ms,
                                    connection_duration_ms = connection_duration.as_millis() as u64,
                                    was_stable,
                                    "Discord client.start() returned Ok unexpectedly; reconnecting"
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    backoff_ms,
                                    connection_duration_ms = connection_duration.as_millis() as u64,
                                    was_stable,
                                    "Discord client error; reconnecting after backoff"
                                );
                            }
                        }
                        // Reset backoff if the connection was stable: a healthy
                        // session followed by a transient drop should reconnect
                        // fast, not inherit accumulated backoff from prior
                        // failures. Use the pre-reset value for the sleep below
                        // so we still pace the reconnect by one INITIAL_MS tick.
                        if was_stable {
                            tracing::info!(
                                connection_duration_ms = connection_duration.as_millis() as u64,
                                prior_backoff_ms = backoff_ms,
                                "Discord connection was stable; resetting reconnect backoff"
                            );
                            backoff_ms = BACKOFF_INITIAL_MS;
                        }
                        tokio::select! {
                            _ = tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)) => {}
                            _ = &mut shutdown_rx => {
                                tracing::info!("Discord shutdown received during reconnect-backoff");
                                return;
                            }
                        }
                        backoff_ms = (backoff_ms.saturating_mul(2)).min(BACKOFF_MAX_MS);
                        continue;
                    }
                    _ = &mut shutdown_rx => {
                        tracing::info!("Discord client shutting down");
                        client.shard_manager.shutdown_all().await;
                        connected.store(false, Ordering::SeqCst);
                        return;
                    }
                }
            }
        });

        tracing::info!("Discord adapter started (full bot mode)");
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

        // Clear HTTP client
        {
            let mut http_guard = self.http.write().await;
            *http_guard = None;
        }

        tracing::info!("Discord adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "discord" {
            return Err(Error::channel("Invalid channel source for Discord"));
        }

        let channel_id_str = to
            .chat_id
            .as_deref()
            .ok_or_else(|| Error::channel("Discord send requires a chat_id (channel_id)"))?;
        let channel_id_u64: u64 = channel_id_str.parse().map_err(|_| {
            Error::channel(format!("Invalid Discord channel_id: {}", channel_id_str))
        })?;

        let http_guard = self.http.read().await;
        let http = http_guard
            .as_ref()
            .ok_or_else(|| Error::channel("Discord not connected"))?;

        let channel = ChannelId::new(channel_id_u64);

        // Discord limit: 2000 chars - auto-chunk long messages
        let chunks = chunk_message(content, 1990);
        for chunk in &chunks {
            channel
                .send_message(http.as_ref(), CreateMessage::new().content(chunk))
                .await
                .map_err(|e| Error::channel(format!("Failed to send Discord message: {}", e)))?;
            if chunks.len() > 1 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }

        tracing::debug!(channel_id = channel_id_u64, chunks = chunks.len(), "Sent Discord message");
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn send_file(
        &self,
        to: &ChannelSource,
        filename: &str,
        data: &[u8],
        caption: Option<&str>,
    ) -> Result<()> {
        let channel_id_str = to
            .chat_id
            .as_deref()
            .ok_or_else(|| Error::channel("Discord send_file requires a chat_id"))?;
        let channel_id: u64 = channel_id_str.parse().map_err(|_| {
            Error::channel(format!("Invalid Discord channel_id: {}", channel_id_str))
        })?;

        let ext = std::path::Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let is_audio = matches!(
            ext.as_str(),
            "aiff" | "aif" | "wav" | "mp3" | "m4a" | "flac" | "ogg" | "opus"
        );

        // For audio files, convert to OGG/Opus and send as Discord voice message
        if is_audio {
            let (send_data, send_filename) = if ext == "ogg" || ext == "opus" {
                (data.to_vec(), filename.to_string())
            } else {
                // Convert to OGG via ffmpeg
                match Self::convert_to_ogg(data, &ext).await {
                    Ok(ogg_data) => {
                        let ogg_name = std::path::Path::new(filename)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("voice");
                        (ogg_data, format!("{}.ogg", ogg_name))
                    }
                    Err(e) => {
                        tracing::warn!("ffmpeg conversion failed, sending as-is: {}", e);
                        (data.to_vec(), filename.to_string())
                    }
                }
            };

            let http_guard = self.http.read().await;
            let http = http_guard
                .as_ref()
                .ok_or_else(|| Error::channel("Discord not connected"))?;

            let channel = ChannelId::new(channel_id);

            // Send OGG as regular audio attachment (Discord plays it inline)
            // IS_VOICE_MESSAGE requires waveform/duration metadata which serenity
            // doesn't expose yet, so we send as a playable audio file instead.
            let msg = CreateMessage::new()
                .content(caption.unwrap_or(""))
                .add_file(CreateAttachment::bytes(send_data, &send_filename));

            channel
                .send_message(http.as_ref(), msg)
                .await
                .map_err(|e| {
                    Error::channel(format!("Failed to send audio file: {}", e))
                })?;

            tracing::info!(channel_id, "Sent Discord audio file (OGG/Opus)");
            Ok(())
        } else {
            // Non-audio: send as regular file attachment
            self.send_with_attachments(
                channel_id,
                caption.unwrap_or(""),
                vec![(filename.to_string(), data.to_vec())],
            )
            .await
        }
    }

    async fn send_typing(&self, to: &ChannelSource) -> Result<()> {
        let channel_id_str = to.chat_id.as_deref().unwrap_or(&to.user_id);
        let channel_id: u64 = channel_id_str.parse().map_err(|_| {
            Error::channel(format!("Invalid Discord channel_id: {}", channel_id_str))
        })?;
        let http_guard = self.http.read().await;
        if let Some(http) = http_guard.as_ref() {
            let channel = serenity::model::id::ChannelId::new(channel_id);
            let _ = channel.broadcast_typing(http).await;
        }
        Ok(())
    }

    async fn set_presence(&self, status: crate::PresenceStatus) -> Result<()> {
        tracing::debug!(status = %status, "Discord presence update requested");
        Ok(())
    }

    fn supports_typing(&self) -> bool {
        true
    }
    fn supports_presence(&self) -> bool {
        true
    }

    async fn add_reaction(&self, channel_id: &str, message_id: &str, emoji: &str) -> Result<()> {
        let ch_id: u64 = channel_id
            .parse()
            .map_err(|_| Error::channel(format!("Invalid Discord channel_id: {}", channel_id)))?;
        let msg_id: u64 = message_id
            .parse()
            .map_err(|_| Error::channel(format!("Invalid Discord message_id: {}", message_id)))?;
        self.react(ch_id, msg_id, emoji).await
    }

    async fn remove_reaction(&self, channel_id: &str, message_id: &str, emoji: &str) -> Result<()> {
        let ch_id: u64 = channel_id
            .parse()
            .map_err(|_| Error::channel(format!("Invalid Discord channel_id: {}", channel_id)))?;
        let msg_id: u64 = message_id
            .parse()
            .map_err(|_| Error::channel(format!("Invalid Discord message_id: {}", message_id)))?;
        self.unreact(ch_id, msg_id, emoji).await
    }

    fn supports_reactions(&self) -> bool {
        true
    }

    fn supports_threading(&self) -> bool {
        true
    }

    /// Discord Tier 1: native per-message identity via webhook API.
    ///
    /// When `webhook_url` is configured, posts the message with the agent's
    /// `username` and `avatar_url` so each agent gets a distinct avatar.
    /// Falls back to the default text-prefix if no webhook URL is set.
    async fn send_as(
        &self,
        to: &ChannelSource,
        content: &str,
        identity: &crate::AgentSendIdentity,
    ) -> Result<()> {
        let webhook_url = match &self.config.webhook_url {
            Some(url) => url,
            None => {
                // No webhook configured — fall back to text-prefix (Tier 2)
                return self.send(to, &identity.apply_prefix(content)).await;
            }
        };

        // Build the webhook JSON payload
        let mut payload = serde_json::json!({
            "content": content,
            "username": identity.name,
        });

        if let Some(avatar) = &identity.avatar_url {
            payload["avatar_url"] = serde_json::json!(avatar);
        }

        let client = reqwest::Client::new();
        let resp = client
            .post(webhook_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::channel(format!("Discord webhook request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::channel(format!(
                "Discord webhook returned {status}: {body}"
            )));
        }

        tracing::debug!(
            agent = %identity.name,
            "Sent Discord message via webhook with agent identity"
        );
        Ok(())
    }

    /// Returns `true` when a webhook URL is configured (Tier 1 native identity).
    fn supports_native_identity(&self) -> bool {
        self.config.webhook_url.is_some()
    }
}

// ============================================================================
// Streaming delivery support (EditableChannel)
// ============================================================================

#[async_trait]
impl crate::streaming::EditableChannel for DiscordAdapter {
    async fn send_initial(&self, to: &ChannelSource, content: &str) -> Result<String> {
        let channel_id_str = to
            .chat_id
            .as_deref()
            .ok_or_else(|| Error::channel("Discord streaming requires a chat_id"))?;
        let channel_id: u64 = channel_id_str.parse().map_err(|_| {
            Error::channel(format!("Invalid Discord channel_id: {}", channel_id_str))
        })?;

        let msg_id = self.send_returning_id(channel_id, content).await?;
        Ok(msg_id.to_string())
    }

    async fn edit_message(&self, to: &ChannelSource, msg_id: &str, content: &str) -> Result<()> {
        let channel_id_str = to
            .chat_id
            .as_deref()
            .ok_or_else(|| Error::channel("Discord edit requires a chat_id"))?;
        let channel_id: u64 = channel_id_str.parse().map_err(|_| {
            Error::channel(format!("Invalid Discord channel_id: {}", channel_id_str))
        })?;
        let message_id: u64 = msg_id
            .parse()
            .map_err(|_| Error::channel("Invalid Discord message_id for edit"))?;

        self.edit_message_by_id(channel_id, message_id, content)
            .await
    }

    fn supports_editing(&self) -> bool {
        true
    }
}

impl DiscordAdapter {
    /// Send a streaming reply with coalesced message edits.
    ///
    /// Uses `StreamingDelivery` to batch rapid token chunks into fewer
    /// Discord message edits, providing a "typing" effect. Discord rate
    /// limits are ~5 requests/sec per channel for edits.
    ///
    /// # Parameters
    /// - `channel_id`: Target channel ID (or thread ID)
    /// - `rx`: Token stream receiver from LLM
    /// - `coalesce_ms`: Minimum milliseconds between edits (default: 800)
    /// - `min_chars`: Minimum characters before sending initial message (default: 50)
    pub async fn streaming_reply(
        &self,
        channel_id: &str,
        rx: &mut tokio::sync::mpsc::Receiver<String>,
        coalesce_ms: Option<u64>,
        min_chars: Option<usize>,
    ) -> Result<String> {
        // Discord rate limits are stricter than Telegram — default to 800ms
        let delivery = crate::streaming::StreamingDelivery::new()
            .with_coalesce_ms(coalesce_ms.unwrap_or(800))
            .with_min_chars(min_chars.unwrap_or(50));

        let to = ChannelSource::with_chat("discord", "relay", channel_id);

        delivery
            .deliver(
                Some(self as &dyn crate::streaming::EditableChannel),
                |content: &str| {
                    let content = content.to_string();
                    async move {
                        // Fallback: this shouldn't be called since we support editing
                        tracing::warn!("Discord streaming fallback send: {} chars", content.len());
                        Ok(())
                    }
                },
                &to,
                rx,
            )
            .await
    }
}

// ── Config ──────────────────────────────────────────────────────────────

/// Discord configuration
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct DiscordConfig {
    /// Bot token from Discord Developer Portal
    #[serde(default, skip_serializing)]
    pub bot_token: String,
    /// Application ID (optional, for slash commands)
    pub application_id: Option<u64>,
    /// Allowed guild IDs (empty = all guilds)
    #[serde(default)]
    pub allowed_guilds: Vec<u64>,
    /// Default channel ID for outbound messages
    #[serde(default)]
    pub default_channel_id: Option<u64>,
    /// Enable reaction tracking
    #[serde(default)]
    pub track_reactions: bool,
    /// Bot status message
    #[serde(default)]
    pub status_message: Option<String>,
    /// Webhook URL for per-message identity (Tier 1).
    /// When set, `send_as()` posts via Discord webhook API with per-message
    /// `username` and `avatar_url`, enabling distinct agent avatars.
    /// Format: `https://discord.com/api/webhooks/{id}/{token}`
    #[serde(default, skip_serializing)]
    pub webhook_url: Option<String>,
    /// Access policy (group mention filtering, DM access)
    #[serde(default)]
    pub policy: Option<zeus_core::ChannelPolicyConfig>,
    /// Account identifier for multi-bot routing (S35).
    /// When set, inbound messages are tagged with this ID via
    /// `ChannelSource::with_account()`, enabling gateway-level
    /// dispatch to the correct agent session.
    #[serde(default)]
    pub account_id: Option<String>,
    /// Bot message filter: "off" (default), "mentions", "on".
    /// OpenClaw `allowBots` parity.
    #[serde(default)]
    pub allow_bots: Option<String>,
    /// Discord role IDs this agent belongs to.
    /// Role mentions (<@&ID>) matching these IDs are treated as direct mentions.
    #[serde(default)]
    pub role_ids: Vec<String>,
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discord_config_default() {
        let config = DiscordConfig::default();
        assert!(config.bot_token.is_empty());
        assert!(config.allowed_guilds.is_empty());
        assert!(config.default_channel_id.is_none());
        assert!(!config.track_reactions);
    }

    #[tokio::test]
    async fn test_adapter_creation() {
        let config = DiscordConfig::default();
        let adapter = DiscordAdapter::new(config).await;
        assert!(adapter.is_ok());
    }

    #[test]
    fn test_embed_builder() {
        let embed = DiscordEmbed::new()
            .title("Test Title")
            .description("Test description")
            .color(0xFF0000)
            .field("Field 1", "Value 1", true)
            .field("Field 2", "Value 2", false)
            .footer("Footer text")
            .thumbnail("https://example.com/thumb.png")
            .image("https://example.com/image.png")
            .author("Test Author")
            .author_icon("https://example.com/icon.png")
            .url("https://example.com");

        assert_eq!(embed.title, Some("Test Title".to_string()));
        assert_eq!(embed.description, Some("Test description".to_string()));
        assert_eq!(embed.color, Some(0xFF0000));
        assert_eq!(embed.fields.len(), 2);
        assert!(embed.fields[0].inline);
        assert!(!embed.fields[1].inline);
        assert_eq!(embed.footer, Some("Footer text".to_string()));
        assert!(embed.thumbnail_url.is_some());
        assert!(embed.image_url.is_some());
        assert_eq!(embed.author_name, Some("Test Author".to_string()));
    }

    #[test]
    fn test_embed_to_serenity() {
        let embed = DiscordEmbed::new()
            .title("Hello")
            .description("World")
            .color(0x00FF00);
        // Should not panic
        let _ = embed.to_serenity();
    }

    #[test]
    fn test_slash_command_creation() {
        let cmd = SlashCommand {
            name: "ask".to_string(),
            description: "Ask Zeus a question".to_string(),
            options: vec![SlashCommandOption {
                name: "question".to_string(),
                description: "Your question".to_string(),
                kind: SlashOptionKind::String,
                required: true,
            }],
        };
        assert_eq!(cmd.name, "ask");
        assert_eq!(cmd.options.len(), 1);
        assert!(cmd.options[0].required);
    }

    #[test]
    fn test_slash_option_kind_to_serenity() {
        assert!(matches!(
            SlashOptionKind::String.to_serenity(),
            CommandOptionType::String
        ));
        assert!(matches!(
            SlashOptionKind::Integer.to_serenity(),
            CommandOptionType::Integer
        ));
        assert!(matches!(
            SlashOptionKind::Boolean.to_serenity(),
            CommandOptionType::Boolean
        ));
        assert!(matches!(
            SlashOptionKind::User.to_serenity(),
            CommandOptionType::User
        ));
        assert!(matches!(
            SlashOptionKind::Channel.to_serenity(),
            CommandOptionType::Channel
        ));
        assert!(matches!(
            SlashOptionKind::Role.to_serenity(),
            CommandOptionType::Role
        ));
    }

    #[test]
    fn test_reaction_event() {
        let event = ReactionEvent {
            user_id: "123".to_string(),
            channel_id: "456".to_string(),
            message_id: "789".to_string(),
            emoji: "👍".to_string(),
            added: true,
        };
        assert!(event.added);
        assert_eq!(event.emoji, "👍");
    }

    #[test]
    fn test_slash_command_invocation() {
        let mut options = HashMap::new();
        options.insert("question".to_string(), "What is Zeus?".to_string());
        let inv = SlashCommandInvocation {
            command_name: "ask".to_string(),
            user_id: "123".to_string(),
            channel_id: "456".to_string(),
            guild_id: Some("789".to_string()),
            options,
            interaction_token: "token".to_string(),
        };
        assert_eq!(inv.command_name, "ask");
        assert_eq!(inv.options["question"], "What is Zeus?");
    }

    #[tokio::test]
    async fn test_register_slash_command() {
        let config = DiscordConfig::default();
        let adapter = DiscordAdapter::new(config)
            .await
            .expect("DiscordAdapter::new should succeed");

        let cmd = SlashCommand {
            name: "test".to_string(),
            description: "Test command".to_string(),
            options: vec![],
        };
        adapter.register_slash_command(cmd).await;

        let commands = adapter.slash_commands.read().await;
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "test");
    }

    #[tokio::test]
    async fn test_take_receivers() {
        let config = DiscordConfig::default();
        let adapter = DiscordAdapter::new(config)
            .await
            .expect("DiscordAdapter::new should succeed");

        // First take should succeed
        assert!(adapter.take_slash_receiver().await.is_some());
        // Second take should return None (already taken)
        assert!(adapter.take_slash_receiver().await.is_none());

        assert!(adapter.take_reaction_receiver().await.is_some());
        assert!(adapter.take_reaction_receiver().await.is_none());
    }

    #[test]
    fn test_bot_presence_variants() {
        let p1 = BotPresence::Playing("a game".to_string());
        let p2 = BotPresence::Listening("music".to_string());
        let p3 = BotPresence::Watching("streams".to_string());
        let p4 = BotPresence::Competing("a tournament".to_string());
        let p5 = BotPresence::Custom("custom status".to_string());

        assert!(matches!(p1, BotPresence::Playing(_)));
        assert!(matches!(p2, BotPresence::Listening(_)));
        assert!(matches!(p3, BotPresence::Watching(_)));
        assert!(matches!(p4, BotPresence::Competing(_)));
        assert!(matches!(p5, BotPresence::Custom(_)));
    }

    #[test]
    fn test_embed_serialization() {
        let embed = DiscordEmbed::new()
            .title("Test")
            .description("Hello")
            .color(0xFF0000);

        let json = serde_json::to_string(&embed).expect("should serialize to JSON");
        let deserialized: DiscordEmbed =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(deserialized.title, Some("Test".to_string()));
        assert_eq!(deserialized.color, Some(0xFF0000));
    }

    #[test]
    fn test_discord_config_serialization() {
        let config = DiscordConfig {
            bot_token: "test-token".to_string(),
            application_id: Some(12345),
            allowed_guilds: vec![111, 222],
            default_channel_id: Some(333),
            track_reactions: true,
            status_message: Some("Thinking...".to_string()),
            ..Default::default()
        };

        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        // Credentials must not appear in serialized output
        assert!(
            !json.contains("test-token"),
            "bot_token must not be serialized"
        );
        let deserialized: DiscordConfig =
            serde_json::from_str(&json).expect("should parse successfully");
        // bot_token is intentionally empty after roundtrip (skip_serializing security policy)
        assert!(deserialized.bot_token.is_empty());
        assert_eq!(deserialized.application_id, Some(12345));
        assert_eq!(deserialized.allowed_guilds.len(), 2);
        assert!(deserialized.track_reactions);
        assert_eq!(deserialized.status_message, Some("Thinking...".to_string()));
    }

    #[test]
    fn test_webhook_url_not_serialized() {
        let config = DiscordConfig {
            bot_token: "tok".to_string(),
            webhook_url: Some("https://discord.com/api/webhooks/123/abc".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(
            !json.contains("webhooks"),
            "webhook_url must not be serialized (skip_serializing)"
        );
    }

    #[test]
    fn test_supports_native_identity_without_webhook() {
        // No webhook → Tier 2 (text-prefix)
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let config = DiscordConfig::default();
            let adapter = DiscordAdapter::new(config).await.unwrap();
            assert!(
                !adapter.supports_native_identity(),
                "Should be false without webhook_url"
            );
        });
    }

    #[test]
    fn test_supports_native_identity_with_webhook() {
        // Webhook configured → Tier 1 (native identity)
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let config = DiscordConfig {
                webhook_url: Some("https://discord.com/api/webhooks/123/abc".to_string()),
                ..Default::default()
            };
            let adapter = DiscordAdapter::new(config).await.unwrap();
            assert!(
                adapter.supports_native_identity(),
                "Should be true with webhook_url"
            );
        });
    }

    #[tokio::test]
    async fn test_send_as_webhook_posts_json() {
        // Spin up a wiremock server to capture the webhook POST
        let server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/"))
            .respond_with(wiremock::ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let config = DiscordConfig {
            webhook_url: Some(server.uri()),
            ..Default::default()
        };
        let adapter = DiscordAdapter::new(config).await.unwrap();

        let identity =
            crate::AgentSendIdentity::with_avatar("zeus106", "https://example.com/zeus106.png");
        let dest = ChannelSource::with_chat("discord", "user1", "12345");

        adapter
            .send_as(&dest, "hello from zeus106", &identity)
            .await
            .expect("webhook send_as should succeed");
    }

    #[tokio::test]
    async fn test_send_as_webhook_payload_structure() {
        let server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "content": "test message",
                "username": "fbsd1",
                "avatar_url": "https://example.com/fbsd1.png"
            })))
            .respond_with(wiremock::ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let config = DiscordConfig {
            webhook_url: Some(server.uri()),
            ..Default::default()
        };
        let adapter = DiscordAdapter::new(config).await.unwrap();

        let identity =
            crate::AgentSendIdentity::with_avatar("fbsd1", "https://example.com/fbsd1.png");
        let dest = ChannelSource::with_chat("discord", "u", "99");

        adapter
            .send_as(&dest, "test message", &identity)
            .await
            .expect("payload should match expected structure");
    }

    #[tokio::test]
    async fn test_send_as_without_avatar() {
        let server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "content": "no avatar",
                "username": "zeus107"
            })))
            .respond_with(wiremock::ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let config = DiscordConfig {
            webhook_url: Some(server.uri()),
            ..Default::default()
        };
        let adapter = DiscordAdapter::new(config).await.unwrap();

        let identity = crate::AgentSendIdentity::new("zeus107");
        let dest = ChannelSource::with_chat("discord", "u", "99");

        adapter
            .send_as(&dest, "no avatar", &identity)
            .await
            .expect("should send without avatar_url field");
    }

    #[tokio::test]
    async fn test_send_as_webhook_error_propagates() {
        let server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(401).set_body_string("Unauthorized"))
            .expect(1)
            .mount(&server)
            .await;

        let config = DiscordConfig {
            webhook_url: Some(server.uri()),
            ..Default::default()
        };
        let adapter = DiscordAdapter::new(config).await.unwrap();

        let identity = crate::AgentSendIdentity::new("bad-agent");
        let dest = ChannelSource::with_chat("discord", "u", "99");

        let result = adapter.send_as(&dest, "should fail", &identity).await;
        assert!(result.is_err(), "401 should propagate as error");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("401"),
            "Error should contain status code: {err_msg}"
        );
    }

    // -- #66-L1: is_addressed plumbing smoke tests --
    //
    // The full Handler::message path requires serenity's Context/Message which
    // can't be constructed standalone, so we cover the contract: ChannelMessage
    // round-trips with_addressed() per the discord plumb-through expectations.

    #[test]
    fn test_discord_channel_message_dm_is_addressed() {
        // DM path: is_group = false → is_addressed = true (DM always addressed).
        let source = ChannelSource::new("discord", "user-1");
        let msg = ChannelMessage::new(source, "hi".to_string()).with_addressed(true);
        assert_eq!(msg.is_addressed, Some(true));
    }

    #[test]
    fn test_discord_channel_message_mention_is_addressed() {
        // Group + @mention or slash: is_addressed = true.
        let source = ChannelSource::with_chat("discord", "user-1", "channel-99");
        let msg = ChannelMessage::new(source, "@bot hi".to_string()).with_addressed(true);
        assert_eq!(msg.is_addressed, Some(true));
    }

    #[test]
    fn test_discord_channel_message_unaddressed_group_msg() {
        // Group + no @mention + not slash: is_addressed = false.
        let source = ChannelSource::with_chat("discord", "user-1", "channel-99");
        let msg = ChannelMessage::new(source, "random chatter".to_string())
            .with_addressed(false);
        assert_eq!(msg.is_addressed, Some(false));
    }

    #[test]
    fn test_discord_slash_command_always_addressed() {
        // Slash commands are explicit invocations → always addressed = true.
        let source = ChannelSource::with_chat("discord", "user-1", "channel-99");
        let msg = ChannelMessage::new(source, "/status".to_string()).with_addressed(true);
        assert_eq!(msg.is_addressed, Some(true));
    }
}
