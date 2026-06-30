//! Zeus MCP Server Binary
//!
//! Standalone MCP server that exposes Zeus tools via JSON-RPC

use std::path::PathBuf;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use zeus_mcp::{McpConfig, McpServer};
use zeus_memory::Workspace;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "zeus_mcp=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Parse command line args
    let args: Vec<String> = std::env::args().collect();

    let mut config = McpConfig::default();
    let mut full_mode = false;
    let mut no_talos = false;
    let mut no_agents = false;

    // Simple arg parsing
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                if i + 1 < args.len() {
                    config.port = args[i + 1].parse().unwrap_or(9999);
                    i += 1;
                }
            }
            "--host" | "-h" => {
                if i + 1 < args.len() {
                    config.host = args[i + 1].clone();
                    i += 1;
                }
            }
            "--workspace" | "-w" => {
                if i + 1 < args.len() {
                    config.workspace = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--no-cors" => {
                config.cors = false;
            }
            "--full" => {
                full_mode = true;
            }
            "--no-talos" => {
                no_talos = true;
            }
            "--no-agents" => {
                no_agents = true;
            }
            "--help" => {
                print_help();
                return Ok(());
            }
            "--version" | "-V" => {
                println!("zeus-mcp {}", zeus_mcp::VERSION);
                return Ok(());
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                print_help();
                return Ok(());
            }
        }
        i += 1;
    }

    // Use default workspace if not specified
    if config.workspace.is_none() {
        let default_workspace = dirs::home_dir()
            .map(|h| h.join(".zeus").join("workspace"))
            .unwrap_or_else(|| PathBuf::from(".zeus/workspace"));

        if default_workspace.exists() {
            config.workspace = Some(default_workspace.to_string_lossy().to_string());
        }
    }

    // Initialize workspace if specified
    if let Some(ref workspace_path) = config.workspace {
        let workspace = Workspace::new(workspace_path);
        workspace.init().await?;
        tracing::info!("Using workspace: {}", workspace_path);
    }

    let server = if full_mode {
        let mut zeus_config = zeus_core::Config::load().unwrap_or_default();
        // Apply CLI overrides
        if no_talos || no_agents {
            let mcp_srv = zeus_config.mcp_server.get_or_insert_with(Default::default);
            if no_talos {
                mcp_srv.enable_talos = false;
            }
            if no_agents {
                mcp_srv.enable_agents = false;
            }
        }
        tracing::info!(
            "Starting in full mode (Talos: {}, Agents: {})",
            zeus_config
                .mcp_server
                .as_ref()
                .map(|c| c.enable_talos)
                .unwrap_or(true),
            zeus_config
                .mcp_server
                .as_ref()
                .map(|c| c.enable_agents)
                .unwrap_or(false),
        );
        // Surface D(b.i): build a real ChannelManager so the standalone MCP
        // binary's `message` tool can dispatch to platform adapters under
        // `--full`. The inbound receiver is dropped here — standalone MCP has
        // no Agent loop to drain it, so adapters buffer up to backpressure.
        let (channels, _inbound_rx) =
            zeus_agent::channel_builder::build_channel_manager_from_config(&zeus_config).await?;
        if channels.is_some() {
            tracing::info!(
                "Standalone MCP --full: ChannelManager constructed; \
                 inbound receiver dropped (no in-proc Agent to drain)"
            );
        }
        McpServer::with_full_config(config, &zeus_config, channels)
    } else {
        McpServer::with_config(config)
    };

    // Setup graceful shutdown
    let shutdown = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
        tracing::info!("Shutting down...");
    };

    server.run_with_shutdown(shutdown).await
}

fn print_help() {
    println!(
        r#"zeus-mcp - MCP Server for Zeus

USAGE:
    zeus-mcp [OPTIONS]

OPTIONS:
    -p, --port <PORT>         Port to listen on (default: 9999)
    -h, --host <HOST>         Host to bind to (default: 127.0.0.1)
    -w, --workspace <PATH>    Workspace directory path
        --no-cors             Disable CORS headers
        --full                Load ~/.zeus/config.toml, enable Talos + agents
        --no-talos            Disable Talos tools (with --full)
        --no-agents           Disable agent tools (with --full)
        --help                Print help information
    -V, --version             Print version information

EXAMPLES:
    zeus-mcp                           # Start with defaults (core tools only)
    zeus-mcp --full                    # Full mode with Talos + config
    zeus-mcp --full --no-agents        # Full mode without agent tools
    zeus-mcp --port 8080               # Custom port
    zeus-mcp -w ~/.zeus/workspace      # Custom workspace

ENDPOINTS:
    GET  /health              Health check
    POST /mcp                 MCP JSON-RPC endpoint
    POST /v1/mcp              MCP JSON-RPC endpoint (versioned)

MCP METHODS:
    initialize                Initialize connection
    tools/list                List available tools
    tools/call                Execute a tool
    resources/list            List workspace resources
    resources/read            Read a resource
    prompts/list              List available prompts
    prompts/get               Get a prompt template
"#
    );
}
