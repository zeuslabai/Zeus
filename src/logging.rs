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
//!    adds two stable-name file sinks under `{zeus_home}/logs/` —
//!    `gateway.log` (info+) and `error.log` (warn+) — active in *all* modes.
//!    Rotation renames the OLD file (`gateway.YYYY-MM-DD.log`) so the active
//!    filename never changes and `tail -f` survives rotation (#321).
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
/// Layers (#321 — exactly two files):
/// - **`logs/gateway.log`** (all modes, unless `[logging].file_enabled=false`):
///   everything at `file_level` (info+ by default), non-ANSI, stable filename.
///   Rotation renames the old day's file to `gateway.YYYY-MM-DD.log` and keeps
///   `retention_days` archives — `tail -f gateway.log` survives rotation.
/// - **`logs/error.log`**: warn+error only, same stable-name rotation.
/// - **Console layer**: stderr in normal interactive modes only. Skipped when
///   stderr is not a TTY (daemon/launchd redirects stderr into `error.log`,
///   and echoing info+ there would pollute it — raw panics still reach
///   `error.log` via the OS-level redirect). In TUI / MCP-stdio modes there is
///   no console layer at all — the two file sinks capture everything (the
///   tracing `target` field distinguishes sources); the legacy `zeus.log` /
///   `mcp-stdio.log` files are retired.
pub fn init_logging(verbose: bool, is_tui: bool, is_stdio_mcp: bool, cfg: &LoggingConfig) -> Result<()> {
    let resolved = resolve(verbose, cfg);
    let console_filter = workspace_filter(&resolved.console_level);

    // Durable stable-name file sinks under {zeus_home}/logs/ (#321):
    //   gateway.log — everything at file_level (info+ by default)
    //   error.log   — warn+error only
    // The active filenames never change; rotation renames the OLD day's file
    // to `{prefix}.YYYY-MM-DD.log`, so `tail -f` just works forever.
    let (file_layer, error_layer) = if resolved.file_enabled {
        let log_dir = zeus_paths::zeus_home().join("logs");
        std::fs::create_dir_all(&log_dir).ok();
        let retention = resolved.retention_days as usize;
        let gateway = StableFileWriter::new(&log_dir, "gateway", retention).map(|w| {
            fmt::layer()
                .with_writer(w)
                .with_ansi(false)
                .with_filter(workspace_filter(&resolved.file_level))
        });
        let error = StableFileWriter::new(&log_dir, "error", retention).map(|w| {
            fmt::layer()
                .with_writer(w)
                .with_ansi(false)
                .with_filter(tracing_subscriber::filter::LevelFilter::WARN)
        });
        (gateway, error)
    } else {
        (None, None)
    };

    if is_tui || is_stdio_mcp {
        // TUI: stderr logs corrupt the ratatui display. MCP stdio: logs
        // corrupt the JSON-RPC stream. The stable-name sinks above already
        // capture everything (the tracing `target` field distinguishes
        // tui/mcp sources from gateway internals), so the legacy per-mode
        // files (`zeus.log`, `mcp-stdio.log`) are retired — end state is
        // exactly two files (#321). If the file sink is disabled, fall back
        // to an append-mode `logs/gateway.log` so these modes are never
        // left sinkless.
        if file_layer.is_some() || error_layer.is_some() {
            tracing_subscriber::registry()
                .with(file_layer)
                .with(error_layer)
                .init();
        } else {
            let log_dir = zeus_paths::zeus_home().join("logs");
            std::fs::create_dir_all(&log_dir).ok();
            let log_file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_dir.join("gateway.log"))?;
            tracing_subscriber::registry()
                .with(
                    fmt::layer()
                        .with_writer(std::sync::Mutex::new(log_file))
                        .with_ansi(false)
                        .with_filter(console_filter),
                )
                .init();
        }
    } else {
        // Console layer only when stderr is a real TTY. Under launchd/daemon
        // supervision stderr is redirected into error.log — echoing the info+
        // console stream there would pollute the warn+ sink. Raw panics and
        // pre-init prints still reach error.log through the OS-level
        // redirect, which is exactly why StandardErrorPath points there.
        use std::io::IsTerminal as _;
        let console_on = std::io::stderr().is_terminal() || file_layer.is_none();
        let console = console_on.then(|| fmt::layer().with_filter(console_filter));
        tracing_subscriber::registry()
            .with(file_layer)
            .with(error_layer)
            .with(console)
            .init();
    }
    Ok(())
}

/// A stable-name file sink with restart- and day-boundary rotation (#321).
///
/// The active file is always `{dir}/{prefix}.log` — no date stamp — so
/// `tail -f` keeps working forever. When the calendar day changes (checked on
/// every write, and once at open for files left over from a previous run),
/// the OLD file is renamed to `{prefix}.{YYYY-MM-DD}.log` (stamped with the
/// day it belongs to) and a fresh active file is opened under the same
/// stable name. Date-stamped archives beyond `retention` are pruned.
#[derive(Clone)]
pub struct StableFileWriter {
    inner: std::sync::Arc<std::sync::Mutex<StableFileInner>>,
}

struct StableFileInner {
    dir: std::path::PathBuf,
    prefix: String,
    retention: usize,
    /// Day (YYYY-MM-DD) the currently-open file belongs to.
    day: String,
    file: std::fs::File,
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

impl StableFileWriter {
    pub fn new(dir: &std::path::Path, prefix: &str, retention: usize) -> Option<Self> {
        let active = dir.join(format!("{prefix}.log"));
        // Restart-boundary rotation: if the existing active file was last
        // written on an earlier day, archive it under that day before opening.
        if let Ok((len, mtime)) =
            std::fs::metadata(&active).and_then(|m| m.modified().map(|t| (m.len(), t)))
        {
            let day = chrono::DateTime::<chrono::Local>::from(mtime)
                .format("%Y-%m-%d")
                .to_string();
            if day != today() && len > 0 {
                let _ = std::fs::rename(&active, dir.join(format!("{prefix}.{day}.log")));
            }
        }
        prune_archives(dir, prefix, retention);
        match std::fs::OpenOptions::new().create(true).append(true).open(&active) {
            Ok(file) => Some(Self {
                inner: std::sync::Arc::new(std::sync::Mutex::new(StableFileInner {
                    dir: dir.to_path_buf(),
                    prefix: prefix.to_string(),
                    retention,
                    day: today(),
                    file,
                })),
            }),
            Err(e) => {
                eprintln!("warning: failed to open log sink {}: {e}", active.display());
                None
            }
        }
    }
}

impl StableFileInner {
    fn rotate_if_needed(&mut self) {
        let now = today();
        if now == self.day {
            return;
        }
        let active = self.dir.join(format!("{}.log", self.prefix));
        // Rename the OLD day's file out of the way, then reopen the stable
        // name. The rename is what keeps `tail -f {prefix}.log` alive.
        let _ = std::fs::rename(
            &active,
            self.dir.join(format!("{}.{}.log", self.prefix, self.day)),
        );
        prune_archives(&self.dir, &self.prefix, self.retention);
        if let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&active)
        {
            self.file = file;
            self.day = now;
        }
    }
}

/// Remove date-stamped archives (`{prefix}.YYYY-MM-DD.log`) beyond `retention`.
/// The active `{prefix}.log` never matches the archive shape and is never pruned.
fn prune_archives(dir: &std::path::Path, prefix: &str, retention: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut archives: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with(&format!("{prefix}."))
                    && n.ends_with(".log")
                    // "{prefix}." + "YYYY-MM-DD" + ".log"
                    && n.len() == prefix.len() + 15
            })
        })
        .collect();
    if archives.len() <= retention {
        return;
    }
    archives.sort(); // date-stamped names sort chronologically
    let excess = archives.len() - retention;
    for p in archives.into_iter().take(excess) {
        let _ = std::fs::remove_file(p);
    }
}

/// Per-write handle: locks the shared state, rotates on day change, writes.
pub struct StableFileGuard(std::sync::Arc<std::sync::Mutex<StableFileInner>>);

impl std::io::Write for StableFileGuard {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        inner.rotate_if_needed();
        std::io::Write::write(&mut inner.file, buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        std::io::Write::flush(&mut inner.file)
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for StableFileWriter {
    type Writer = StableFileGuard;
    fn make_writer(&'a self) -> Self::Writer {
        StableFileGuard(self.inner.clone())
    }
}

#[cfg(test)]
mod stable_writer_tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("zeus-logtest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn active_name_is_stable_no_date_stamp() {
        let d = scratch("stable");
        let w = StableFileWriter::new(&d, "gateway", 3).unwrap();
        {
            use std::io::Write as _;
            let mut g = tracing_subscriber::fmt::MakeWriter::make_writer(&w);
            g.write_all(b"hello\n").unwrap();
            g.flush().unwrap();
        }
        let names: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["gateway.log".to_string()]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn day_change_renames_old_file_and_reopens_stable_name() {
        let d = scratch("rotate");
        let w = StableFileWriter::new(&d, "gateway", 30).unwrap();
        {
            use std::io::Write as _;
            let mut g = tracing_subscriber::fmt::MakeWriter::make_writer(&w);
            g.write_all(b"old-day line\n").unwrap();
        }
        // Simulate a day boundary: backdate the open file's day marker.
        w.inner.lock().unwrap().day = "2000-01-01".into();
        {
            use std::io::Write as _;
            let mut g = tracing_subscriber::fmt::MakeWriter::make_writer(&w);
            g.write_all(b"new-day line\n").unwrap();
        }
        let archived = std::fs::read_to_string(d.join("gateway.2000-01-01.log")).unwrap();
        assert!(archived.contains("old-day line"));
        let active = std::fs::read_to_string(d.join("gateway.log")).unwrap();
        assert!(active.contains("new-day line"));
        assert!(!active.contains("old-day line"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn prune_keeps_newest_archives_and_never_touches_active() {
        let d = scratch("prune");
        for day in ["2000-01-01", "2000-01-02", "2000-01-03", "2000-01-04"] {
            std::fs::write(d.join(format!("error.{day}.log")), "x").unwrap();
        }
        std::fs::write(d.join("error.log"), "active").unwrap();
        prune_archives(&d, "error", 2);
        assert!(!d.join("error.2000-01-01.log").exists());
        assert!(!d.join("error.2000-01-02.log").exists());
        assert!(d.join("error.2000-01-03.log").exists());
        assert!(d.join("error.2000-01-04.log").exists());
        assert!(
            d.join("error.log").exists(),
            "active file must never be pruned"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
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
