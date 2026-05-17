//! Shared message-filtering types for all channel adapters.
//!
//! These filters were originally developed for the Discord adapter
//! (OpenClaw parity) and are being ported to all adapters in S43.

/// Bot message filter mode (OpenClaw parity: `allowBots` config).
///
/// Controls whether bot-authored messages are forwarded to the agent loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AllowBotsMode {
    /// Skip all bot messages
    Off,
    /// Allow bot messages that @mention this bot — default.
    /// Prevents fleet agents from triggering on each other's status reports and
    /// broadcasts. Self-echo is always blocked at Layer 1 regardless of this
    /// setting. Set `allow_bots = "on"` in config.toml to allow all bot messages.
    #[default]
    Mentions,
    /// Allow all bot messages.
    /// Self-echo is always blocked at Layer 1 regardless of this setting.
    On,
}

impl AllowBotsMode {
    /// Parse from a config string value. When `value` is `None`, returns the
    /// default (`Mentions` — only bot messages that @mention this bot pass through).
    pub fn from_config(value: Option<&str>) -> Self {
        match value {
            Some("off" | "false" | "none") => Self::Off,
            Some("mentions") => Self::Mentions,
            Some("on" | "true" | "all") => Self::On,
            None => Self::default(),
            _ => Self::default(),
        }
    }
}
