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

/// External (non-workspace) crates whose warnings we deliberately let through
/// the filter, as `(target, level)` directives.
///
/// `serenity`: its shard runner handles Discord gateway disconnects/resumes
/// *internally* — close codes (e.g. 4004 auth-failed, session-timeout) are
/// logged inside serenity at warn/error and never reach our adapter. Without
/// this directive the workspace allowlist silently drops them, which is how
/// shard-level session drops stayed invisible (zeus112 case study — sessions
/// dropped every few minutes with zero logged reason). Capped at `warn` so
/// serenity's chatty info/debug stream stays out of gateway.log.
///
/// #332 ②: the same silent-drop class exists in EVERY adapter SDK that
/// manages its own connection — reconnects/timeouts/auth failures are logged
/// inside the SDK crate and dropped by the workspace allowlist. All
/// connection-owning SDKs in zeus-channels ride through at `warn` (crate
/// names use `_` in tracing targets):
///   serenity/songbird (Discord + voice) · grammers (Telegram MTProto) ·
///   matrix_sdk (Matrix sync loop) · rumqttc (MQTT) · lettre/async_imap
///   (SMTP/IMAP) · tokio_tungstenite/tungstenite (raw WebSocket adapters:
///   Slack RTM, WebChat, Nostr relays, etc.) · hyper/reqwest (HTTP transport
///   errors under webhook adapters).
const EXTERNAL_TARGETS: &[(&str, &str)] = &[
    ("serenity", "warn"),
    ("songbird", "warn"),
    ("grammers_client", "warn"),
    ("grammers_session", "warn"),
    ("matrix_sdk", "warn"),
    ("rumqttc", "warn"),
    ("lettre", "warn"),
    ("async_imap", "warn"),
    ("tokio_tungstenite", "warn"),
    ("tungstenite", "warn"),
    ("hyper", "warn"),
    ("reqwest", "warn"),
];

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

/// Build an [`EnvFilter`] covering **all** workspace crates at `level`,
/// plus `[logging.targets]` per-subsystem overrides (#332 ⑤).
///
/// If `RUST_LOG` is set and non-empty it wins verbatim — no directives are
/// added, so operator overrides behave exactly like stock `tracing`.
/// Config overrides are applied LAST, so they win over the base level,
/// the event-target defaults, and the external-SDK warn caps — but never
/// over an explicit `RUST_LOG`, which keeps its stock-tracing semantics of
/// being the operator's final word.
pub fn workspace_filter_with_overrides(
    level: &str,
    overrides: &std::collections::HashMap<String, String>,
) -> EnvFilter {
    let rust_log_set = rust_log_is_set();
    let mut filter = if rust_log_set {
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
    // External-crate forensics directives ride every filter for the same
    // reason: a Discord shard disconnect that only serenity sees must land in
    // gateway.log even under an operator RUST_LOG that doesn't mention
    // serenity. warn-capped, so they can't flood anything.
    for (target, level) in EXTERNAL_TARGETS {
        if let Ok(directive) = format!("{}={}", target, level).parse() {
            filter = filter.add_directive(directive);
        }
    }
    // #332 ⑤ — [logging.targets] knobs, last so they win (EnvFilter resolves
    // conflicting directives for the same target in favor of the last added).
    // Skipped entirely under RUST_LOG: the env override stays absolute.
    if !rust_log_set {
        for (target, lvl) in overrides {
            if let Ok(directive) = format!("{}={}", target, sanitize_level(lvl)).parse() {
                filter = filter.add_directive(directive);
            }
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
    let console_filter = workspace_filter_with_overrides(&resolved.console_level, &cfg.targets);

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
                .with_filter(workspace_filter_with_overrides(
                    &resolved.file_level,
                    &cfg.targets,
                ))
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
    /// (dev, inode) of the open file — external-deletion detection (#332 ①).
    /// `None` on platforms without stable file identity (non-unix).
    file_id: Option<(u64, u64)>,
    /// Last time we verified the active path still points at our open file.
    /// Throttles the per-write stat to at most once per second.
    last_liveness_check: std::time::Instant,
    /// True while writes are falling back to stderr, so the fallback notice
    /// prints once per outage instead of once per line.
    fallback_notified: bool,
}

/// (dev, inode) for external-deletion detection. `None` where unsupported.
#[cfg(unix)]
fn file_identity(m: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((m.dev(), m.ino()))
}
#[cfg(not(unix))]
fn file_identity(_m: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}

/// #332 ① pure decision: must the active file be reopened?
///
/// The rm-~/.zeus case: after an external `rm`, the held fd keeps accepting
/// writes into the *unlinked* inode — every line silently lands in a black
/// hole while the path stays missing. Detection is identity-based: reopen when
/// the path is gone, or when it exists but names a different (dev, inode) than
/// the file we hold (external rotate/replace). When identities are unavailable
/// (non-unix) and the path exists, keep the fd — false reopens would truncate
/// tail -f sessions for no reason.
fn needs_reopen(
    open_id: Option<(u64, u64)>,
    path_exists: bool,
    path_id: Option<(u64, u64)>,
) -> bool {
    if !path_exists {
        return true;
    }
    match (open_id, path_id) {
        (Some(o), Some(p)) => o != p,
        _ => false,
    }
}

/// #332 ① pure decision: which events must be fsync'd to disk?
///
/// The sinks are unbuffered (every line is a direct write(2)), so lines
/// survive process death already — the remaining loss window is the OS page
/// cache (power loss / kernel panic). WARN+ lines are exactly the ones a
/// post-mortem needs, so those pay the `sync_data` cost; info/debug don't.
fn sync_for_level(level: &tracing::Level) -> bool {
    *level <= tracing::Level::WARN
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
            Ok(file) => {
                let file_id = file.metadata().ok().as_ref().and_then(file_identity);
                Some(Self {
                    inner: std::sync::Arc::new(std::sync::Mutex::new(StableFileInner {
                        dir: dir.to_path_buf(),
                        prefix: prefix.to_string(),
                        retention,
                        day: today(),
                        file,
                        file_id,
                        last_liveness_check: std::time::Instant::now(),
                        fallback_notified: false,
                    })),
                })
            }
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
            self.file_id = file.metadata().ok().as_ref().and_then(file_identity);
            self.file = file;
            self.day = now;
        }
    }

    /// #332 ① sink reliability: detect the held fd going stale (active path
    /// deleted or replaced under us — the rm-~/.zeus case) and reopen the
    /// stable name so subsequent lines land on disk again. Stat cost is
    /// throttled to once per second.
    fn reopen_if_stale(&mut self) {
        const LIVENESS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
        if self.last_liveness_check.elapsed() < LIVENESS_INTERVAL {
            return;
        }
        self.last_liveness_check = std::time::Instant::now();
        let active = self.dir.join(format!("{}.log", self.prefix));
        let meta = std::fs::metadata(&active);
        let path_exists = meta.is_ok();
        let path_id = meta.ok().as_ref().and_then(file_identity);
        if !needs_reopen(self.file_id, path_exists, path_id) {
            return;
        }
        // Recreate the directory too — rm -rf ~/.zeus takes logs/ with it.
        let _ = std::fs::create_dir_all(&self.dir);
        match std::fs::OpenOptions::new().create(true).append(true).open(&active) {
            Ok(file) => {
                self.file_id = file.metadata().ok().as_ref().and_then(file_identity);
                self.file = file;
                self.fallback_notified = false;
                // Lands as the first line of the fresh file — makes the gap
                // self-documenting in post-mortems.
                use std::io::Write as _;
                let _ = writeln!(
                    &self.file,
                    "[logging] active log file was deleted or replaced externally — sink reopened (#332)"
                );
            }
            Err(e) => {
                // Sink unwritable: fall back to stderr (once per outage) so
                // the failure itself is never silent. launchd/daemon(8)
                // redirect stderr to a wrapper logfile on service seats.
                if !self.fallback_notified {
                    eprintln!(
                        "zeus-logging: cannot reopen {}: {e} — falling back to stderr until the path is writable",
                        active.display()
                    );
                    self.fallback_notified = true;
                }
            }
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

/// Per-write handle: locks the shared state, rotates on day change, verifies
/// the fd is still live (#332 ①), writes — and durably syncs WARN+ events.
pub struct StableFileGuard {
    inner: std::sync::Arc<std::sync::Mutex<StableFileInner>>,
    /// WARN+ events sync to disk after write (see [`sync_for_level`]).
    sync_after_write: bool,
}

impl std::io::Write for StableFileGuard {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.rotate_if_needed();
        inner.reopen_if_stale();
        if inner.fallback_notified {
            // Sink is in a confirmed-unwritable outage: route the line to
            // stderr so it lands SOMEWHERE (service wrappers capture stderr).
            return std::io::stderr().write(buf);
        }
        let n = std::io::Write::write(&mut inner.file, buf)?;
        if self.sync_after_write {
            // WARN+ must survive power loss: push past the OS page cache.
            let _ = inner.file.sync_data();
        }
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        std::io::Write::flush(&mut inner.file)
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for StableFileWriter {
    type Writer = StableFileGuard;
    fn make_writer(&'a self) -> Self::Writer {
        StableFileGuard {
            inner: self.inner.clone(),
            sync_after_write: false,
        }
    }
    fn make_writer_for(&'a self, meta: &tracing::Metadata<'_>) -> Self::Writer {
        StableFileGuard {
            inner: self.inner.clone(),
            sync_after_write: sync_for_level(meta.level()),
        }
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

    /// Test convenience: base filter with no `[logging.targets]` overrides.
    fn workspace_filter(level: &str) -> EnvFilter {
        workspace_filter_with_overrides(level, &std::collections::HashMap::new())
    }

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
    fn external_targets_ride_the_filter() {
        // Discord shard-disconnect visibility: serenity handles gateway
        // disconnects internally, so its warn/error lines are the ONLY record
        // of close codes (4004 auth-failed etc.). The filter must pass them.
        assert!(
            EXTERNAL_TARGETS.contains(&("serenity", "warn")),
            "serenity=warn directive missing — shard disconnects go dark"
        );
        for (t, _) in EXTERNAL_TARGETS {
            assert!(
                !WORKSPACE_TARGETS.contains(t),
                "external target {t} collides with a workspace crate target"
            );
        }
        // Functional: the built filter must actually contain the directive.
        let rendered = workspace_filter("info").to_string();
        assert!(
            rendered.contains("serenity=warn"),
            "workspace_filter dropped serenity=warn: {rendered}"
        );
    }

    #[test]
    fn all_connection_owning_sdks_ride_the_filter() {
        // #332 ②: every SDK crate that owns a connection must pass its
        // warnings through, or that adapter's drops go dark (the zeus112
        // Discord case, generalized). Deletion of any entry = red here.
        let rendered = workspace_filter("info").to_string();
        for sdk in [
            "serenity",
            "songbird",
            "grammers_client",
            "grammers_session",
            "matrix_sdk",
            "rumqttc",
            "lettre",
            "async_imap",
            "tokio_tungstenite",
            "tungstenite",
            "hyper",
            "reqwest",
        ] {
            assert!(
                rendered.contains(&format!("{sdk}=warn")),
                "connection-owning SDK {sdk} missing from filter: {rendered}"
            );
        }
    }

    #[test]
    fn needs_reopen_decision_table() {
        // Path gone → reopen, regardless of identity availability.
        assert!(needs_reopen(Some((1, 2)), false, None));
        assert!(needs_reopen(None, false, None));
        // Path present, same identity → keep the fd.
        assert!(!needs_reopen(Some((1, 2)), true, Some((1, 2))));
        // Path present, DIFFERENT identity → replaced under us → reopen.
        assert!(needs_reopen(Some((1, 2)), true, Some((1, 3))));
        assert!(needs_reopen(Some((1, 2)), true, Some((9, 2))));
        // Identity unavailable (non-unix) and path exists → never reopen.
        assert!(!needs_reopen(None, true, None));
        assert!(!needs_reopen(Some((1, 2)), true, None));
        assert!(!needs_reopen(None, true, Some((1, 2))));
    }

    #[test]
    fn warn_and_error_sync_info_does_not() {
        assert!(sync_for_level(&tracing::Level::ERROR));
        assert!(sync_for_level(&tracing::Level::WARN));
        assert!(!sync_for_level(&tracing::Level::INFO));
        assert!(!sync_for_level(&tracing::Level::DEBUG));
        assert!(!sync_for_level(&tracing::Level::TRACE));
    }

    #[test]
    fn sink_reopens_after_external_delete() {
        let d = std::env::temp_dir().join(format!("zeus-log-reopen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let w = StableFileWriter::new(&d, "gateway", 7).unwrap();
        {
            use std::io::Write as _;
            let mut g = tracing_subscriber::fmt::MakeWriter::make_writer(&w);
            g.write_all(b"before-delete\n").unwrap();
        }
        // The rm-~/.zeus case: active file removed from under the held fd.
        std::fs::remove_file(d.join("gateway.log")).unwrap();
        // Defeat the 1s liveness throttle deterministically.
        w.inner.lock().unwrap().last_liveness_check =
            std::time::Instant::now() - std::time::Duration::from_secs(2);
        {
            use std::io::Write as _;
            let mut g = tracing_subscriber::fmt::MakeWriter::make_writer(&w);
            g.write_all(b"after-delete\n").unwrap();
        }
        let active = std::fs::read_to_string(d.join("gateway.log"))
            .expect("active file must exist again after reopen");
        assert!(
            active.contains("after-delete"),
            "post-delete line must land in the REOPENED file, not the unlinked inode: {active:?}"
        );
        assert!(
            active.contains("sink reopened"),
            "reopen must self-document as the first line: {active:?}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn target_overrides_win_over_base_and_external_caps() {
        // #332 ⑤: [logging.targets] overrides are added last, so they beat
        // the base workspace level AND the external warn caps.
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("zeus_channels".to_string(), "debug".to_string());
        overrides.insert("serenity".to_string(), "info".to_string());
        overrides.insert("zeus_api".to_string(), "warn".to_string());
        let rendered = workspace_filter_with_overrides("info", &overrides).to_string();
        // EnvFilter renders last-wins per target: the override directive must
        // be present, positioned after the base directive it overrides.
        let base_pos = rendered.find("zeus_channels=info");
        let override_pos = rendered.find("zeus_channels=debug");
        assert!(override_pos.is_some(), "override missing: {rendered}");
        if let (Some(b), Some(o)) = (base_pos, override_pos) {
            assert!(o > b, "override must be added after base: {rendered}");
        }
        let cap_pos = rendered.find("serenity=warn");
        let deep_pos = rendered.find("serenity=info");
        assert!(deep_pos.is_some(), "serenity override missing: {rendered}");
        if let (Some(c), Some(d)) = (cap_pos, deep_pos) {
            assert!(d > c, "override must follow the warn cap: {rendered}");
        }
        assert!(
            rendered.rfind("zeus_api=warn") > rendered.find("zeus_api=info"),
            "quiet-down override must follow base: {rendered}"
        );
        // Invalid level in an override sanitizes rather than corrupting.
        let mut bad = std::collections::HashMap::new();
        bad.insert("boot".to_string(), "shouting".to_string());
        let rendered = workspace_filter_with_overrides("info", &bad).to_string();
        assert!(
            rendered.contains("boot=info"),
            "bad override level must sanitize to info: {rendered}"
        );
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
