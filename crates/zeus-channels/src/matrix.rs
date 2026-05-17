//! Matrix channel adapter via matrix-sdk
//!
//! Uses the native matrix-sdk Rust crate for full Matrix protocol support.
//! Supports password login, access token restore, room joining, message
//! sending/receiving, and image uploads.

use crate::policy::ChannelPolicy;
use crate::sanitize::strip_markdown;
use async_trait::async_trait;
use matrix_sdk::{
    Client, Room, RoomState,
    config::SyncSettings,
    ruma::{
        OwnedRoomId, OwnedRoomOrAliasId, OwnedUserId,
        events::room::message::{
            MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
        },
    },
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};
use tracing::{error, info, warn};
use zeus_core::{Error, Result};

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};

/// Matrix configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixConfig {
    /// Homeserver URL (e.g., "https://matrix.org")
    #[serde(default = "default_homeserver")]
    pub homeserver: String,
    /// Username (e.g., "@bot:matrix.org" or just "bot")
    #[serde(default)]
    pub username: Option<String>,
    /// Password for password-based login
    #[serde(default, skip_serializing)]
    pub password: Option<String>,
    /// Access token for token-based auth (alternative to password login)
    #[serde(default, skip_serializing)]
    pub access_token: Option<String>,
    /// User ID (e.g., "@bot:matrix.org") — used with access_token restore
    #[serde(default)]
    pub user_id: Option<String>,
    /// Rooms to join/monitor (room IDs or aliases)
    #[serde(default)]
    pub rooms: Vec<String>,
    /// Display name to set after login
    #[serde(default)]
    pub display_name: Option<String>,
    /// Access policy (group mention filtering, DM access)
    #[serde(default)]
    pub policy: Option<zeus_core::ChannelPolicyConfig>,
    /// Account identifier for multi-account routing (S43)
    #[serde(default)]
    pub account_id: Option<String>,
    /// Bot message filter: "off" (default), "mentions", "on"
    /// Note: Matrix has no native bot flag; this applies best-effort heuristics only.
    #[serde(default)]
    pub allow_bots: Option<String>,
}

fn default_homeserver() -> String {
    "https://matrix.org".to_string()
}

impl Default for MatrixConfig {
    fn default() -> Self {
        Self {
            homeserver: default_homeserver(),
            username: None,
            password: None,
            access_token: None,
            user_id: None,
            rooms: Vec::new(),
            display_name: None,
            policy: None,
            account_id: None,
            allow_bots: None,
        }
    }
}

impl MatrixConfig {
    /// Merge environment variable overrides into a config.
    ///
    /// Checks:
    /// - `MATRIX_HOMESERVER` — overrides `homeserver`
    /// - `MATRIX_USER` — overrides `username`
    /// - `MATRIX_PASSWORD` — overrides `password`
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(hs) = std::env::var("MATRIX_HOMESERVER")
            && !hs.is_empty()
        {
            self.homeserver = hs;
        }
        if let Ok(user) = std::env::var("MATRIX_USER")
            && !user.is_empty()
        {
            self.username = Some(user);
        }
        if let Ok(pass) = std::env::var("MATRIX_PASSWORD")
            && !pass.is_empty()
        {
            self.password = Some(pass);
        }
        self
    }

    /// Apply env var overrides without consuming self (in-place mutation).
    pub fn apply_env_overrides(&mut self) {
        if let Ok(hs) = std::env::var("MATRIX_HOMESERVER")
            && !hs.is_empty()
        {
            self.homeserver = hs;
        }
        if let Ok(user) = std::env::var("MATRIX_USER")
            && !user.is_empty()
        {
            self.username = Some(user);
        }
        if let Ok(pass) = std::env::var("MATRIX_PASSWORD")
            && !pass.is_empty()
        {
            self.password = Some(pass);
        }
    }
}

/// Matrix channel adapter using matrix-sdk
pub struct MatrixAdapter {
    config: MatrixConfig,
    client: Arc<RwLock<Option<Client>>>,
    connected: Arc<AtomicBool>,
}

impl MatrixAdapter {
    /// Create a new Matrix adapter with the given configuration.
    ///
    /// Applies environment variable overrides (MATRIX_HOMESERVER, MATRIX_USER,
    /// MATRIX_PASSWORD) on top of the provided config.
    pub async fn new(config: MatrixConfig) -> Result<Self> {
        let config = config.with_env_overrides();
        Ok(Self {
            config,
            client: Arc::new(RwLock::new(None)),
            connected: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Build the matrix-sdk Client from the homeserver URL.
    async fn build_client(&self) -> Result<Client> {
        let homeserver = self.config.homeserver.trim_end_matches('/');
        Client::builder()
            .homeserver_url(homeserver)
            .build()
            .await
            .map_err(|e| Error::Channel(format!("Failed to build Matrix client: {e}")))
    }

    /// Login with username + password.
    async fn login_password(client: &Client, username: &str, password: &str) -> Result<()> {
        client
            .matrix_auth()
            .login_username(username, password)
            .send()
            .await
            .map_err(|e| Error::Channel(format!("Matrix password login failed: {e}")))?;
        Ok(())
    }

    /// Restore session from an access token.
    ///
    /// Requires both `access_token` and `user_id` to be set in config.
    async fn restore_token(client: &Client, access_token: &str, user_id: &str) -> Result<()> {
        use matrix_sdk::SessionMeta;
        use matrix_sdk::authentication::SessionTokens;
        use matrix_sdk::authentication::matrix::MatrixSession;

        let uid: OwnedUserId = user_id
            .try_into()
            .map_err(|e| Error::Channel(format!("Invalid Matrix user ID '{user_id}': {e}")))?;

        // Generate a device ID from the user ID for consistency
        let device_id_str = format!("zeus_{}", uid.localpart());
        let device_id: matrix_sdk::ruma::OwnedDeviceId = device_id_str.into();

        let session = MatrixSession {
            meta: SessionMeta {
                user_id: uid,
                device_id,
            },
            tokens: SessionTokens {
                access_token: access_token.to_string(),
                refresh_token: None,
            },
        };

        client
            .restore_session(session)
            .await
            .map_err(|e| Error::Channel(format!("Matrix session restore failed: {e}")))?;

        Ok(())
    }

    /// Join all rooms configured in `self.config.rooms`.
    async fn join_configured_rooms(client: &Client, rooms: &[String]) -> Vec<String> {
        let mut joined = Vec::new();
        for room_str in rooms {
            match Self::join_room_impl(client, room_str).await {
                Ok(_) => {
                    info!("Joined Matrix room: {}", room_str);
                    joined.push(room_str.clone());
                }
                Err(e) => {
                    warn!("Failed to join Matrix room '{}': {}", room_str, e);
                }
            }
        }
        joined
    }

    /// Internal: join a room by ID or alias string.
    async fn join_room_impl(client: &Client, room_id_or_alias: &str) -> Result<Room> {
        // Try parsing as a room ID first (starts with '!')
        if room_id_or_alias.starts_with('!') {
            let room_id: OwnedRoomId = room_id_or_alias.try_into().map_err(|e| {
                Error::Channel(format!("Invalid room ID '{room_id_or_alias}': {e}"))
            })?;
            let room = client.join_room_by_id(&room_id).await.map_err(|e| {
                Error::Channel(format!("Failed to join room {room_id_or_alias}: {e}"))
            })?;
            return Ok(room);
        }

        // Otherwise treat as room alias or RoomOrAliasId
        let room_or_alias: OwnedRoomOrAliasId = room_id_or_alias.try_into().map_err(|e| {
            Error::Channel(format!(
                "Invalid room ID or alias '{room_id_or_alias}': {e}"
            ))
        })?;
        let room = client
            .join_room_by_id_or_alias(&room_or_alias, &[])
            .await
            .map_err(|e| Error::Channel(format!("Failed to join room {room_id_or_alias}: {e}")))?;
        Ok(room)
    }

    /// Join a room by ID or alias (public API).
    ///
    /// The adapter must be started (connected) before calling this.
    pub async fn join_room(&self, room_id_or_alias: &str) -> Result<()> {
        let guard = self.client.read().await;
        let client = guard
            .as_ref()
            .ok_or_else(|| Error::Channel("Matrix client not connected".to_string()))?;
        Self::join_room_impl(client, room_id_or_alias).await?;
        Ok(())
    }

    /// Upload and send an image to a Matrix room.
    ///
    /// The adapter must be started (connected) before calling this.
    pub async fn send_image(
        &self,
        room_id: &str,
        data: Vec<u8>,
        filename: &str,
        mime_type: &str,
    ) -> Result<()> {
        let guard = self.client.read().await;
        let client = guard
            .as_ref()
            .ok_or_else(|| Error::Channel("Matrix client not connected".to_string()))?;

        let rid: OwnedRoomId = room_id
            .try_into()
            .map_err(|e| Error::Channel(format!("Invalid room ID '{room_id}': {e}")))?;

        let room = client
            .get_room(&rid)
            .ok_or_else(|| Error::Channel(format!("Room not found: {room_id}")))?;

        if room.state() != RoomState::Joined {
            return Err(Error::Channel(format!("Not joined to room: {room_id}")));
        }

        let content_type: mime::Mime = mime_type
            .parse()
            .map_err(|e| Error::Channel(format!("Invalid MIME type '{mime_type}': {e}")))?;

        room.send_attachment(
            filename,
            &content_type,
            data,
            matrix_sdk::attachment::AttachmentConfig::new(),
        )
        .await
        .map_err(|e| Error::Channel(format!("Failed to send image to {room_id}: {e}")))?;

        Ok(())
    }

    /// Get a reference to the config (for testing / introspection).
    pub fn config(&self) -> &MatrixConfig {
        &self.config
    }

    /// Helper: build the API URL for a path.
    #[cfg(test)]
    fn api_url(&self, path: &str) -> String {
        format!(
            "{}/_matrix/client/v3{}",
            self.config.homeserver.trim_end_matches('/'),
            path
        )
    }
}

#[async_trait]
impl ChannelAdapter for MatrixAdapter {
    fn channel_type(&self) -> &'static str {
        "matrix"
    }

    fn account_id(&self) -> Option<&str> {
        self.config.account_id.as_deref()
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Native
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        // Build client
        let client = self.build_client().await?;

        // Authenticate
        if let Some(ref password) = self.config.password {
            let username = self.config.username.as_deref().ok_or_else(|| {
                Error::Channel("Matrix username required for password login".to_string())
            })?;
            Self::login_password(&client, username, password).await?;
        } else if let Some(ref token) = self.config.access_token {
            let user_id = self.config.user_id.as_deref().ok_or_else(|| {
                Error::Channel("Matrix user_id required when using access_token auth".to_string())
            })?;
            Self::restore_token(&client, token, user_id).await?;
        } else {
            return Err(Error::Channel(
                "Matrix authentication required: set password or access_token".to_string(),
            ));
        }

        // Optionally set display name
        if let Some(ref name) = self.config.display_name
            && let Err(e) = client.account().set_display_name(Some(name)).await
        {
            warn!("Failed to set Matrix display name: {}", e);
        }

        // Join configured rooms
        Self::join_configured_rooms(&client, &self.config.rooms).await;

        // Store client
        {
            let mut guard = self.client.write().await;
            *guard = Some(client.clone());
        }

        self.connected.store(true, Ordering::SeqCst);

        // Get our own user ID to filter outgoing messages
        let own_user_id = client.user_id().map(|u| u.to_owned());

        // Policy + display name for mention detection
        let policy_config = self.config.policy.clone();
        let bot_display_name = self.config.display_name.clone();
        let account_id = self.config.account_id.clone();
        let allow_bots = self.config.allow_bots.clone();

        // Register event handler for incoming room messages
        let tx_clone = tx.clone();
        let connected = self.connected.clone();
        client.add_event_handler(move |event: OriginalSyncRoomMessageEvent, room: Room| {
            let tx = tx_clone.clone();
            let connected = connected.clone();
            let own_user_id = own_user_id.clone();
            let policy_config = policy_config.clone();
            let bot_display_name = bot_display_name.clone();
            let account_id = account_id.clone();
            let allow_bots = allow_bots.clone();
            async move {
                // Layer 1: self-echo — skip our own messages
                if let Some(ref our_id) = own_user_id
                    && event.sender == *our_id
                {
                    return;
                }

                // Layer 2: AllowBotsMode — Matrix has no native bot flag;
                // heuristic: sender localpart contains "bot" (best-effort).
                let sender_str = event.sender.to_string();
                let localpart = sender_str
                    .trim_start_matches('@')
                    .split(':')
                    .next()
                    .unwrap_or("");
                let looks_like_bot = localpart.contains("bot");
                if looks_like_bot {
                    match crate::filters::AllowBotsMode::from_config(allow_bots.as_deref()) {
                        crate::filters::AllowBotsMode::Off => return,
                        crate::filters::AllowBotsMode::Mentions => {
                            // Allow only if bot is explicitly addressed by display name or @id
                            let body_preview = match &event.content.msgtype {
                                MessageType::Text(t) => t.body.as_str(),
                                MessageType::Notice(n) => n.body.as_str(),
                                MessageType::Emote(e) => e.body.as_str(),
                                _ => "",
                            };
                            let is_mention = own_user_id
                                .as_ref()
                                .is_some_and(|uid| body_preview.contains(uid.as_str()))
                                || bot_display_name
                                    .as_ref()
                                    .is_some_and(|name| body_preview.to_lowercase().contains(&name.to_lowercase()));
                            if !is_mention {
                                return;
                            }
                        }
                        crate::filters::AllowBotsMode::On => {}
                    }
                }

                // Only process messages from joined rooms
                if room.state() != RoomState::Joined {
                    return;
                }

                // Extract text body from message
                let body = match event.content.msgtype {
                    MessageType::Text(text) => text.body,
                    MessageType::Notice(notice) => notice.body,
                    MessageType::Emote(emote) => emote.body,
                    _ => return, // Ignore non-text message types
                };

                if body.is_empty() {
                    return;
                }

                let room_id = room.room_id().to_string();
                let sender = event.sender.to_string();

                // Policy check: detect DM (2-member room) vs group
                let policy = ChannelPolicy::new(policy_config.unwrap_or_default());
                let is_dm = room.joined_members_count() <= 2;

                // Robust mention detection (Discord parity): DMs always addressed,
                // groups addressed on @user_id, display name, or / command
                let is_addressed = is_dm
                    || own_user_id.as_ref().is_some_and(|uid| body.contains(uid.as_str()))
                    || bot_display_name.as_ref().is_some_and(|name| body.to_lowercase().contains(&name.to_lowercase()))
                    || body.starts_with('/');

                if is_dm {
                    let result = policy.check_dm(&sender);
                    if result.is_denied() {
                        return;
                    }
                } else {
                    // Group room: check for mention (display name or @user_id in body)
                    let is_mention = own_user_id
                        .as_ref()
                        .is_some_and(|uid| body.contains(uid.as_str()))
                        || bot_display_name
                            .as_ref()
                            .is_some_and(|name| body.to_lowercase().contains(&name.to_lowercase()))
                        || body.starts_with('/');
                    let result = policy.check_group(&room_id, &sender, is_mention);
                    if result.is_denied() {
                        return;
                    }
                }

                let mut source = ChannelSource::with_chat("matrix", &sender, &room_id);
                if let Some(ref acct_id) = account_id {
                    source = source.with_account(acct_id);
                }
                let msg = ChannelMessage::new(source, body).with_addressed(is_addressed);

                if tx.send(msg).await.is_err() {
                    connected.store(false, Ordering::SeqCst);
                }
            }
        });

        // Spawn sync loop in background
        let connected = self.connected.clone();
        tokio::spawn(async move {
            info!("Starting Matrix sync loop");

            let settings = SyncSettings::default().timeout(Duration::from_secs(30));

            // Run sync loop — this blocks until an error or shutdown
            loop {
                if !connected.load(Ordering::SeqCst) {
                    break;
                }

                match client.sync_once(settings.clone()).await {
                    Ok(_response) => {
                        // Sync succeeded; event handlers already fired
                    }
                    Err(e) => {
                        if !connected.load(Ordering::SeqCst) {
                            break;
                        }
                        error!("Matrix sync error: {}", e);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }

            connected.store(false, Ordering::SeqCst);
            info!("Matrix sync loop stopped");
        });

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        // Clear client reference
        let mut guard = self.client.write().await;
        *guard = None;
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        // Matrix m.text events render as plain text (m.notice/m.text without
        // formatted_body). Strip Markdown to avoid leaking raw `**bold**`,
        // `[text](url)`, etc. into the user's view.
        // Mirror-symmetric to Telegram/IRC adapter-layer sanitize (d9bef524).
        let content = strip_markdown(content);

        let guard = self.client.read().await;
        let client = guard
            .as_ref()
            .ok_or_else(|| Error::Channel("Matrix client not connected".to_string()))?;

        let room_id_str = to.chat_id.as_deref().unwrap_or(&to.user_id);
        let room_id: OwnedRoomId = room_id_str
            .try_into()
            .map_err(|e| Error::Channel(format!("Invalid room ID '{room_id_str}': {e}")))?;

        let room = client
            .get_room(&room_id)
            .ok_or_else(|| Error::Channel(format!("Room not found: {room_id_str}")))?;

        if room.state() != RoomState::Joined {
            return Err(Error::Channel(format!("Not joined to room: {room_id_str}")));
        }

        let msg_content = RoomMessageEventContent::text_plain(content);
        room.send(msg_content)
            .await
            .map_err(|e| Error::Channel(format!("Matrix send failed: {e}")))?;

        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn supports_native_identity(&self) -> bool {
        false
    }

    async fn send_typing(&self, to: &ChannelSource) -> Result<()> {
        let room_id_str = to.chat_id.as_deref().unwrap_or(&to.user_id);
        let client_guard = self.client.read().await;
        if let Some(client) = client_guard.as_ref()
            && let Ok(room_id) =
                <matrix_sdk::ruma::OwnedRoomId as std::str::FromStr>::from_str(room_id_str)
            && let Some(room) = client.get_room(&room_id)
        {
            let _ = room.typing_notice(true).await;
        }
        Ok(())
    }

    fn supports_typing(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentSendIdentity;

    #[test]
    fn test_matrix_config_defaults() {
        let config = MatrixConfig::default();
        assert_eq!(config.homeserver, "https://matrix.org");
        assert!(config.username.is_none());
        assert!(config.password.is_none());
        assert!(config.access_token.is_none());
        assert!(config.user_id.is_none());
        assert!(config.rooms.is_empty());
        assert!(config.display_name.is_none());
    }

    #[test]
    fn test_matrix_config_serde_roundtrip() {
        let config = MatrixConfig {
            homeserver: "https://example.com".to_string(),
            username: Some("@bot:example.com".to_string()),
            password: Some("secret".to_string()),
            access_token: None,
            user_id: Some("@bot:example.com".to_string()),
            rooms: vec![
                "!abc:example.com".to_string(),
                "#general:example.com".to_string(),
            ],
            display_name: Some("Zeus Bot".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        // Credentials must not appear in serialized output
        assert!(!json.contains("secret"), "password must not be serialized");
        let deserialized: MatrixConfig =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(deserialized.homeserver, "https://example.com");
        assert_eq!(deserialized.username.as_deref(), Some("@bot:example.com"));
        // Password is intentionally None after roundtrip (skip_serializing security policy)
        assert!(deserialized.password.is_none());
        assert!(deserialized.access_token.is_none());
        assert_eq!(deserialized.user_id.as_deref(), Some("@bot:example.com"));
        assert_eq!(deserialized.rooms.len(), 2);
        assert_eq!(deserialized.display_name.as_deref(), Some("Zeus Bot"));
    }

    #[test]
    fn test_matrix_config_env_override_merge() {
        // Test the merge logic directly without setting real env vars.
        // We verify that with_env_overrides preserves fields when env vars
        // are not set (which is the normal test environment).
        let config = MatrixConfig {
            homeserver: "https://my.server.com".to_string(),
            username: Some("myuser".to_string()),
            password: Some("mypass".to_string()),
            ..Default::default()
        };

        // In test environment, env vars are typically not set, so fields are preserved
        let merged = config.with_env_overrides();
        // If MATRIX_HOMESERVER is not set, the original value is kept
        // (We can't guarantee env vars aren't set, so just check it's non-empty)
        assert!(!merged.homeserver.is_empty());
        // At minimum the username/password should be present (if env vars are unset)
        assert!(merged.username.is_some());
        assert!(merged.password.is_some());
    }

    #[test]
    fn test_matrix_config_apply_env_overrides_in_place() {
        let mut config = MatrixConfig {
            homeserver: "https://original.server".to_string(),
            ..Default::default()
        };
        config.apply_env_overrides();
        // Without env vars set, the original value is preserved
        assert!(!config.homeserver.is_empty());
    }

    #[tokio::test]
    async fn test_matrix_adapter_creation() {
        let config = MatrixConfig::default();
        let adapter = MatrixAdapter::new(config).await;
        assert!(adapter.is_ok());
        let adapter = adapter.expect("operation should succeed");
        assert_eq!(adapter.channel_type(), "matrix");
        assert!(!adapter.is_connected());
    }

    #[test]
    fn test_matrix_channel_type() {
        // Verify the static channel type string
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build should succeed");
        rt.block_on(async {
            let config = MatrixConfig::default();
            let adapter = MatrixAdapter::new(config)
                .await
                .expect("MatrixAdapter::new should succeed");
            assert_eq!(adapter.channel_type(), "matrix");
        });
    }

    #[test]
    fn test_matrix_receive_mode() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build should succeed");
        rt.block_on(async {
            let config = MatrixConfig::default();
            let adapter = MatrixAdapter::new(config)
                .await
                .expect("MatrixAdapter::new should succeed");
            let mode = adapter.receive_mode();
            assert!(matches!(mode, ReceiveMode::Native));
            assert!(mode.can_receive());
            assert!(!mode.needs_webhook_server());
        });
    }

    #[test]
    fn test_matrix_config_with_password() {
        let config = MatrixConfig {
            homeserver: "https://matrix.example.com".to_string(),
            username: Some("bot".to_string()),
            password: Some("hunter2".to_string()),
            ..Default::default()
        };
        assert_eq!(config.homeserver, "https://matrix.example.com");
        assert_eq!(config.username.as_deref(), Some("bot"));
        assert_eq!(config.password.as_deref(), Some("hunter2"));
        assert!(config.access_token.is_none());
    }

    #[test]
    fn test_matrix_config_with_access_token() {
        let config = MatrixConfig {
            homeserver: "https://matrix.example.com".to_string(),
            access_token: Some("syt_token_here".to_string()),
            user_id: Some("@bot:matrix.example.com".to_string()),
            ..Default::default()
        };
        assert!(config.password.is_none());
        assert_eq!(config.access_token.as_deref(), Some("syt_token_here"));
        assert_eq!(config.user_id.as_deref(), Some("@bot:matrix.example.com"));
    }

    #[test]
    fn test_matrix_config_with_rooms() {
        let config = MatrixConfig {
            rooms: vec![
                "!abc123:matrix.org".to_string(),
                "#general:matrix.org".to_string(),
                "!xyz789:example.com".to_string(),
            ],
            ..Default::default()
        };
        assert_eq!(config.rooms.len(), 3);
        assert_eq!(config.rooms[0], "!abc123:matrix.org");
        assert_eq!(config.rooms[1], "#general:matrix.org");
        assert_eq!(config.rooms[2], "!xyz789:example.com");
    }

    #[test]
    fn test_matrix_channel_source_creation() {
        let source = ChannelSource::with_chat("matrix", "@user:matrix.org", "!room123:matrix.org");
        assert_eq!(source.channel_type(), "matrix");
        assert_eq!(source.user_id, "@user:matrix.org");
        assert_eq!(source.chat_id.as_deref(), Some("!room123:matrix.org"));
    }

    #[tokio::test]
    async fn test_matrix_adapter_not_connected_by_default() {
        let config = MatrixConfig {
            homeserver: "https://matrix.org".to_string(),
            username: Some("testbot".to_string()),
            password: Some("testpass".to_string()),
            rooms: vec!["!test:matrix.org".to_string()],
            ..Default::default()
        };
        let adapter = MatrixAdapter::new(config)
            .await
            .expect("MatrixAdapter::new should succeed");
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_matrix_adapter_config_preserved() {
        let config = MatrixConfig {
            homeserver: "https://custom.server".to_string(),
            username: Some("mybot".to_string()),
            display_name: Some("My Bot".to_string()),
            rooms: vec!["!room:custom.server".to_string()],
            ..Default::default()
        };
        let adapter = MatrixAdapter::new(config)
            .await
            .expect("MatrixAdapter::new should succeed");
        // If MATRIX_HOMESERVER env var is not set, the configured value is preserved
        let cfg = adapter.config();
        assert!(cfg.username.is_some());
        assert_eq!(cfg.display_name.as_deref(), Some("My Bot"));
        assert_eq!(cfg.rooms.len(), 1);
    }

    #[test]
    fn test_matrix_config_serde_partial() {
        // Deserialize with only some fields set — defaults fill in the rest
        let json = r#"{"homeserver":"https://example.com","rooms":["!abc:example.com"]}"#;
        let config: MatrixConfig = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(config.homeserver, "https://example.com");
        assert!(config.username.is_none());
        assert!(config.password.is_none());
        assert!(config.access_token.is_none());
        assert_eq!(config.rooms.len(), 1);
    }

    #[tokio::test]
    async fn test_matrix_api_url_helper() {
        let config = MatrixConfig {
            homeserver: "https://my.matrix.server".to_string(),
            ..Default::default()
        };
        let adapter = MatrixAdapter::new(config)
            .await
            .expect("MatrixAdapter::new should succeed");
        let url = adapter.api_url("/sync");
        assert_eq!(url, "https://my.matrix.server/_matrix/client/v3/sync");
    }

    #[tokio::test]
    async fn test_matrix_api_url_trailing_slash() {
        let config = MatrixConfig {
            homeserver: "https://my.matrix.server/".to_string(),
            ..Default::default()
        };
        let adapter = MatrixAdapter::new(config)
            .await
            .expect("MatrixAdapter::new should succeed");
        let url = adapter.api_url("/sync");
        assert_eq!(url, "https://my.matrix.server/_matrix/client/v3/sync");
    }

    // ── S43: Multi-account tests ──────────────────────────────────────────

    #[tokio::test]
    async fn test_matrix_account_id_none_by_default() {
        let config = MatrixConfig::default();
        let adapter = MatrixAdapter::new(config).await.expect("should create");
        assert!(adapter.account_id().is_none());
    }

    #[tokio::test]
    async fn test_matrix_account_id_set() {
        let config = MatrixConfig {
            account_id: Some("support".to_string()),
            ..Default::default()
        };
        let adapter = MatrixAdapter::new(config).await.expect("should create");
        assert_eq!(adapter.account_id(), Some("support"));
    }

    #[test]
    fn test_matrix_config_with_account_id_serde() {
        let json = r#"{"homeserver":"https://matrix.org","account_id":"bot1","allow_bots":"mentions"}"#;
        let config: MatrixConfig = serde_json::from_str(json).expect("should parse");
        assert_eq!(config.account_id.as_deref(), Some("bot1"));
        assert_eq!(config.allow_bots.as_deref(), Some("mentions"));
    }

    // ── S33 Track D: Tier 2 identity tests ──────────────────────────────────

    #[tokio::test]
    async fn test_matrix_supports_native_identity_false() {
        let config = MatrixConfig {
            homeserver: "https://matrix.org".to_string(),
            username: Some("testbot".to_string()),
            password: Some("testpass".to_string()),
            ..Default::default()
        };
        let adapter = MatrixAdapter::new(config)
            .await
            .expect("MatrixAdapter::new should succeed");
        assert!(!adapter.supports_native_identity());
    }

    #[test]
    fn test_matrix_send_as_text_prefix_format() {
        let identity = AgentSendIdentity::new("zeus_agent");
        let prefixed = identity.apply_prefix("Hello from Matrix");
        assert_eq!(prefixed, "[zeus_agent] Hello from Matrix");
    }
}
