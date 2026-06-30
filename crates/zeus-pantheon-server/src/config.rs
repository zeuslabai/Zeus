//! PantheonServerConfig — loaded from config.toml [pantheon_server] section.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PantheonServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Shared secret used to generate/verify auth tokens.
    pub channel_key: String,
    /// User IDs that receive Admin tier on connect.
    #[serde(default)]
    pub admin_ids: Vec<String>,
    /// Channels every client is auto-joined to on auth.
    #[serde(default = "default_channels")]
    pub default_channels: Vec<String>,
    /// Max messages kept in per-channel history buffer.
    #[serde(default = "default_history_limit")]
    pub history_limit: usize,

    // ── Phase 5: TLS ──────────────────────────────────────────────────────────
    /// Enable TLS. Requires cert_path + key_path.
    #[serde(default)]
    pub tls: bool,
    /// Path to PEM certificate file (required if tls = true).
    #[serde(default)]
    pub cert_path: Option<String>,
    /// Path to PEM private key file (required if tls = true).
    #[serde(default)]
    pub key_path: Option<String>,

    // ── Phase 5: Rate limiting ────────────────────────────────────────────────
    /// Burst capacity in messages (default: 10).
    #[serde(default = "default_burst")]
    pub rate_burst: u32,
    /// Sustained message rate per second (default: 2.0).
    #[serde(default = "default_rate_per_sec")]
    pub rate_per_sec: f64,

    // ── Phase 5: Nick reservation ─────────────────────────────────────────────
    /// If true, nicks are reserved on first AUTH and rejected if already taken.
    #[serde(default = "default_true")]
    pub nick_reservation: bool,

    // ── Phase 5: MOTD ─────────────────────────────────────────────────────────
    /// Message of the day — sent to each client after successful AUTH.
    #[serde(default = "default_motd")]
    pub motd: String,
}

fn default_host() -> String { "0.0.0.0".into() }
fn default_port() -> u16 { 6669 } // Avoids conflict with standard IRC (6667)
fn default_history_limit() -> usize { 200 }
fn default_burst() -> u32 { 10 }
fn default_rate_per_sec() -> f64 { 2.0 }
fn default_true() -> bool { true }
fn default_motd() -> String {
    "Welcome to Pantheon — Zeus agent fleet communication hub.".into()
}
fn default_channels() -> Vec<String> {
    vec![
        "#general".into(),
        "#ops".into(),
        "#builds".into(),
        "#alerts".into(),
        "#agents".into(),
        "#missions".into(),
        "#random".into(),
    ]
}
