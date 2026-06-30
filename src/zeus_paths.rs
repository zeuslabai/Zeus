//! Shared path helpers for Zeus runtime paths.
//!
//! All modules that need `ZEUS_HOME` or the gateway PID file path
//! MUST use these helpers to ensure writer/reader agreement on paths.

use std::path::PathBuf;

/// Resolve the Zeus home directory.
///
/// Priority:
/// 1. `$ZEUS_HOME` env var (if set and non-empty)
/// 2. `~/.zeus` (default)
///
/// This logic MUST match `zeus-setup::config::zeus_home()` and
/// `scripts/install.sh`'s `--zeus-home` / `$ZEUS_HOME` handling.
pub fn zeus_home() -> PathBuf {
    if let Ok(custom) = std::env::var("ZEUS_HOME") {
        if !custom.is_empty() {
            return PathBuf::from(custom);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".zeus")
}

/// Resolve the gateway PID file path.
///
/// Always `{zeus_home}/gateway.pid`.
/// Used by both the writer (`gateway_lock.rs`) and readers (`daemon.rs`).
pub fn zeus_pid_path() -> PathBuf {
    zeus_home().join("gateway.pid")
}
