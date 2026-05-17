//! config.rs — Pantheon config loaded from zeus config.toml
//! Reads channel-key auth and server settings.

use serde::{Deserialize, Serialize};

/// Pantheon-specific config block from config.toml [pantheon]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PantheonConfig {
    /// IRC nick to use (defaults to agent name)
    pub nick: Option<String>,
    /// Channel key for auth (from config.toml)
    pub channel_key: Option<String>,
    /// Gateway URL override for Pantheon (falls back to main gateway)
    pub gateway_url: Option<String>,
    /// Auto-join channels on connect
    #[serde(default = "default_channels")]
    pub autojoin: Vec<String>,
}

fn default_channels() -> Vec<String> {
    vec!["#general".to_string()]
}

/// The 7 default IRC channels for the Zeus fleet
pub const DEFAULT_CHANNELS: &[&str] = &[
    "#general",
    "#ops-alerts",
    "#dev",
    "#missions",
    "#research",
    "#comms-log",
    "#debug",
];
