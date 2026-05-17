//! Zeus - Minimal autonomous AI assistant
//!
//! A Rust implementation with the power of the original Zeus
//! Autonomous AI assistant with cognitive engine, multi-channel chat, and native clients.

mod agent_executor;
mod benchmark;

mod daemon;
mod gateway;
mod gateway_bootstrap;
mod gateway_consumer;
mod gateway_lock;
mod gateway_relays;
mod gateway_web;
// inbox moved to zeus-core::inbox for cross-crate access
mod onboard;
mod presence_tracker;
mod reset;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};
use zeus_core::Config;

#[derive(Parser)]
#[command(name = "zeus")]
#[command(author, version, about = "Minimal autonomous AI assistant")]
struct Cli {
    /// Run in verbose mode
    #[arg(short, long)]
    verbose: bool,

    /// Configuration file path
    #[arg(short, long)]
    config: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the TUI interface (default)
    Tui,

    /// Run the HTTP API server
    Serve {
        /// Host to bind to
        #[arg(short = 'H', long, default_value = "127.0.0.1")]
        host: String,

        /// Port to listen on
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },

    /// Send a single message and exit
    Chat {
        /// The message to send
        message: String,

        /// Use streaming output
        #[arg(short, long)]
        stream: bool,
    },

    /// Run a single tool
    Tool {
        /// Tool name
        name: String,

        /// Tool arguments as JSON
        #[arg(default_value = "{}")]
        args: String,
    },

    /// Show or manage configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,

        /// Show all configuration including secrets
        #[arg(long)]
        show_secrets: bool,
    },

    /// Manage workspace memory
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },

    /// Manage sessions
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },

    /// Run MCP server on stdio (for Claude Code integration)
    Mcp {
        /// Disable Talos macOS automation tools
        #[arg(long)]
        no_talos: bool,
    },

    /// Run unified gateway daemon (API + channels + heartbeat)
    Gateway {
        /// Host to bind to (default: from config or 127.0.0.1)
        #[arg(short = 'H', long)]
        host: Option<String>,

        /// Port to listen on (default: from config or 8080)
        #[arg(short, long)]
        port: Option<u16>,

        /// Disable channel adapters
        #[arg(long)]
        no_channels: bool,

        /// Disable cron scheduler
        #[arg(long)]
        no_cron: bool,

        /// Disable heartbeat
        #[arg(long)]
        no_heartbeat: bool,

        /// Disable MCP server
        #[arg(long)]
        no_mcp: bool,

        /// MCP server port (default: from config or 3002)
        #[arg(long)]
        mcp_port: Option<u16>,

        /// Connect to a hub gateway as a fleet node (WebSocket URL)
        /// e.g. ws://192.168.1.112:8080/v1/ws/nodes
        #[arg(long)]
        connect_hub: Option<String>,

        /// Clear all sessions and context before starting (fresh slate)
        #[arg(long)]
        fresh: bool,
    },

    /// Run diagnostics and check configuration
    Doctor {
        /// Automatically fix common issues (missing dirs, orphaned files, permissions)
        #[arg(long)]
        repair: bool,
    },

    /// Run interactive setup wizard
    Onboard {
        /// Use classic numbered-menu wizard instead of conversational AI setup
        #[arg(long)]
        classic: bool,

        /// Check current service configuration (doctor mode)
        #[arg(long)]
        check: bool,

        /// Non-interactive mode for CI/automation (requires --provider and --model)
        #[arg(long)]
        non_interactive: bool,

        /// Provider for non-interactive mode (anthropic, openai, ollama, etc.)
        #[arg(long)]
        provider: Option<String>,

        /// Model for non-interactive mode (e.g., claude-sonnet-4-6)
        #[arg(long)]
        model: Option<String>,

        /// Force reconfigure even if already set up
        #[arg(long)]
        reconfigure: bool,
    },

    /// Manage daemon service (install/start/stop/status)
    Daemon {
        #[command(subcommand)]
        action: DaemonCommand,
    },

    /// Generate shell completions
    Completion {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },

    /// Import data from external sources (OpenClaw, ChatGPT, etc.)
    Import {
        /// Path to the directory or file to import
        path: String,

        /// Source format (auto-detected if not specified)
        #[arg(long, default_value = "auto", value_parser = ["auto", "openclaw", "chatgpt", "generic"])]
        from: String,
    },

    /// Manage the Zeus agent fleet
    Fleet {
        #[command(subcommand)]
        action: FleetAction,
    },

    /// Manage Pantheon multi-agent missions
    Pantheon {
        #[command(subcommand)]
        action: PantheonAction,
    },

    /// Run benchmark tasks and score agent performance
    Benchmark,

    /// Start the standalone Pantheon IRC server.
    ///
    /// Reads [pantheon_server] from config.toml. Agents connect via
    /// WebSocket and authenticate with the shared channel_key.
    /// Default port: 6669.
    #[command(name = "pantheon-server")]
    PantheonServer {
        /// Override the listen port (default: from config or 6669)
        #[arg(short, long)]
        port: Option<u16>,
        /// Override the listen host (default: from config or 0.0.0.0)
        #[arg(long)]
        host: Option<String>,
    },

    /// Reset fleet state (destructive — preserves credentials + identity files).
    ///
    /// Wipes a curated set of state DBs and scratch dirs under ~/.zeus/
    /// while preserving config.toml, wallet/, skills/, agents/, and the
    /// identity-class workspace .md files (SOUL/AGENTS/USER/IDENTITY/etc).
    Reset {
        /// Full 10-surface wipe (default if no scope flag).
        #[arg(long)]
        all: bool,
        /// Only purge ~/.zeus/memory.db (via mnemosyne-cleanup subprocess).
        #[arg(long)]
        memory_only: bool,
        /// Only wipe ~/.zeus/scheduler.db.
        #[arg(long)]
        scheduler_only: bool,
        /// Only wipe ~/.zeus/sessions/*.jsonl.
        #[arg(long)]
        sessions_only: bool,
        /// Print the plan without performing any IO.
        #[arg(long)]
        dry_run: bool,
        /// Skip the first 'yes' interactive confirm.
        #[arg(long)]
        yes: bool,
        /// Skip the second 'RESET' interactive confirm. Requires --yes.
        #[arg(long)]
        hard: bool,
    },
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Install daemon service file
    Install,
    /// Uninstall daemon service file
    Uninstall,
    /// Start the daemon
    Start,
    /// Stop the daemon
    Stop,
    /// Restart the daemon
    Restart {
        /// Clear all sessions and context before restarting (fresh slate)
        #[arg(long)]
        fresh: bool,
    },
    /// Show daemon status
    Status,
}

#[derive(Subcommand)]
enum FleetAction {
    /// Provision and add a new agent node to the fleet
    Add {
        /// Target host IP or hostname
        host: String,
        /// SSH username (default: mike)
        #[arg(long, default_value = "mike")]
        user: String,
        /// Target OS: darwin | linux | freebsd (default: linux)
        #[arg(long, default_value = "linux")]
        os: String,
        /// Agent role (default: worker)
        #[arg(long, default_value = "worker")]
        role: String,
        /// LLM model to configure (e.g. anthropic/claude-sonnet-4-6)
        #[arg(long)]
        model: String,
        /// SSH port (default: 22)
        #[arg(long, default_value = "22")]
        port: u16,
    },
    /// List all registered fleet agents
    List,
    /// Remove/deregister a fleet agent
    Remove {
        /// Agent ID to remove
        id: String,
    },
    /// Show provisioning status for a job
    Status {
        /// Provisioning job ID
        id: String,
    },
}

#[derive(Subcommand)]
enum PantheonAction {
    /// Join a mission room and watch live activity (IRC-style observer)
    Join {
        /// Mission ID to join
        mission_id: String,
        /// Poll interval in seconds (default: 3)
        #[arg(long, default_value = "3")]
        interval: u64,
    },
    /// List all active missions
    List,
    /// Show mission detail (team, tasks, progress)
    Status {
        /// Mission ID
        mission_id: String,
    },
}

#[derive(Subcommand)]
enum MemoryAction {
    /// Show current context
    Show,
    /// Add a memory fact
    Remember { fact: String },
    /// Add a daily note
    Note { content: String },
    /// List recent memories
    List,
}

#[derive(Subcommand)]
enum SessionAction {
    /// List all sessions
    List,
    /// Show a specific session
    Show { id: String },
    /// Export session to markdown
    Export { id: String, output: Option<String> },
    /// Fork a session — create a new session with the same message history
    Fork {
        /// Source session ID to fork from
        id: String,
        /// Optional label for the new session
        #[arg(long)]
        label: Option<String>,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Export configuration to a file
    Export {
        /// Output file path
        path: String,
    },
    /// Import configuration from a file
    Import {
        /// Input file path
        path: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // ── Credential loading: config.toml is the SOLE source of truth.
    // No .env file. No credentials.json generation.
    // credentials.json is only written by OAuthManager::login_with_token() during onboarding.
    // zeus-llm reads it via CredentialStore for OAuth auth resolution.
    if let Some(home) = dirs::home_dir() {
        let zeus_dir = home.join(".zeus");

        // Load config.toml [credentials] and inject into process env for API keys.
        // config.toml is the canonical source of truth for all secrets.
        if let Ok(config) = zeus_core::Config::load() {
            // S97: If OAuth is enabled, skip ANTHROPIC_API_KEY from credentials
            // to prevent auth method confusion (OAuth token in API key field).
            let use_oauth = config.auth.use_oauth;
            for (key, value) in &config.credentials {
                if !value.is_empty() {
                    if use_oauth && key == "ANTHROPIC_API_KEY" {
                        println!("  Skipping ANTHROPIC_API_KEY env var (OAuth enabled, config.toml is SSoT)");
                        continue;
                    }
                    // SAFETY: single-threaded at startup, before tokio runtime
                    unsafe { std::env::set_var(key, value) };
                }
            }
        }

        // S78: Read config.toml [oauth] token and populate CredentialStore for zeus-llm.
        // This is READ-ONLY — main.rs never writes to config.toml.
        // config.toml is the SSoT. CredentialStore is a runtime derivative.
        if let Ok(config_content) = std::fs::read_to_string(zeus_dir.join("config.toml")) {
            if let Some(oauth_start) = config_content.find("[oauth]") {
                let oauth_section = &config_content[oauth_start..];
                for line in oauth_section.lines().skip(1) {
                    if line.starts_with('[') { break; }
                    if let Some(token_val) = line.strip_prefix("token").and_then(|s| s.trim_start().strip_prefix('=')) {
                        let token = token_val.trim().trim_matches('"');
                        if !token.is_empty() {
                            // Populate in-memory CredentialStore — from_config() handles provider routing.
                            if let Err(e) = zeus_llm::OAuthManager::login_with_token(token) {
                                warn!("Failed to populate CredentialStore from config.toml [oauth]: {}", e);
                            }
                        }
                        break;
                    }
                }
            }
        }
    }

    let cli = Cli::parse();

    // Ensure ~/.zeus/logs/ exists before any code path that might write to it.
    // launchd plists (see src/daemon.rs + crates/zeus-setup/src/ops/service.rs) redirect
    // StandardOutPath/StandardErrorPath to ~/.zeus/logs/gateway.{out,err}.log; if the
    // directory is missing, launchd silently drops to /dev/null and we lose runtime logs.
    // Idempotent, .ok() to never fail boot on this.
    if let Some(home) = dirs::home_dir() {
        std::fs::create_dir_all(home.join(".zeus").join("logs")).ok();
    }

    // Initialize logging — redirect to file in TUI mode to avoid display corruption
    // In MCP stdio mode, redirect logs to file to keep stdout clean for JSON-RPC
    let is_tui = matches!(cli.command, None | Some(Commands::Tui));
    let is_stdio_mcp = matches!(cli.command, Some(Commands::Mcp { .. }));
    let log_level = if cli.verbose { "debug" } else { "info" };
    let filter =
        EnvFilter::from_default_env().add_directive(format!("zeus={}", log_level).parse()?);

    if is_tui || is_stdio_mcp {
        // TUI: logs corrupt display. MCP stdio: logs corrupt JSON-RPC stream.
        let log_dir = dirs::home_dir().unwrap_or_default().join(".zeus");
        std::fs::create_dir_all(&log_dir).ok();
        let log_name = if is_stdio_mcp {
            "mcp-stdio.log"
        } else {
            "zeus.log"
        };
        let log_file = std::fs::File::create(log_dir.join(log_name))?;
        tracing_subscriber::registry()
            .with(
                fmt::layer()
                    .with_writer(std::sync::Mutex::new(log_file))
                    .with_ansi(false),
            )
            .with(filter)
            .init();
    } else {
        tracing_subscriber::registry()
            .with(fmt::layer())
            .with(filter)
            .init();
    }

    // Install panic hook for non-TUI commands (TUI installs its own)
    if std::env::var("ZEUS_NO_PANIC_HOOK").is_err() {
        let log_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".zeus/logs");
        let _ = std::fs::create_dir_all(&log_dir);
        std::panic::set_hook(Box::new(move |info| {
            let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let log_path = log_dir.join(format!("gateway-panic-{}.log", timestamp));
            let backtrace = std::backtrace::Backtrace::capture();
            let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = info.payload().downcast_ref::<String>() {
                s.clone()
            } else {
                "Unknown panic payload".to_string()
            };
            let location = info.location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_else(|| "unknown location".to_string());
            let content = format!(
                "Zeus gateway panicked at {}\nPayload: {}\nBacktrace:\n{}\n",
                location, payload, backtrace
            );
            let _ = std::fs::write(&log_path, content);
            eprintln!("Zeus gateway panicked. Log written to: {}", log_path.display());
        }));
    }

    info!("Zeus starting...");

    // Determine if this is an interactive command that needs config
    let needs_wizard = matches!(cli.command, None | Some(Commands::Tui));

    // Load configuration (skip wizard for quick CLI commands)
    let config = load_config(cli.config.as_deref(), needs_wizard).await?;

    // Validate configuration (skip for daemon/utility commands to reduce noise)
    let show_warnings = matches!(
        cli.command,
        None | Some(Commands::Tui)
            | Some(Commands::Serve { .. })
            | Some(Commands::Chat { .. })
            | Some(Commands::Gateway { .. })
    );
    if show_warnings {
        let warnings = config.validate();
        if !warnings.is_empty() {
            eprintln!();
            for w in &warnings {
                eprintln!("  ⚠ Config: {}", w);
            }
            eprintln!();
            eprintln!("  Run 'zeus doctor' to diagnose and fix configuration issues.");
            eprintln!();
        }
    }

    // Handle commands
    match cli.command {
        None | Some(Commands::Tui) => run_tui(config).await,
        Some(Commands::Serve { host, port }) => run_server(config, &host, port).await,
        Some(Commands::Chat { message, stream }) => run_chat(config, &message, stream).await,
        Some(Commands::Tool { name, args }) => run_tool(config, &name, &args).await,
        Some(Commands::Config {
            action,
            show_secrets,
        }) => match action {
            Some(ConfigAction::Export { path }) => {
                config.export_to_file(&path)?;
                println!("Config exported to {}", path);
                Ok(())
            }
            Some(ConfigAction::Import { path }) => {
                let imported = Config::import_from_file(&path)?;
                imported.save()?;
                println!(
                    "Config imported from {} and saved to ~/.zeus/config.toml",
                    path
                );
                Ok(())
            }
            None => show_config(&config, show_secrets),
        },
        Some(Commands::Memory { action }) => run_memory(config, action).await,
        Some(Commands::Session { action }) => run_session(config, action).await,
        Some(Commands::Mcp { no_talos }) => {
            // Override mcp_server.enable_talos if --no-talos flag
            let mut config = config;
            if no_talos {
                let mcp_srv = config.mcp_server.get_or_insert_with(Default::default);
                mcp_srv.enable_talos = false;
            }

            // MCP is a pure STDIO tool server — no gateway spawn.
            // Gateway runs independently as an OS service (launchd/rc.d/systemd).
            zeus_mcp::McpStdio::run(&config).await
        }
        Some(Commands::Gateway {
            host,
            port,
            no_channels,
            no_cron,
            no_heartbeat,
            no_mcp,
            mcp_port,
            connect_hub,
            fresh,
        }) => {
            if fresh {
                let sessions_dir = &config.sessions;
                match std::fs::read_dir(sessions_dir) {
                    Ok(entries) => {
                        let mut cleared = 0usize;
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                                if std::fs::remove_file(&path).is_ok() {
                                    cleared += 1;
                                }
                            }
                        }
                        tracing::info!("--fresh: cleared {} session file(s) from {:?}", cleared, sessions_dir);
                    }
                    Err(e) => {
                        tracing::warn!("--fresh: could not read sessions dir {:?}: {}", sessions_dir, e);
                    }
                }
                // Also clear cooking checkpoints
                if let Ok(home) = zeus_core::Config::zeus_home() {
                    let checkpoint_db = home.join("cooking_checkpoints.db");
                    if checkpoint_db.exists() {
                        let _ = std::fs::remove_file(&checkpoint_db);
                        tracing::info!("--fresh: cleared cooking checkpoints");
                    }
                }
            }
            let existing_gw = config.gateway.clone().unwrap_or_default();
            let gw_config = zeus_core::GatewayConfig {
                host: host.unwrap_or(existing_gw.host.clone()),
                port: port.unwrap_or(existing_gw.port),
                public_url: config
                    .gateway
                    .as_ref()
                    .map(|g| g.public_url.clone())
                    .unwrap_or_default(),
                enable_channels: !no_channels,
                enable_cron: !no_cron,
                enable_heartbeat: !no_heartbeat,
                enable_api: true,
                enable_mcp: !no_mcp,
                mcp_port: mcp_port.unwrap_or(existing_gw.mcp_port),
                web_dist: config.gateway.as_ref().and_then(|g| g.web_dist.clone()),
                web_port: config.gateway.as_ref().map(|g| g.web_port).unwrap_or(8081),
                timeout_secs: existing_gw.timeout_secs,
                reconnect_delay_secs: existing_gw.reconnect_delay_secs,
                max_ws_message_bytes: existing_gw.max_ws_message_bytes,
                max_webhook_payload_bytes: existing_gw.max_webhook_payload_bytes,
                max_webhook_message_bytes: existing_gw.max_webhook_message_bytes,
                max_inbound_message_len: existing_gw.max_inbound_message_len,
                rate_limit: existing_gw.rate_limit.clone(),
                enable_agent_processing: existing_gw.enable_agent_processing,
                mentions_only: existing_gw.mentions_only,
                discord_role_ids: existing_gw.discord_role_ids.clone(),
                peer_agent_names: existing_gw.peer_agent_names.clone(),
                dm_scope: existing_gw.dm_scope.clone(),
                response_prefix: existing_gw.response_prefix.clone(),
                channel_prompt: existing_gw.channel_prompt.clone(),
                fleet_channel_id: existing_gw.fleet_channel_id.clone(),
                api_token: existing_gw.api_token.clone(),
                cors_origins: existing_gw.cors_origins.clone(),
            };

            // If --connect-hub is provided, spawn the node client in background
            if let Some(hub_url) = connect_hub {
                let agent_name = config
                    .network
                    .as_ref()
                    .and_then(|n| n.agent_name.clone())
                    .unwrap_or_else(|| "@unknown".to_string());
                let host_ip = std::env::var("ZEUS_HOST_IP").unwrap_or_else(|_| {
                    hostname::get()
                        .ok()
                        .and_then(|h| h.to_str().map(String::from))
                        .unwrap_or_else(|| "127.0.0.1".to_string())
                });
                let tmux_target = config.network.as_ref().and_then(|n| n.tmux_target.clone());

                let client_config = zeus_api::node_client::NodeClientConfig {
                    hub_url: hub_url.clone(),
                    node_id: agent_name.clone(),
                    host: host_ip,
                    capabilities: vec![],
                    reconnect_interval: std::time::Duration::from_secs(5),
                    tmux_target,
                };
                tracing::info!("Connecting to hub at {} as {}", hub_url, agent_name);
                tokio::spawn(zeus_api::node_client::run_node_client(client_config));
            }

            gateway::run_gateway(config, gw_config).await
        }
        Some(Commands::Doctor { repair }) => run_doctor(config, repair).await,
        Some(Commands::Onboard { classic, check, non_interactive, provider, model, reconfigure: _ }) => {
            if check {
                onboard::run_setup_check()?;
            } else if classic {
                onboard::run_onboard()?;
            } else if non_interactive {
                // CI/automation mode — configure directly without TUI
                let mut config = config;
                let provider_str = provider.unwrap_or_else(|| "anthropic".to_string());
                let model_str = model.unwrap_or_else(|| {
                    eprintln!("Error: --model is required for non-interactive mode.");
                    eprintln!("  Example: zeus onboard --non-interactive --provider anthropic --model claude-sonnet-4-6");
                    std::process::exit(1);
                });
                let full_model = if model_str.contains('/') {
                    model_str
                } else {
                    format!("{}/{}", provider_str, model_str)
                };
                println!("Non-interactive setup: {} → {}", provider_str, full_model);
                config.model = full_model;
                config.onboarding_complete = true;
                config.save()?;
                println!("✓ Configuration saved. Run 'zeus gateway' to start.");
            } else {
                // Interactive TUI v2 onboarding — patch file directly so
                // needs_onboarding() sees onboarding_complete = false.
                if let Ok(path) = zeus_core::Config::config_path() {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let patched = content.replace("onboarding_complete = true", "onboarding_complete = false");
                        let _ = std::fs::write(&path, patched);
                    }
                }
                let mut config = config;
                config.onboarding_complete = false;
                run_tui(config).await?;
            }
            Ok(())
        }
        Some(Commands::Daemon { action }) => {
            let da = match action {
                DaemonCommand::Install => daemon::DaemonAction::Install,
                DaemonCommand::Uninstall => daemon::DaemonAction::Uninstall,
                DaemonCommand::Start => daemon::DaemonAction::Start,
                DaemonCommand::Stop => daemon::DaemonAction::Stop,
                DaemonCommand::Restart { fresh } => daemon::DaemonAction::Restart { fresh },
                DaemonCommand::Status => daemon::DaemonAction::Status,
            };
            daemon::run_daemon(da).await
        }
        Some(Commands::Completion { shell }) => {
            use clap::CommandFactory;
            clap_complete::generate(shell, &mut Cli::command(), "zeus", &mut std::io::stdout());
            Ok(())
        }
        Some(Commands::Import { path, from }) => run_import(config, &path, &from).await,
        Some(Commands::Fleet { action }) => run_fleet(config, action).await,
        Some(Commands::Pantheon { action }) => run_pantheon(config, action).await,
        Some(Commands::Benchmark) => benchmark::run_benchmark(config).await,
        Some(Commands::PantheonServer { port, host }) => {
            run_pantheon_server(config, host, port).await
        }
        Some(Commands::Reset {
            all,
            memory_only,
            scheduler_only,
            sessions_only,
            dry_run,
            yes,
            hard,
        }) => {
            let _ = config; // reset reads $HOME/$ZEUS_HOME directly
            reset::run(reset::ResetArgs {
                all,
                memory_only,
                scheduler_only,
                sessions_only,
                dry_run,
                yes,
                hard,
            })
        }
    }
}

async fn load_config(path: Option<&str>, allow_wizard: bool) -> Result<Config> {
    // First-run detection: if no config exists and this is an interactive command,
    // launch the SetupWizard (the full S55 TUI onboarding with all 11 providers,
    // OAuth, model selection, channels, workspace files, launch options).
    if allow_wizard && path.is_none() {
        let config_path = dirs::home_dir()
            .expect("cannot find home dir")
            .join(".zeus")
            .join("config.toml");
        if !config_path.exists() {
            // No config — the TUI v2 onboarding module handles first-run setup.
            // Create a minimal config so the binary can start, then onboarding takes over.
            info!("No config.toml found — TUI v2 onboarding will handle setup");
            let zeus_dir = config_path.parent().unwrap();
            let _ = std::fs::create_dir_all(zeus_dir);
            let _ = std::fs::create_dir_all(zeus_dir.join("workspace"));
            let _ = std::fs::create_dir_all(zeus_dir.join("sessions"));
            // Write minimal bootstrap config (onboarding_complete = false triggers the wizard)
            let bootstrap = format!(
                "model = \"\"\nworkspace = \"{ws}\"\nsessions = \"{ss}\"\nonboarding_complete = false\n",
                ws = zeus_dir.join("workspace").display(),
                ss = zeus_dir.join("sessions").display(),
            );
            let _ = std::fs::write(&config_path, &bootstrap);
        }
    }

    // Load from path or default location
    let config = if let Some(p) = path {
        Config::load_from(p)?
    } else {
        match Config::load() {
            Ok(c) => c,
            Err(e) => {
                // Check if config.toml exists — if it does, the parse FAILED.
                // Never silently fall back to defaults over a real config.
                let config_path = dirs::home_dir()
                    .unwrap_or_default()
                    .join(".zeus/config.toml");
                if config_path.exists() {
                    // Config exists but failed to parse — likely a schema change.
                    // Back up the broken config and error out loudly.
                    let backup = config_path.with_extension("toml.parse-error-backup");
                    let _ = std::fs::copy(&config_path, &backup);
                    eprintln!("ERROR: config.toml exists but failed to parse: {}", e);
                    eprintln!("Backed up to: {}", backup.display());
                    eprintln!("This usually means the binary has new config fields.");
                    eprintln!("Fix: run 'zeus onboard' to regenerate, or manually edit config.toml");
                    std::process::exit(1);
                } else {
                    // No config at all — use defaults (first run)
                    warn!("No config found, using defaults: {}", e);
                    Config { loaded_from_default: true, ..Config::default() }
                }
            }
        }
    };

    Ok(config)
}

/// Check if the configured provider has valid credentials
async fn run_tui(config: Config) -> Result<()> {
    // Redirect stderr to log file — prevents stray output from corrupting TUI display.
    // Tracing is already redirected, but eprintln!, dependency output, and subprocess
    // stderr can still leak through and corrupt the ratatui terminal.
    let log_dir = dirs::home_dir().unwrap_or_default().join(".zeus");
    if let Ok(log_file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("zeus.log"))
    {
        use std::os::unix::io::IntoRawFd;
        let fd = log_file.into_raw_fd();
        unsafe {
            libc::dup2(fd, 2); // redirect stderr (fd 2) to log file
            libc::close(fd);
        }
    }

    zeus_tui::run(config).await?;
    info!("Zeus shutdown complete");
    Ok(())
}

async fn run_server(config: Config, host: &str, port: u16) -> Result<()> {
    use std::sync::Arc;
    use tokio::sync::RwLock;

    info!("Starting API server on {}:{}", host, port);

    // Start session pruning background task if enabled
    let _pruning_handle = if config.pruning.as_ref().map(|p| p.enabled).unwrap_or(false) {
        let pruning_config = config.pruning.clone().unwrap();
        let sessions_dir = config.sessions.clone();
        let handle = zeus_session::start_pruning_task(pruning_config.clone(), sessions_dir);
        info!(
            "Session pruning started (interval={}s, max_age={}d, max_sessions={}, max_size={}MB{})",
            pruning_config.check_interval_secs,
            pruning_config.max_age_days,
            pruning_config.max_sessions,
            pruning_config.max_total_size_mb,
            if pruning_config.dry_run {
                ", dry_run"
            } else {
                ""
            }
        );
        Some(handle)
    } else {
        None
    };

    let state = Arc::new(RwLock::new(
        zeus_api::AppState::new(config)
            .map_err(|e| anyhow::anyhow!("AppState init failed: {}", e))?,
    ));
    zeus_api::AppState::boot(&state).await;

    // Periodic mission timeout check (every 60s, configurable via [gateway].timeout_secs)
    let _mission_timeout_handle = {
        let s = state.read().await;
        let timeout = s
            .config
            .gateway
            .as_ref()
            .map(|g| g.timeout_secs)
            .unwrap_or(1800);
        let store = s.pantheon.clone();
        drop(s);
        zeus_api::PantheonStore::start_timeout_check_task(
            store,
            std::time::Duration::from_secs(timeout),
        )
    };

    let router = zeus_api::create_router(state, true); // Enable CORS

    let addr = format!("{}:{}", host, port);
    // Retry bind up to 10 times (2s apart) so a restarting gateway doesn't
    // fail immediately if the previous process hasn't released the port yet.
    let listener = {
        let mut last_err: Option<std::io::Error> = None;
        let mut bound = None;
        for attempt in 1..=10u32 {
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(l) => { bound = Some(l); break; }
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                    tracing::warn!("Port {} in use, retry {}/10 in 2s…", port, attempt);
                    last_err = Some(e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
        bound.ok_or_else(|| anyhow::anyhow!("Port {} still in use after 10 retries: {}", port, last_err.unwrap()))?
    };

    println!("Zeus API server running at http://{}", addr);
    println!("Endpoints:");
    println!("  POST /v1/chat        - Send message to agent");
    println!("  GET  /v1/sessions    - List sessions");
    println!("  POST /v1/sessions    - Create new session");
    println!("  GET  /v1/tools       - List available tools");
    println!("  POST /v1/tools/:name - Execute a tool");
    println!("  GET  /v1/memory      - Get workspace context");
    println!("  POST /v1/memory/remember - Add to memory");
    println!();
    println!("Press Ctrl+C to stop");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // Abort pruning task on shutdown
    if let Some(handle) = _pruning_handle {
        handle.abort();
    }

    info!("API server shut down gracefully");
    Ok(())
}

/// Wait for a shutdown signal (Ctrl+C or SIGTERM)
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C, shutting down...");
        }
        _ = terminate => {
            info!("Received SIGTERM, shutting down...");
        }
    }
}

async fn run_chat(config: Config, message: &str, stream: bool) -> Result<()> {
    use zeus_core::Message;
    use zeus_llm::LlmClient;
    use zeus_memory::Workspace;

    let llm = LlmClient::from_config(&config)?;
    let workspace = Workspace::from_config(&config);
    workspace.init().await?;

    let system = workspace.get_context().await?;
    let messages = vec![Message::user(message)];

    if stream {
        let (mut rx, handle) = llm.stream(&messages, &[], Some(&system)).await?;
        while let Some(chunk) = rx.recv().await {
            print!("{}", chunk);
            use std::io::Write;
            std::io::stdout().flush()?;
        }
        println!();
        let _ = handle.await;
    } else {
        let response = llm.complete(&messages, &[], Some(&system)).await?;
        println!("{}", response.content);
    }

    Ok(())
}

async fn run_tool(config: Config, name: &str, args: &str) -> Result<()> {
    use serde_json::Value;
    use zeus_agent::ToolRegistry;
    use zeus_talos::TalosRegistry;

    // Build tool registry with Talos tools if configured
    let registry = if config.talos.is_some() {
        let talos = TalosRegistry::with_defaults();
        ToolRegistry::with_talos(talos)
    } else {
        ToolRegistry::with_defaults()
    };

    let args: Value = serde_json::from_str(args)?;

    println!("Executing tool: {}", name);
    let result = registry.execute(name, args).await?;
    println!("{}", result);

    Ok(())
}

fn show_config(config: &Config, show_secrets: bool) -> Result<()> {
    println!("Zeus Configuration");
    println!("==================");
    println!();
    println!("Model: {}", config.model);
    println!("Workspace: {}", config.workspace.display());
    println!("Sessions: {}", config.sessions.display());
    println!("Max iterations: {}", config.max_iterations);

    if show_secrets {
        println!();
        println!("API Keys (masked):");
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            println!("  ANTHROPIC_API_KEY: {}...", &key[..12.min(key.len())]);
        }
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            println!("  OPENAI_API_KEY: {}...", &key[..12.min(key.len())]);
        }
        if let Ok(key) = std::env::var("OPENROUTER_API_KEY") {
            println!("  OPENROUTER_API_KEY: {}...", &key[..12.min(key.len())]);
        }
    }

    Ok(())
}

async fn run_memory(config: Config, action: MemoryAction) -> Result<()> {
    use zeus_memory::Workspace;

    let workspace = Workspace::from_config(&config);
    workspace.init().await?;

    match action {
        MemoryAction::Show => {
            let context = workspace.get_context().await?;
            println!("{}", context);
        }
        MemoryAction::Remember { fact } => {
            workspace.remember(&fact).await?;
            println!("Remembered: {}", fact);
        }
        MemoryAction::Note { content } => {
            workspace.note(&content).await?;
            println!("Added daily note");
        }
        MemoryAction::List => {
            let memory = workspace.get_memory().await?;
            println!("{}", memory);
        }
    }

    Ok(())
}

async fn run_session(config: Config, action: SessionAction) -> Result<()> {
    use zeus_session::Session;

    match action {
        SessionAction::List => {
            let sessions = Session::list(&config.sessions).await?;
            if sessions.is_empty() {
                println!("No sessions found");
            } else {
                println!("Sessions:");
                for (id, created) in sessions {
                    println!("  {} - {}", id, created.format("%Y-%m-%d %H:%M"));
                }
            }
        }
        SessionAction::Show { id } => {
            let session = Session::load(&config.sessions, &id).await?;
            println!("Session: {}", session.id);
            println!("Messages: {}", session.messages.len());
            println!();
            for msg in &session.messages {
                println!("[{:?}] {}", msg.role, msg.content);
                println!();
            }
        }
        SessionAction::Export { id, output } => {
            let session = Session::load(&config.sessions, &id).await?;
            let markdown = session.export_markdown().await;

            if let Some(path) = output {
                std::fs::write(&path, &markdown)?;
                println!("Exported to: {}", path);
            } else {
                println!("{}", markdown);
            }
        }
        SessionAction::Fork { id, label } => {
            let source = Session::load(&config.sessions, &id).await?;
            let msg_count = source.messages.len();
            let mut forked = Session::new(&config.sessions);
            forked.label = label.clone();
            forked.init().await?;
            for msg in source.messages {
                forked.add(msg).await?;
            }
            println!("Forked session {} → {}", id, forked.id);
            if let Some(l) = label {
                println!("Label: {}", l);
            }
            println!("Messages copied: {}", msg_count);
        }
    }

    Ok(())
}

async fn run_doctor(config: Config, repair: bool) -> Result<()> {
    use zeus_llm::LlmClient;

    println!("Zeus Doctor{}", if repair { " (repair mode)" } else { "" });
    println!("===========\n");

    // 1. Config file
    let config_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("config.toml");
    if config_path.exists() {
        println!("[OK] Config file: {}", config_path.display());
    } else {
        println!(
            "[WARN] Config file not found: {} (using defaults)",
            config_path.display()
        );
    }

    // 2. Model + Provider
    let (provider, model) = config.parse_model();
    println!("[INFO] Provider: {:?}, Model: {}", provider, model);

    // 3. API Key
    let llm = LlmClient::from_config(&config)?;
    if llm.has_credentials() {
        println!("[OK] Credentials configured for {:?}", provider);
    } else {
        println!("[FAIL] No credentials for {:?}", provider);
        if let zeus_llm::CredentialStatus::Missing { suggestions, .. } = llm.credential_status() {
            for s in suggestions {
                println!("  Suggestion: {}", s);
            }
        }
    }

    // 4. Workspace directory
    if config.workspace.exists() {
        println!("[OK] Workspace: {}", config.workspace.display());
    } else {
        println!(
            "[WARN] Workspace not found: {} (will be created on first run)",
            config.workspace.display()
        );
    }

    // 5. Sessions directory
    if config.sessions.exists() {
        println!("[OK] Sessions: {}", config.sessions.display());
    } else {
        println!(
            "[WARN] Sessions not found: {} (will be created on first run)",
            config.sessions.display()
        );
    }

    // 6. Mnemosyne DB
    if let Some(ref mnemosyne) = config.mnemosyne {
        if mnemosyne.db_path.exists() {
            println!("[OK] Mnemosyne DB: {}", mnemosyne.db_path.display());
        } else {
            println!(
                "[WARN] Mnemosyne DB not found: {} (will be created)",
                mnemosyne.db_path.display()
            );
        }
    } else {
        println!("[INFO] Mnemosyne: not configured");
    }

    // 7. Channel configs (config.toml + env var auto-detect)
    let effective_channels = match &config.channels {
        Some(cc) => {
            let mut merged = cc.clone();
            merged.merge_env();
            Some(merged)
        }
        None => zeus_core::ChannelsConfig::from_env(),
    };
    if let Some(ref channels) = effective_channels {
        if channels.telegram.is_some() {
            println!("[INFO] Telegram: configured");
        }
        if channels.discord.is_some() {
            let src = if config
                .channels
                .as_ref()
                .and_then(|c| c.discord.as_ref())
                .is_some()
            {
                "config"
            } else {
                "env"
            };
            println!("[INFO] Discord: configured ({})", src);
        }
        if channels.slack.is_some() {
            let src = if config
                .channels
                .as_ref()
                .and_then(|c| c.slack.as_ref())
                .is_some()
            {
                "config"
            } else {
                "env"
            };
            println!("[INFO] Slack: configured ({})", src);
        }
        if channels.email.is_some() {
            println!("[INFO] Email: configured");
        }
        if channels.whatsapp.is_some() {
            println!("[INFO] WhatsApp: configured");
        }
        if channels.signal.is_some() {
            let src = if config
                .channels
                .as_ref()
                .and_then(|c| c.signal.as_ref())
                .is_some()
            {
                "config"
            } else {
                "env"
            };
            println!("[INFO] Signal: configured ({})", src);
        }
        if channels.matrix.is_some() {
            let src = if config
                .channels
                .as_ref()
                .and_then(|c| c.matrix.as_ref())
                .is_some()
            {
                "config"
            } else {
                "env"
            };
            println!("[INFO] Matrix: configured ({})", src);
        }
    } else {
        println!("[INFO] Channels: none configured");
    }

    // 8. Talos (macOS automation)
    if config.talos.is_some() {
        #[cfg(target_os = "macos")]
        {
            if std::process::Command::new("osascript")
                .arg("-e")
                .arg("return \"ok\"")
                .output()
                .is_ok()
            {
                println!("[OK] Talos: osascript available");
            } else {
                println!("[WARN] Talos: osascript not available");
            }
        }
        #[cfg(not(target_os = "macos"))]
        println!("[WARN] Talos: macOS only, some tools unavailable on this platform");
    } else {
        println!("[INFO] Talos: not configured");
    }

    // 9. Subsystem summary
    println!("\nSubsystems:");
    println!(
        "  Aegis (security):     {}",
        if config.aegis.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!(
        "  Athena (docs):        {}",
        if config.athena.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!(
        "  Hermes (notify):      {}",
        if config.hermes.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!(
        "  Nous (cognitive):     {}",
        if config.nous.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!(
        "  Prometheus (brain):   {}",
        if config.prometheus.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!(
        "  Search:               {}",
        if config.search.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!(
        "  Session compaction:   {}",
        if config.session_compaction.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!(
        "  Thinking level:       {}",
        config.thinking_level.as_deref().unwrap_or("disabled")
    );

    // 10. Environment detection
    let detected = Config::detect_environment();
    if detected.is_empty() {
        println!("\n[WARN] No API keys detected in environment");
    } else {
        println!("\nDetected API keys:");
        for (provider, env_var) in &detected {
            println!("  [OK] {:?} ({})", provider, env_var);
        }
        if let Some((suggested, model)) = Config::suggest_provider() {
            println!("  Suggested: {:?} -> {}", suggested, model);
        }
    }

    // 11. Channel config validation
    let channel_warnings = config.validate_channels();
    if !channel_warnings.is_empty() {
        println!("\nChannel warnings:");
        for w in &channel_warnings {
            println!("  [WARN] {}", w);
        }
    }

    // 12. File descriptor limit (ulimit -n)
    #[cfg(unix)]
    {
        let mut rlim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: getrlimit is safe with a valid pointer to an rlimit struct on the stack.
        let ret = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) };
        if ret == 0 {
            #[allow(clippy::unnecessary_cast)]
            let soft = rlim.rlim_cur as u64;
            const MIN_FDS: u64 = 4096;
            if soft >= MIN_FDS {
                println!("[OK] File descriptor limit (ulimit -n): {}", soft);
            } else {
                println!(
                    "[WARN] File descriptor limit too low: {} (recommended: >= {})",
                    soft, MIN_FDS
                );
                println!(
                    "  FreeBSD fix: sudo sysctl kern.maxfilesperproc=65536  +  ulimit -n 10240"
                );
                println!("  Linux fix:   ulimit -n 10240  (or set in /etc/security/limits.conf)");
                println!("  macOS fix:   ulimit -n 10240");
            }
        } else {
            println!("[INFO] File descriptor limit: unable to query");
        }
    }
    #[cfg(not(unix))]
    println!("[INFO] File descriptor limit: not checked on this platform");

    // 13. AgentShield — static config security scan
    {
        let mut shield_warnings: Vec<String> = Vec::new();

        // 13a. Config file permissions (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if config_path.exists()
                && let Ok(meta) = std::fs::metadata(&config_path) {
                    let mode = meta.permissions().mode();
                    // Check if world-readable (o+r = 0o004) or group-readable (g+r = 0o040)
                    if mode & 0o044 != 0 {
                        shield_warnings.push(format!(
                            "config.toml is readable by others (mode {:o}) — may expose API keys. Fix: chmod 600 {}",
                            mode & 0o777,
                            config_path.display()
                        ));
                    }
                }
        }

        // 13b. Aegis security subsystem disabled
        if config.aegis.is_none() {
            shield_warnings.push(
                "Aegis security subsystem not enabled — no sandbox, no command filtering, no audit log. Add [aegis] to config.toml".to_string()
            );
        } else if let Some(ref aegis) = config.aegis {
            // 13c. Overly permissive security settings
            if aegis.sandbox_level == "none" {
                shield_warnings.push(
                    "Aegis sandbox_level = \"none\" — shell commands run unrestricted. Consider \"standard\" or \"strict\"".to_string()
                );
            }
            if aegis.permissions.contains(&"*".to_string()) {
                shield_warnings.push(
                    "Aegis permissions = [\"*\"] — all operations allowed without filtering".to_string()
                );
            }
            if aegis.network_allowlist.contains(&"*".to_string()) {
                shield_warnings.push(
                    "Aegis network_allowlist = [\"*\"] — all URLs allowed for web_fetch. Consider restricting to known domains".to_string()
                );
            }
        }

        // 13d. Scan workspace for accidentally committed secrets
        let workspace_dir = &config.workspace;
        if workspace_dir.exists() {
            let secret_patterns = [
                ("sk-", "OpenAI/Anthropic API key"),
                ("xoxb-", "Slack bot token"),
                ("xapp-", "Slack app token"),
                ("ghp_", "GitHub personal access token"),
                ("AKIA", "AWS access key ID"),
            ];
            let scan_extensions = ["md", "toml", "txt", "json", "yaml", "yml"];
            if let Ok(entries) = std::fs::read_dir(workspace_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                        if scan_extensions.contains(&ext)
                            && let Ok(content) = std::fs::read_to_string(&path) {
                                for (pattern, label) in &secret_patterns {
                                    if content.contains(pattern) {
                                        shield_warnings.push(format!(
                                            "Possible {} found in {}",
                                            label,
                                            path.display()
                                        ));
                                    }
                                }
                        }
                    }
                }
            }
        }

        // Print results
        if shield_warnings.is_empty() {
            println!("[OK] AgentShield: no security issues found");
        } else {
            println!(
                "\n[WARN] AgentShield — {} security issue(s):",
                shield_warnings.len()
            );
            for w in &shield_warnings {
                println!("  ⚠ {}", w);
            }
        }
    }

    // 14. Rate limiting — check whether HTTP rate limiting is active
    {
        let rl_enabled = config
            .gateway
            .as_ref()
            .map(|g| g.rate_limit.enabled)
            .unwrap_or(true); // default enabled
        if rl_enabled {
            let (global_rpm, llm_rpm, burst) = config
                .gateway
                .as_ref()
                .map(|g| (g.rate_limit.global_rpm, g.rate_limit.llm_rpm, g.rate_limit.burst_size))
                .unwrap_or((120, 20, 10));
            println!(
                "[OK] Rate limiting: enabled (global {}/min, LLM {}/min, burst {})",
                global_rpm, llm_rpm, burst
            );
        } else {
            println!("[WARN] Rate limiting: DISABLED — gateway is unprotected");
        }
    }

    // --repair: auto-fix common issues (shared logic via zeus_setup::ops::doctor)
    if repair {
        println!("\n--- Repair Mode ---\n");
        let zeus_dir = dirs::home_dir().unwrap_or_default().join(".zeus");
        let outcomes = zeus_setup::ops::doctor::perform_repairs_sync(
            &config.workspace,
            &config.sessions,
            &config_path,
            &zeus_dir,
        );
        let fixed = outcomes.len();
        for o in &outcomes {
            let tag = if o.success { "[FIXED]" } else { "[FAIL]" };
            println!("{} {}: {}", tag, o.name, o.detail);
        }
        if fixed == 0 {
            println!("Nothing to fix — all checks passed.");
        } else {
            println!("\n{} issue(s) fixed.", fixed);
        }
    }

    println!("\nDoctor check complete.");
    Ok(())
}

async fn run_import(config: Config, path: &str, from: &str) -> Result<()> {
    use std::path::Path;
    use zeus_agent::{ImportSource, MigrationEngine};

    let import_path = Path::new(path);
    if !import_path.exists() {
        anyhow::bail!("Import path does not exist: {}", path);
    }

    let engine = MigrationEngine::new(config.sessions.clone(), config.workspace.clone());

    // Determine source format
    let source = match from {
        "openclaw" => ImportSource::OpenClaw,
        "chatgpt" => ImportSource::ChatGPT,
        "generic" => ImportSource::Generic,
        _ => MigrationEngine::detect_source(import_path),
    };

    println!("Importing from {} format...", source);
    println!("  Source: {}", import_path.display());
    println!("  Sessions dir: {}", config.sessions.display());
    println!("  Workspace dir: {}", config.workspace.display());
    println!();

    let result = match source {
        ImportSource::OpenClaw => engine.import_openclaw(import_path).await?,
        ImportSource::ChatGPT => engine.import_chatgpt(import_path).await?,
        ImportSource::ClaudeExport => {
            println!("Claude export format is not fully supported yet. Trying generic import...");
            engine.import_generic(import_path).await?
        }
        ImportSource::Generic => engine.import_generic(import_path).await?,
    };

    // Print summary
    println!("Import Results");
    println!("==============");
    println!("  Sessions imported: {}", result.sessions_imported);
    println!("  Memories imported: {}", result.memories_imported);
    println!("  Skills imported:   {}", result.skills_imported);
    println!("  Total items:       {}", result.total_imported());

    if !result.warnings.is_empty() {
        println!("\nWarnings:");
        for w in &result.warnings {
            println!("  [WARN] {}", w);
        }
    }

    if !result.errors.is_empty() {
        println!("\nErrors ({}):", result.errors.len());
        for e in &result.errors {
            println!("  [ERR] {}", e);
        }
    }

    if result.total_imported() == 0 && result.errors.is_empty() {
        println!("\nNo importable data found at the specified path.");
    } else if result.errors.is_empty() {
        println!("\nImport completed successfully.");
    } else {
        println!("\nImport completed with {} error(s).", result.errors.len());
    }

    Ok(())
}

/// Start the standalone Pantheon IRC server.
///
/// Reads `[pantheon_server]` from config.toml, applies any CLI overrides
/// for host/port, then runs the WebSocket accept loop until the process is
/// killed. Agents connect by adding a `[pantheon]` section to their own
/// config.toml with the matching `channel_key`.
async fn run_pantheon_server(
    config: Config,
    host_override: Option<String>,
    port_override: Option<u16>,
) -> Result<()> {
    use zeus_pantheon_server::{config::PantheonServerConfig, server, state::ServerState};

    // Read [pantheon_server] section from config.toml directly — the main Config
    // struct doesn't carry this field (it lives in the pantheon-server crate to
    // avoid circular deps between zeus-core and zeus-pantheon-server).
    let config_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("config.toml");
    let mut server_config: Option<PantheonServerConfig> = None;
    if config_path.exists() {
        if let Ok(raw) = std::fs::read_to_string(&config_path) {
            if let Ok(table) = raw.parse::<toml::Table>() {
                if let Some(section) = table.get("pantheon_server") {
                    match section.clone().try_into::<PantheonServerConfig>() {
                        Ok(cfg) => server_config = Some(cfg),
                        Err(e) => tracing::warn!("Failed to parse [pantheon_server]: {}", e),
                    }
                }
            }
        }
    }
    let mut server_config = server_config.unwrap_or_else(|| {
        tracing::warn!(
            "No [pantheon_server] section in config.toml — using defaults (port 6669, no channel_key). \
             Set a channel_key before exposing to the network."
        );
        PantheonServerConfig {
            host: "0.0.0.0".into(),
            port: 6669,
            channel_key: String::new(),
            admin_ids: vec![],
            default_channels: vec![
                "#general".into(),
                "#fleet-ops".into(),
                "#dev".into(),
            ],
            history_limit: 500,
            tls: false,
            cert_path: None,
            key_path: None,
            rate_burst: 10,
            rate_per_sec: 2.0,
            nick_reservation: true,
            motd: "Welcome to Pantheon — Zeus agent fleet communication hub.".into(),
        }
    });

    // CLI overrides take precedence over config.toml.
    if let Some(host) = host_override {
        server_config.host = host;
    }
    if let Some(port) = port_override {
        server_config.port = port;
    }

    if server_config.channel_key.is_empty() {
        tracing::warn!(
            "⚠ Pantheon server has no channel_key — any client can connect without auth. \
             Set [pantheon_server] channel_key in config.toml for production use."
        );
    }

    let state = ServerState::new(&server_config.default_channels, server_config.history_limit);

    tracing::info!(
        "Starting Pantheon server on {}:{} ({} default channels, {} admins)",
        server_config.host,
        server_config.port,
        server_config.default_channels.len(),
        server_config.admin_ids.len(),
    );

    server::run(server_config, state).await
}

async fn run_pantheon(config: Config, action: PantheonAction) -> Result<()> {
    let gateway_port = std::env::var("ZEUS_GATEWAY_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or_else(|| config.gateway.as_ref().map(|g| g.port).unwrap_or(8080));
    let base = format!("http://127.0.0.1:{}", gateway_port);
    let token = std::env::var("ZEUS_API_TOKEN").unwrap_or_default();
    let auth = format!("Bearer {}", token);
    let client = reqwest::Client::new();

    match action {
        PantheonAction::List => {
            let resp = client
                .get(format!("{}/v1/pantheon/missions", base))
                .header("Authorization", &auth)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Gateway unreachable: {}", e))?;
            let body: serde_json::Value = resp.json().await?;
            let missions = body["missions"]
                .as_array()
                .cloned()
                .or_else(|| body.as_array().cloned())
                .unwrap_or_default();
            if missions.is_empty() {
                println!("No active missions.");
            } else {
                println!("{:<36}  {:<10}  {:<6}  GOAL", "ID", "STATUS", "PROG%");
                println!("{}", "-".repeat(90));
                for m in &missions {
                    println!(
                        "{:<36}  {:<10}  {:<6}  {}",
                        m["id"].as_str().unwrap_or("-"),
                        m["status"].as_str().unwrap_or("-"),
                        format!("{:.0}%", m["progress_pct"].as_f64().unwrap_or(0.0)),
                        m["goal"].as_str().unwrap_or("-"),
                    );
                }
                println!("\n{} mission(s).", missions.len());
            }
        }

        PantheonAction::Status { mission_id } => {
            let resp = client
                .get(format!("{}/v1/pantheon/missions/{}", base, mission_id))
                .header("Authorization", &auth)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Gateway unreachable: {}", e))?;
            let m: serde_json::Value = resp.json().await?;
            let status = m["status"].as_str().unwrap_or("?");
            let progress = m["progress_pct"].as_f64().unwrap_or(0.0);
            let done = m["tasks_done"].as_u64().unwrap_or(0);
            let total = m["tasks_total"].as_u64().unwrap_or(0);
            let tokens = m["tokens_used"].as_u64().unwrap_or(0);
            println!("Mission: {}", mission_id);
            println!("  Goal:     {}", m["goal"].as_str().unwrap_or("?"));
            println!("  Status:   {} ({:.0}%)", status, progress);
            println!("  Tasks:    {}/{}", done, total);
            println!("  Tokens:   {}", tokens);
            if let Some(team) = m["team"].as_array() {
                println!("  Team ({}):", team.len());
                for member in team {
                    println!(
                        "    - {} [{}] — {}",
                        member["name"].as_str().unwrap_or("?"),
                        member["role"].as_str().unwrap_or("?"),
                        member["status"].as_str().unwrap_or("?"),
                    );
                }
            }
            if let Some(tasks) = m["tasks"].as_array()
                && !tasks.is_empty()
            {
                println!("  Tasks:");
                for t in tasks {
                    let icon = match t["status"].as_str().unwrap_or("") {
                        "complete" | "approved" => "✓",
                        "in_progress" => "↻",
                        "awaiting_review" => "?",
                        "failed" | "rejected" => "✗",
                        _ => "○",
                    };
                    println!(
                        "    {} {} → {}",
                        icon,
                        t["description"].as_str().unwrap_or("?"),
                        t["assigned_to"].as_str().unwrap_or("unassigned"),
                    );
                }
            }
        }

        PantheonAction::Join {
            mission_id,
            interval,
        } => {
            println!("Joining mission {} (Ctrl+C to exit)", mission_id);
            println!(
                "Polling every {}s — watching live activity feed...\n",
                interval
            );

            let mut seen_count = 0usize;
            loop {
                let resp = client
                    .get(format!("{}/v1/pantheon/missions/{}/feed", base, mission_id))
                    .header("Authorization", &auth)
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!("Gateway unreachable: {}", e))?;

                let feed: serde_json::Value = resp.json().await.unwrap_or_default();
                let entries = feed.as_array().cloned().unwrap_or_default();

                // Print new entries only
                if entries.len() > seen_count {
                    for entry in &entries[seen_count..] {
                        let agent = entry["agent_name"].as_str().unwrap_or("?");
                        let activity = entry["activity"].as_str().unwrap_or("?");
                        let detail = entry["detail"]
                            .as_str()
                            .map(|s| s.to_string())
                            .or_else(|| {
                                entry["detail"]
                                    .get("summary")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                            })
                            .unwrap_or_default();
                        let ts = entry["timestamp"].as_str().unwrap_or("");
                        let ts_short = if ts.len() >= 19 { &ts[11..19] } else { ts };
                        if detail.is_empty() {
                            println!("[{}] {} — {}", ts_short, agent, activity);
                        } else {
                            println!("[{}] {} — {}: {}", ts_short, agent, activity, detail);
                        }
                    }
                    seen_count = entries.len();
                }

                // Also show mission status
                let status_resp = client
                    .get(format!("{}/v1/pantheon/missions/{}", base, mission_id))
                    .header("Authorization", &auth)
                    .send()
                    .await;
                if let Ok(sr) = status_resp
                    && let Ok(m) = sr.json::<serde_json::Value>().await
                {
                    let status = m["status"].as_str().unwrap_or("?");
                    let pct = m["progress_pct"].as_f64().unwrap_or(0.0);
                    // Exit when mission is done
                    if matches!(status, "done" | "complete" | "failed" | "cancelled") {
                        println!("\nMission {}: {} ({:.0}%)", mission_id, status, pct);
                        if let Some(summary) = m["summary"].as_str() {
                            println!("Summary: {}", summary);
                        }
                        break;
                    }
                }

                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            }
        }
    }

    Ok(())
}

async fn run_fleet(config: Config, action: FleetAction) -> Result<()> {
    let gateway_port = std::env::var("ZEUS_GATEWAY_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or_else(|| config.gateway.as_ref().map(|g| g.port).unwrap_or(8080));
    let base = format!("http://127.0.0.1:{}", gateway_port);
    let token = std::env::var("ZEUS_API_TOKEN").unwrap_or_default();
    let auth = format!("Bearer {}", token);
    let client = reqwest::Client::new();

    match action {
        FleetAction::List => {
            let resp = client
                .get(format!("{}/v1/fleet", base))
                .header("Authorization", &auth)
                .send()
                .await
                .map_err(|e| {
                    anyhow::anyhow!("Gateway unreachable (is `zeus gateway` running?): {}", e)
                })?;
            let body: serde_json::Value = resp.json().await?;
            let agents = body["agents"].as_array().cloned().unwrap_or_default();
            if agents.is_empty() {
                println!("No fleet agents registered.");
            } else {
                println!(
                    "{:<36}  {:<16}  {:<12}  {:<20}  STATUS",
                    "ID", "HOST", "ROLE", "MODEL"
                );
                println!("{}", "-".repeat(100));
                for a in &agents {
                    let meta = &a["metadata"];
                    println!(
                        "{:<36}  {:<16}  {:<12}  {:<20}  {}",
                        a["id"].as_str().unwrap_or("-"),
                        meta["ip"]
                            .as_str()
                            .unwrap_or(a["host"].as_str().unwrap_or("-")),
                        meta["role"]
                            .as_str()
                            .unwrap_or(a["role"].as_str().unwrap_or("-")),
                        a["model"].as_str().unwrap_or("-"),
                        a["status"].as_str().unwrap_or("-"),
                    );
                }
                println!("\n{} agent(s) registered.", agents.len());
            }
        }

        FleetAction::Add {
            host,
            user,
            os,
            role,
            model,
            port,
        } => {
            println!("Provisioning agent on {}@{}:{} ...", user, host, port);
            let payload = serde_json::json!({
                "host": host,
                "user": user,
                "os": os,
                "agent_role": role,
                "model": model,
                "port": port,
            });
            let resp = client
                .post(format!("{}/v1/fleet/provision", base))
                .header("Authorization", &auth)
                .json(&payload)
                .send()
                .await
                .map_err(|e| {
                    anyhow::anyhow!("Gateway unreachable (is `zeus gateway` running?): {}", e)
                })?;
            let status = resp.status();
            let body: serde_json::Value = resp.json().await?;
            if status.is_success() {
                let job_id = body["job_id"].as_str().unwrap_or("?");
                println!("Provisioning started. Job ID: {}", job_id);
                println!("Poll status with: zeus fleet status {}", job_id);
            } else {
                anyhow::bail!("Provision failed ({}): {}", status, body);
            }
        }

        FleetAction::Remove { id } => {
            let resp = client
                .delete(format!("{}/v1/fleet/{}", base, id))
                .header("Authorization", &auth)
                .send()
                .await
                .map_err(|e| {
                    anyhow::anyhow!("Gateway unreachable (is `zeus gateway` running?): {}", e)
                })?;
            if resp.status().is_success() {
                println!("Agent {} deregistered.", id);
            } else {
                let body: serde_json::Value = resp.json().await?;
                anyhow::bail!("Remove failed: {}", body);
            }
        }

        FleetAction::Status { id } => {
            let resp = client
                .get(format!("{}/v1/fleet/provision/status/{}", base, id))
                .header("Authorization", &auth)
                .send()
                .await
                .map_err(|e| {
                    anyhow::anyhow!("Gateway unreachable (is `zeus gateway` running?): {}", e)
                })?;
            let body: serde_json::Value = resp.json().await?;
            let status = body["status"].as_str().unwrap_or("unknown");
            let progress = body["progress"].as_u64().unwrap_or(0);
            let message = body["message"].as_str().unwrap_or("");
            println!("Job {}: {} ({}%) — {}", id, status, progress, message);
            if let Some(logs) = body["logs"].as_array() {
                for log in logs.iter().rev().take(10) {
                    println!("  {}", log.as_str().unwrap_or(""));
                }
            }
        }
    }

    Ok(())
}
