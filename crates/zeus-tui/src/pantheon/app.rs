//! Pantheon app state — shared types for the IRC-style agent communication network.
//!
//! All widgets (`channel_list`, `message_view`, `user_list`, `chat`) import
//! from here to avoid circular dependencies. The types are deliberately simple
//! (no async, no Arc/Mutex) — the Pantheon TUI is single-threaded ratatui.

/// Root state for the Pantheon tab.
pub struct PantheonApp {
    /// All joined channels, ordered by join time.
    pub channels: Vec<IrcChannel>,
    /// Index into `channels` for the currently-viewed channel.
    pub active_channel: usize,
    /// This agent's IRC nick (e.g. "ZeusMolty").
    pub nick: String,
    /// Whether the Pantheon WebSocket/gateway connection is alive.
    pub connected: bool,
    /// Raw input buffer for the input bar (pre-send).
    pub input: String,
}

impl PantheonApp {
    /// Get a reference to the active channel, or `None` if the channel
    /// list is empty.
    pub fn active_channel(&self) -> Option<&IrcChannel> {
        self.channels.get(self.active_channel)
    }

    /// Mutable reference to the active channel.
    pub fn active_channel_mut(&mut self) -> Option<&mut IrcChannel> {
        self.channels.get_mut(self.active_channel)
    }

    /// Create a fresh PantheonApp with the default channel list and no messages.
    pub fn new(nick: String) -> Self {
        let channels = super::config::DEFAULT_CHANNELS
            .iter()
            .map(|name| IrcChannel::new(name))
            .collect();
        Self {
            channels,
            active_channel: 0,
            nick,
            connected: false,
            input: String::new(),
        }
    }
}

/// A single IRC-style channel.
pub struct IrcChannel {
    /// Channel name including the prefix (e.g. "#general", "#ops-alerts").
    pub name: String,
    /// Current topic line, if set.
    pub topic: Option<String>,
    /// Number of unread messages since the user last viewed this channel.
    pub unread: usize,
    /// Message history (oldest first).
    pub messages: Vec<IrcMessage>,
    /// Users currently in this channel.
    pub users: Vec<IrcUser>,
}

impl IrcChannel {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            topic: None,
            unread: 0,
            messages: Vec::new(),
            users: Vec::new(),
        }
    }
}

/// A single message in a channel.
#[derive(Clone)]
pub struct IrcMessage {
    /// Sender's nick.
    pub nick: String,
    /// Message body text.
    pub content: String,
    /// Message kind (normal, action, system, join/part, etc.).
    pub kind: MessageKind,
    /// Timestamp (UTC) when the message was received.
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Classification of an IRC-style message for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    /// Regular PRIVMSG — rendered as `<nick> content`.
    Normal,
    /// `/me` action — rendered as `* nick content`.
    Action,
    /// Server/bot notice — rendered as `-nick- content`.
    Notice,
    /// Internal system message — rendered as `[system] content`.
    System,
    /// User joined the channel — rendered as `→ nick joined`.
    Join,
    /// User left the channel — rendered as `← nick left`.
    Part,
    /// Topic change — rendered as `[topic] nick set topic to: content`.
    Topic,
}

/// A user present in a channel.
#[derive(Clone)]
pub struct IrcUser {
    /// Display nick.
    pub nick: String,
    /// IRC mode prefix: `Some('@')` = channel op, `Some('+')` = voice, `None` = normal.
    pub mode: Option<char>,
    /// Whether the user is currently online (true) or idle/offline (false).
    pub is_online: bool,
    /// Optional role label (e.g. "coordinator", "worker", "observer").
    pub role: Option<String>,
}

impl IrcUser {
    pub fn new(nick: &str) -> Self {
        Self {
            nick: nick.to_string(),
            mode: None,
            is_online: true,
            role: None,
        }
    }

    /// Create an op (@) user.
    pub fn op(nick: &str) -> Self {
        Self {
            mode: Some('@'),
            ..Self::new(nick)
        }
    }

    /// Create a voiced (+) user.
    pub fn voiced(nick: &str) -> Self {
        Self {
            mode: Some('+'),
            ..Self::new(nick)
        }
    }

    /// Sorting key: ops first (0), then voiced (1), then normal (2).
    /// Within the same tier, sort alphabetically by nick (case-insensitive).
    pub fn sort_key(&self) -> (u8, String) {
        let tier = match self.mode {
            Some('@') => 0,
            Some('+') => 1,
            _ => 2,
        };
        (tier, self.nick.to_lowercase())
    }
}

// ── Login types ──────────────────────────────────────────────────────────────

/// The four credential fields on the Pantheon login screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoginField {
    #[default]
    Nick,
    ChannelKey,
    GatewayUrl,
    AgentName,
}

impl LoginField {
    /// Cycle to the next field (wraps around).
    pub fn next(self) -> Self {
        match self {
            Self::Nick       => Self::ChannelKey,
            Self::ChannelKey => Self::GatewayUrl,
            Self::GatewayUrl => Self::AgentName,
            Self::AgentName  => Self::Nick,
        }
    }

    /// Cycle to the previous field.
    pub fn prev(self) -> Self {
        match self {
            Self::Nick       => Self::AgentName,
            Self::ChannelKey => Self::Nick,
            Self::GatewayUrl => Self::ChannelKey,
            Self::AgentName  => Self::GatewayUrl,
        }
    }
}

/// Mutable form state for the login screen.
#[derive(Debug, Clone, Default)]
pub struct LoginForm {
    pub nick:        String,
    pub channel_key: String,
    pub gateway_url: String,
    pub agent_name:  String,
    /// Which field has keyboard focus.
    pub focused:     LoginField,
}

impl LoginForm {
    /// Return a mutable reference to the currently focused field's buffer.
    pub fn focused_buf_mut(&mut self) -> &mut String {
        match self.focused {
            LoginField::Nick       => &mut self.nick,
            LoginField::ChannelKey => &mut self.channel_key,
            LoginField::GatewayUrl => &mut self.gateway_url,
            LoginField::AgentName  => &mut self.agent_name,
        }
    }

    /// True if the minimum required fields are filled to attempt login.
    pub fn is_ready(&self) -> bool {
        !self.nick.trim().is_empty() && !self.channel_key.trim().is_empty()
    }
}

/// Current authentication state — drives what the login screen displays.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AuthState {
    /// Idle — user hasn't submitted yet.
    #[default]
    Idle,
    /// Login attempt in progress (connecting to gateway).
    Connecting,
    /// Authentication succeeded.
    Authenticated,
    /// Authentication failed — carries an error message to display.
    Failed(String),
}
