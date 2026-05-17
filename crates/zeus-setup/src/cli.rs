//! CLI argument parsing with clap

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "zeus-setup",
    version,
    about = "Zeus installer, builder, and deployer",
    long_about = "Professional TUI installer for Zeus AI assistant.\n\
                   Run without arguments for interactive mode."
)]
pub struct Cli {
    /// Non-interactive mode (plain text output for CI/CD)
    #[arg(long, global = true)]
    pub non_interactive: bool,

    /// Theme name (dark, light, nord, dracula, catppuccin, etc.)
    #[arg(long, global = true, default_value = "dark")]
    pub theme: String,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Install Zeus locally
    Install {
        /// Install specific version
        #[arg(long)]
        version: Option<String>,

        /// Install from a pre-built local binary
        #[arg(long)]
        local: Option<PathBuf>,

        /// Build from source (requires Cargo)
        #[arg(long)]
        source: bool,

        /// Install prefix directory (~/.local by default)
        #[arg(long)]
        prefix: Option<PathBuf>,

        /// Re-run configuration wizard and overwrite existing config
        #[arg(long)]
        reconfigure: bool,
    },

    /// Build from source (CLI, web, FFI, macOS, iOS)
    Build {
        /// Git pull before building
        #[arg(long)]
        pull: bool,

        /// Run test suite before building
        #[arg(long)]
        test: bool,

        /// Build web frontend (trunk + WASM)
        #[arg(long)]
        web: bool,

        /// Build Rust->Swift FFI (universal binary + XCFramework)
        #[arg(long)]
        ffi: bool,

        /// Build macOS Desktop app (implies --ffi)
        #[arg(long)]
        macos: bool,

        /// Build iOS app
        #[arg(long)]
        ios: bool,

        /// Build all Apple targets (FFI + macOS + iOS)
        #[arg(long)]
        apple: bool,

        /// Regenerate Xcode projects via xcodegen
        #[arg(long)]
        xcode: bool,

        /// Skip binary installation after build
        #[arg(long)]
        no_install: bool,

        /// Skip daemon restart after install
        #[arg(long)]
        no_restart: bool,

        /// Build CLI only (no web, no FFI)
        #[arg(long)]
        cli_only: bool,

        /// Build web frontend only
        #[arg(long)]
        web_only: bool,

        /// Configure MCP after build
        #[arg(long)]
        mcp: bool,

        /// Build everything: test + CLI + web + install + restart
        #[arg(long)]
        all: bool,

        /// Full deploy: test + CLI + web + install + MCP
        #[arg(long)]
        deploy: bool,

        /// Number of parallel compile jobs
        #[arg(short, long)]
        jobs: Option<usize>,
    },

    /// Deploy to remote fleet hosts via SSH
    Deploy {
        /// Fleet node shortnames (.100) or user@host
        targets: Vec<String>,

        /// Deploy to all nodes in fleet.conf
        #[arg(long)]
        all: bool,

        /// Read deploy targets from file
        #[arg(long)]
        hosts: Option<PathBuf>,

        /// List fleet nodes and exit
        #[arg(long)]
        list: bool,

        /// Full setup: create ~/.zeus/, config.toml, .env, service, MCP (default: true on first deploy)
        #[arg(long)]
        setup: bool,

        /// Only push config files (no binary)
        #[arg(long)]
        config_only: bool,

        /// Skip workspace/config setup (binary only)
        #[arg(long)]
        no_setup: bool,

        /// Install and start gateway service (launchd/rc.d)
        #[arg(long)]
        service: bool,
    },

    /// Deploy web frontend to .226 FreeBSD
    DeployWeb {
        /// Git branch to checkout before building
        #[arg(short, long)]
        branch: Option<String>,

        /// Skip git pull, use local state
        #[arg(long)]
        skip_pull: bool,
    },

    /// Package for distribution (.pkg installer)
    Package {
        /// Only create CLI .pkg installer (no Desktop, web, extras)
        #[arg(long)]
        cli_only: bool,

        /// Only create Desktop app package
        #[arg(long)]
        app_only: bool,

        /// Override version string
        #[arg(long)]
        version: Option<String>,

        /// Code signing identity (Developer ID Installer certificate)
        #[arg(long)]
        sign: Option<String>,

        /// Output directory
        #[arg(long)]
        dist_dir: Option<PathBuf>,

        /// Notarize the package with Apple (requires --sign, --apple-id, --team-id)
        #[arg(long)]
        notarize: bool,

        /// Apple ID for notarization
        #[arg(long, env = "ZEUS_APPLE_ID")]
        apple_id: Option<String>,

        /// Apple Developer Team ID for notarization
        #[arg(long, env = "ZEUS_TEAM_ID")]
        team_id: Option<String>,

        /// Skip cargo build (use existing binaries in target/release)
        #[arg(long)]
        skip_build: bool,
    },

    /// Configure MCP integration for Claude
    Mcp {
        /// Configure for Claude Code
        #[arg(long)]
        code: bool,

        /// Configure for Claude Desktop
        #[arg(long)]
        desktop: bool,

        /// Remove MCP configurations
        #[arg(long)]
        remove: bool,

        /// Show current MCP configuration
        #[arg(long)]
        show: bool,
    },

    /// Manage gateway service (launchd/systemd/rc.d)
    Service {
        /// Service action
        action: Option<ServiceAction>,
    },

    /// Run diagnostics
    Doctor,
}

#[derive(Clone, ValueEnum)]
pub enum ServiceAction {
    Install,
    Start,
    Stop,
    Restart,
    Status,
    Logs,
    Uninstall,
}
