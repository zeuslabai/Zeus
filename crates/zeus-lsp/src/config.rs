use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Config for a single LSP server, sourced from config.toml [lsp.<name>]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
}
