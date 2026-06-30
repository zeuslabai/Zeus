//! Athena configuration

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Athena configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AthenaConfig {
    /// Obsidian vault path
    #[serde(default = "default_vault_path")]
    pub vault_path: PathBuf,

    /// Enable Apple Notes integration
    #[serde(default)]
    pub enable_apple_notes: bool,

    /// Apple Notes folder name
    #[serde(default = "default_notes_folder")]
    pub apple_notes_folder: String,

    /// Daily notes folder
    #[serde(default = "default_daily_folder")]
    pub daily_notes_folder: String,

    /// Sessions folder
    #[serde(default = "default_sessions_folder")]
    pub sessions_folder: String,
}

fn default_vault_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join("Documents").join("Zeus"))
        .unwrap_or_else(|| PathBuf::from("./zeus-docs"))
}

fn default_notes_folder() -> String {
    "Zeus".to_string()
}

fn default_daily_folder() -> String {
    "Daily".to_string()
}

fn default_sessions_folder() -> String {
    "Sessions".to_string()
}

impl AthenaConfig {
    /// Create a new AthenaConfig with specified vault path
    pub fn new(vault_path: PathBuf) -> Self {
        Self {
            vault_path,
            ..Default::default()
        }
    }
}

impl Default for AthenaConfig {
    fn default() -> Self {
        Self {
            vault_path: default_vault_path(),
            enable_apple_notes: false,
            apple_notes_folder: default_notes_folder(),
            daily_notes_folder: default_daily_folder(),
            sessions_folder: default_sessions_folder(),
        }
    }
}
