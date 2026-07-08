//! Gateway logging initialization: filters, stderr layer, and the durable
//! rotating file sink.
//!
//! Fixes two long-standing observability holes (see gateway-observability audit):
//!
//! 1. **`--verbose` never reached workspace crates.** The old init added a
//!    single `zeus={level}` directive, which matches only the bin crate —
//!    `zeus_agent`, `zeus_api`, `zeus_channels`, etc. were invisible unless an
//!    external `RUST_LOG` was set. [`workspace_filter`] now emits a directive
//!    per workspace crate.
//! 2. **No durable log sink.** Foreground runs logged to stderr only and died
//!    with the terminal; launchd redirection never rotated. [`init_logging`]
//!    adds a `tracing-appender` daily-rotating file layer under
//!    `{zeus_home}/logs/` with retention, active in *all* modes.
//!
//! Policy: an explicit non-empty `RUST_LOG` always wins (we use it verbatim
//! and add no directives). Otherwise levels come from `--verbose` and the
//! `[logging]` config section.

use anyhow::Result;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};
use zeus_core::LoggingConfig;

use crate::zeus_paths;

/// Stable bare event targets emitted by the P2 observability pass (boot
/// banner, adapter lifecycle, cook lifecycle, msg in/out). These don't share
/// the `zeus` crate prefix, so they need their own directives or the
/// workspace filter silently drops them. Always enabled at INFO minimum so
/// operators can grep the file sink deterministically.
const EVENT_TARGETS: &[&str] = &["boot", "adapter", "cook", "msg"];

/// Every crate in the workspace that can emit tracing events, as a tracing
/// `target:` prefix (crate names use `_`, not `-`). The bin crate is `zeus`.
///
/// Keep in sync with `[workspace].members` in the root `Cargo.toml`.
const WORKSPACE_TARGETS: &[&str] = &[
    "zeus",
    "zeus_acp",
    "zeus_aegis",
    "zeus_agent",
    "zeus_agora",
    "zeus_api",
    "zeus_athena",
    "zeus_auth",
    "zeus_browser",
    "zeus_channels",
    "zeus_core",
    "zeus_council",
    "zeus_economy",
    "zeus_extensions",
    "zeus_hermes",
    "zeus_llm",
    "zeus_lsp",
    "zeus_marketplace",
    "zeus_mcp",
    "zeus_memory",
    "zeus_mnemosyne",
    "zeus_nous",
    "zeus_orchestra",
    "zeus_pantheon_server",
    "zeus_plugins",
    "zeus_prometheus",
    "zeus_sandbox",
    "zeus_session",
    "zeus_setup",
    "zeus_skills",
    "zeus_solana",
    "zeus_talos",
    "zeus_templates",
    "zeus_tts",
    "zeus_tui",
    "zeus_voice",
    "zeus_wallet",
];

/// Validate a level string, falling back to `info` on anything unknown.
fn sanitize_level(level: &str) -> &str {
    match level.to_ascii_lowercase().as_str() {
        "trace" => "trace",
        "debug" => "debug",
        "info" => "info",
        "warn" | "warning" => "warn",
        "error" => "error",
        _ => "info",
    }
}

/// True when the user supplied an explicit, non-empty `RUST_LOG`.
fn rust_log_is_set() -> bool {
    std::env::var("RUST_LOG").map(|v| !v.trim().is_empty()).unwrap_or(false)
}

/// Build an [`EnvFilter`] covering **all** workspace crates at `level`.
///
/// If `RUST_LOG` is set and non-empty it wins verbatim — no directives are
/// added, so operator overrides behave exactly like stock `tracing`.
pub fn workspace_filter(level: &str) -> EnvFilter {
    let mut filter = if rust_log_is_set() {
        EnvFilter::from_default_env()
    } else {
        let level = sanitize_level(level);
        let mut filter = EnvFilter::new("");
        for target in WORKSPACE_TARGETS {
            if let Ok(directive) = format!("{}={}", target, level).parse() {
                filter = filter.add_directive(directive);
            }
        }
        filter
    };
    // Stable P2 event targets ride every filter — including an explicit
    // RUST_LOG — so `grep 'target=boot'`-class forensics always work. The
    // directives are additive (INFO on four bare targets) and cannot lower
    // anything the operator asked for.
    for target in EVENT_TARGETS {
        if let Ok(directive) = format!("{}=info", target).parse() {
            filter = filter.add_directive(directive);
        }
    }
    filter
}

/// Resolved logging settings after combining `--verbose`, `[logging]`, and defaults.
pub struct ResolvedLogging {
    pub console_level: String,
    pub file_level: String,
    pub file_enabled: bool,
    pub retention_days: u32,
}

/// Combine CLI flags with the `[logging]` config section.
///
/// `--verbose` forces `debug` on both sinks (it must never *lower* verbosity).
pub fn resolve(verbose: bool, cfg: &LoggingConfig) -> ResolvedLogging {
    let console_level = if verbose {
        "debug".to_string()
    } else {
        sanitize_level(&cfg.level).to_string()
    };
    let file_level = if verbose {
        "debug".to_string()
    } else {
        sanitize_level(cfg.file_level.as_deref().unwrap_or(&cfg.level)).to_string()
    };
    ResolvedLogging {
        console_level,
        file_level,
        file_enabled: cfg.file_enabled,
        retention_days: cfg.retention_days.max(1),
    }
}

/// Peek the `[logging]` section from the config file *without* running the
/// full config loader (which can trigger the onboarding wizard and must not
/// run before logging is up). Malformed/missing config → defaults.
pub fn peek_logging_config(cli_config: Option<&str>) -> LoggingConfig {
    #[derive(serde::Deserialize, Default)]
    struct Peek {
        #[serde(default)]
        logging: Option<LoggingConfig>,
    }

    let path = match cli_config {
        Some(p) => std::path::PathBuf::from(p),
        None => zeus_paths::zeus_home().join("config.toml"),
    };
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| toml::from_str::<Peek>(&raw).ok())
        .and_then(|p| p.logging)
        .unwrap_or_default()
}

/// Initialize the global tracing subscriber.
///
/// Layers:
/// - **Rotating file sink** (all modes, unless `[logging].file_enabled=false`):
///   daily rotation to `{zeus_home}/logs/gateway.{YYYY-MM-DD}.log`, non-ANSI,
///   `retention_days` files kept (older ones pruned by `tracing-appender`).
/// - **Console/legacy layer**: stderr in normal modes; in TUI / MCP-stdio
///   modes, an *append-mode* file at `{zeus_home}/zeus.log` /
///   `{zeus_home}/mcp-stdio.log` (previously `File::create` — truncated on
///   every boot, losing prior-run evidence).
pub fn init_logging(verbose: bool, is_tui: bool, is_stdio_mcp: bool, cfg: &LoggingConfig) -> Result<()> {
    let resolved = resolve(verbose, cfg);
    let console_filter = workspace_filter(&resolved.console_level);

    // Durable rotating file sink under {zeus_home}/logs/.
    let file_layer = if resolved.file_enabled {
        let log_dir = zeus_paths::zeus_home().join("logs");
        std::fs::create_dir_all(&log_dir).ok();
        match tracing_appender::rolling::Builder::new()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix("gateway")
            .filename_suffix("log")
            .max_log_files(resolved.retention_days as usize)
            .build(&log_dir)
        {
            Ok(appender) => Some(
                fmt::layer()
                    .with_writer(appender)
                    .with_ansi(false)
                    .with_filter(workspace_filter(&resolved.file_level)),
            ),
            Err(e) => {
                eprintln!("warning: failed to initialize rotating log sink: {e}");
                None
            }
        }
    } else {
        None
    };

    if is_tui || is_stdio_mcp {
        // TUI: stderr logs corrupt the ratatui display. MCP stdio: logs
        // corrupt the JSON-RPC stream. Route the console layer to a legacy
        // file instead — append mode, so restarts no longer erase evidence.
        let log_dir = zeus_paths::zeus_home();
        std::fs::create_dir_all(&log_dir).ok();
        let log_name = if is_stdio_mcp { "mcp-stdio.log" } else { "zeus.log" };
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join(log_name))?;
        tracing_subscriber::registry()
            .with(file_layer)
            .with(
                fmt::layer()
                    .with_writer(std::sync::Mutex::new(log_file))
                    .with_ansi(false)
                    .with_filter(console_filter),
            )
            .init();
    } else {
        tracing_subscriber::registry()
            .with(file_layer)
            .with(fmt::layer().with_filter(console_filter))
            .init();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_level_falls_back_to_info() {
        assert_eq!(sanitize_level("TRACE"), "trace");
        assert_eq!(sanitize_level("warning"), "warn");
        assert_eq!(sanitize_level("bogus"), "info");
        assert_eq!(sanitize_level(""), "info");
    }

    #[test]
    fn resolve_verbose_forces_debug_everywhere() {
        let cfg = LoggingConfig {
            level: "warn".into(),
            file_level: Some("error".into()),
            ..Default::default()
        };
        let r = resolve(true, &cfg);
        assert_eq!(r.console_level, "debug");
        assert_eq!(r.file_level, "debug");
    }

    #[test]
    fn resolve_file_level_defaults_to_console_level() {
        let cfg = LoggingConfig {
            level: "warn".into(),
            ..Default::default()
        };
        let r = resolve(false, &cfg);
        assert_eq!(r.console_level, "warn");
        assert_eq!(r.file_level, "warn");
        assert!(r.file_enabled);
        assert_eq!(r.retention_days, 7);
    }

    #[test]
    fn resolve_respects_distinct_file_level() {
        let cfg = LoggingConfig {
            level: "info".into(),
            file_level: Some("debug".into()),
            ..Default::default()
        };
        let r = resolve(false, &cfg);
        assert_eq!(r.console_level, "info");
        assert_eq!(r.file_level, "debug");
    }

    #[test]
    fn workspace_targets_cover_key_crates() {
        // The audit's headline bug: these crates were invisible to --verbose.
        for t in ["zeus", "zeus_agent", "zeus_api", "zeus_channels", "zeus_core"] {
            assert!(WORKSPACE_TARGETS.contains(&t), "missing target {t}");
        }
    }

    #[test]
    fn event_targets_present_and_bare() {
        // P2 stable targets must stay in sync with the emit sites
        // (gateway boot banner, adapter lifecycle, cook envelope, msg flow)
        // and must never collide with a workspace crate prefix.
        for t in ["boot", "adapter", "cook", "msg"] {
            assert!(EVENT_TARGETS.contains(&t), "missing event target {t}");
            assert!(
                !WORKSPACE_TARGETS.contains(&t),
                "event target {t} collides with a crate target"
            );
        }
    }

    #[test]
    fn retention_never_zero() {
        let cfg = LoggingConfig {
            retention_days: 0,
            ..Default::default()
        };
        assert_eq!(resolve(false, &cfg).retention_days, 1);
    }
}
