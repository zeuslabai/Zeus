//! Shared path helpers for Zeus runtime paths.
//!
//! All modules that need `ZEUS_HOME` or the gateway PID file path
//! MUST use these helpers to ensure writer/reader agreement on paths.

use std::path::PathBuf;

/// Resolve the Zeus home directory.
///
/// Priority (#248 — must survive daemon(8)/service managers with no $HOME):
/// 1. `$ZEUS_HOME` env var (if set and non-empty)
/// 2. `$HOME/.zeus` — via `dirs::home_dir()`, which on Unix falls back to
///    the passwd entry (`getpwuid_r`) when `$HOME` is unset/empty, so a
///    daemon(8)-scrubbed environment still resolves the *service user's*
///    real home.
/// 3. Last resort: a uid-scoped path under `/var/tmp` — NEVER the shared
///    `/tmp/.zeus`, which another user may own (the old fallback turned a
///    missing $HOME into an EACCES at gateway lock acquire).
///
/// This logic MUST match `zeus-setup::config::zeus_home()` and
/// `scripts/install.sh`'s `--zeus-home` / `$ZEUS_HOME` handling.
pub fn zeus_home() -> PathBuf {
    if let Ok(custom) = std::env::var("ZEUS_HOME")
        && !custom.is_empty()
    {
        return PathBuf::from(custom);
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".zeus");
    }
    // No $ZEUS_HOME, no $HOME, no passwd entry (chroot/jail edge case):
    // fall back to a per-uid dir that this uid can always create — never a
    // path potentially owned by root or another user.
    let uid = current_uid();
    PathBuf::from(format!("/var/tmp/zeus-uid{uid}")).join(".zeus")
}

/// Resolve the gateway PID file path.
///
/// Always `{zeus_home}/gateway.pid`.
/// Used by both the writer (`gateway_lock.rs`) and readers (`daemon.rs`).
pub fn zeus_pid_path() -> PathBuf {
    zeus_home().join("gateway.pid")
}

/// Current numeric uid — 0 on non-unix targets (#308: Windows has no uid;
/// the value is only used for diagnostics and fallback path naming).
pub fn current_uid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::getuid()
    }
    #[cfg(not(unix))]
    0u32
}
