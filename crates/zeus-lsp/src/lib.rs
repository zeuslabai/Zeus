pub mod config;
pub mod session;
pub mod tools;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};

use config::LspServerConfig;
use session::LspSession;
use tools::{build_lsp_tools, LspTool};

/// Start all configured LSP servers and return their tools
pub async fn init_lsp_tools(
    servers: &HashMap<String, LspServerConfig>,
) -> Vec<LspTool> {
    let mut all_tools = Vec::new();

    for (name, config) in servers {
        info!("Spawning LSP server: {} ({})", name, config.command);
        match LspSession::spawn(name, config).await {
            Ok(session) => {
                let shared = Arc::new(Mutex::new(session));
                let tools = build_lsp_tools(name, shared);
                info!("LSP {}: registered {} tools", name, tools.len());
                all_tools.extend(tools);
            }
            Err(e) => {
                error!("Failed to spawn LSP server {}: {}", name, e);
            }
        }
    }

    all_tools
}


