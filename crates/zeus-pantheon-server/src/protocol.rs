/// Pantheon WebSocket protocol — message types for the IRC-style agent collaboration server.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Shared types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionTier {
    /// Read-only — can receive messages, cannot send
    Observer,
    /// Standard member — can send/receive in joined channels
    Member,
    /// Can create channels, set topics, kick members
    Moderator,
    /// Full control — channel keys, bans, server config
    Admin,
}

impl Default for PermissionTier {
    fn default() -> Self {
        Self::Member
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub user_id: String,
    pub display_name: String,
    pub tier: PermissionTier,
    #[serde(default)]
    pub agent: bool,
}

// ── Client → Server messages ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Authenticate with a channel key.
    Auth {
        user_id: String,
        display_name: String,
        /// HMAC-SHA256(channel_key, user_id + ":" + nonce)
        token: String,
        nonce: String,
        #[serde(default)]
        agent: bool,
    },
    /// Join a channel.
    Join { channel: String },
    /// Leave a channel.
    Part { channel: String },
    /// Send a message to a channel.
    Msg {
        channel: String,
        content: String,
        #[serde(default)]
        message_type: MessageKind,
    },
    /// Set channel topic (requires Moderator+).
    Topic { channel: String, topic: String },
    /// Request presence list for a channel.
    Who { channel: String },
    /// Ping keepalive.
    Ping { nonce: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    #[default]
    Chat,
    System,
    ToolCall,
    TaskUpdate,
    PlanCard,
    DeployStatus,
}

// ── Server → Client messages ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Authentication result.
    AuthOk {
        user_id: String,
        tier: PermissionTier,
        /// All channels the user is auto-joined to.
        channels: Vec<String>,
    },
    AuthErr { reason: String },

    /// Confirmation that the client joined a channel.
    Joined {
        channel: String,
        /// Current member list.
        members: Vec<UserInfo>,
        topic: String,
    },
    /// Confirmation the client left.
    Parted { channel: String },

    /// Inbound chat message.
    Msg {
        id: Uuid,
        channel: String,
        from: UserInfo,
        content: String,
        message_type: MessageKind,
        ts: DateTime<Utc>,
    },

    /// Someone joined a channel you're in.
    PresenceJoin { channel: String, user: UserInfo },
    /// Someone left a channel you're in.
    PresencePart { channel: String, user_id: String },
    /// Presence status change (online/idle/offline).
    Presence {
        user_id: String,
        status: PresenceStatus,
    },

    /// Topic was updated.
    TopicUpdate { channel: String, topic: String, set_by: String },

    /// Response to WHO.
    WhoReply { channel: String, members: Vec<UserInfo> },

    /// Pong keepalive.
    Pong { nonce: String },

    /// Message of the day — sent after AUTH_OK (Phase 5).
    Motd { text: String },

    /// Generic error.
    Err { code: ErrorCode, message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus {
    Online,
    Idle,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    Unauthorized,
    Forbidden,
    NoSuchChannel,
    AlreadyInChannel,
    NotInChannel,
    RateLimited,
    InvalidMessage,
    ServerError,
}

impl ServerMessage {
    pub fn err(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Err { code, message: message.into() }
    }
}
