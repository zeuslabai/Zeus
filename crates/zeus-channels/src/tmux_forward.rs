//! Shared tmux forwarding utilities for relay modules.
//!
//! Provides session-agnostic tmux message forwarding used by
//! Telegram, Slack, Matrix, and future relay adapters.

use std::sync::{Arc, OnceLock};
use dashmap::DashMap;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Per-session mutex to serialize concurrent `forward_to_tmux` calls.
/// Without this, concurrent forwards to the same session interleave keystrokes.
static SESSION_LOCKS: OnceLock<DashMap<String, Arc<Mutex<()>>>> = OnceLock::new();

fn session_lock(session: &str) -> Arc<Mutex<()>> {
    let map = SESSION_LOCKS.get_or_init(DashMap::new);
    map.entry(session.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// Find the tmux binary, checking common macOS paths since launchd has limited PATH.
fn tmux_binary() -> &'static str {
    if std::path::Path::new("/opt/homebrew/bin/tmux").exists() {
        "/opt/homebrew/bin/tmux"
    } else if std::path::Path::new("/usr/local/bin/tmux").exists() {
        "/usr/local/bin/tmux"
    } else {
        "tmux"
    }
}

/// Get the tmux socket path for the current user.
fn tmux_socket_path() -> String {
    let uid = std::process::Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "501".to_string());
    format!("/private/tmp/tmux-{}/default", uid)
}

/// Auto-detect the active tmux session for relay forwarding.
/// Finds the first attached session, or falls back to first listed session.
/// Runs at forwarding time (not boot), so it survives session renames/restarts.
pub async fn detect_active_tmux_session() -> Option<String> {
    let tmux = tmux_binary();
    let socket_path = tmux_socket_path();

    let output = tokio::process::Command::new(tmux)
        .args([
            "-S",
            &socket_path,
            "list-sessions",
            "-F",
            "#{session_name}:#{session_attached}",
        ])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut first_session = None;

    for line in stdout.lines() {
        if let Some((name, attached)) = line.rsplit_once(':') {
            if first_session.is_none() {
                first_session = Some(name.to_string());
            }
            if attached == "1" {
                return Some(name.to_string());
            }
        }
    }

    first_session
}

/// Resolve tmux session: explicit config > auto-detect attached > first available
pub async fn resolve_tmux_target(explicit: &Option<String>) -> Option<String> {
    if let Some(session) = explicit {
        return Some(session.clone());
    }
    detect_active_tmux_session().await
}

/// Forward a message to a tmux session by typing it via send-keys.
///
/// The formatted message is typed literally into the target tmux pane
/// and Enter is pressed, so incoming messages appear in the interactive
/// Claude Code session.
///
/// Acquires a per-session lock before sending to prevent interleaved
/// keystrokes when multiple relays forward concurrently to the same session.
pub async fn forward_to_tmux(session: &str, text: &str) {
    let lock = session_lock(session);
    let _guard = lock.lock().await;

    let tmux = tmux_binary();
    let socket_path = tmux_socket_path();

    let escaped = text.replace('\'', "'\\''");

    // Type the message literally into the tmux pane
    let result = tokio::process::Command::new(tmux)
        .args([
            "-S",
            &socket_path,
            "send-keys",
            "-t",
            session,
            "-l",
            &escaped,
        ])
        .output()
        .await;
    match &result {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("tmux send-keys failed (exit {}): {}", output.status, stderr);
            return;
        }
        Err(e) => {
            warn!("tmux send-keys spawn failed: {}", e);
            return;
        }
        _ => {}
    }

    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    // Press Enter
    let result = tokio::process::Command::new(tmux)
        .args(["-S", &socket_path, "send-keys", "-t", session, "C-m"])
        .output()
        .await;
    match &result {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("tmux Enter failed (exit {}): {}", output.status, stderr);
        }
        Err(e) => {
            warn!("tmux Enter spawn failed: {}", e);
        }
        _ => {}
    }

    info!("Forwarded to tmux {}: {} chars", session, text.len());
}
