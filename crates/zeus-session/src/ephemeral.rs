//! Ephemeral heartbeat sessions.
//!
//! Each heartbeat cook runs in an isolated, short-lived session so that
//! tool-using heartbeat work (multi-turn, tool_call → tool_result) doesn't
//! poison the main agent session or other heartbeat tasks running
//! concurrently.
//!
//! Design:
//! - **Deterministic-per-cook, unique-across-cooks** session IDs of the form
//!   `agent:heartbeat:{task}:{unix_ts}`.  The timestamp suffix means each
//!   tick gets its own session file; replays/retries can reuse the ID by
//!   passing the same timestamp.
//! - **Auto-cleanup** via `HeartbeatSessionGuard`.  The underlying `.jsonl`
//!   file is removed when the guard is dropped, so ephemeral work doesn't
//!   accumulate on disk.
//! - **Isolation from main sessions** — the `agent:heartbeat:` prefix keeps
//!   these files clearly namespaced and easy to sweep if cleanup is ever
//!   skipped (e.g. panic).
//!
//! This builds on `Session::resume_or_create`, which already loads-or-creates
//! deterministically from a `stable_id`.  We just provide the naming
//! convention + RAII cleanup.

use crate::Session;
use std::path::{Path, PathBuf};

/// Prefix for all ephemeral heartbeat session IDs.
pub const HEARTBEAT_PREFIX: &str = "agent:heartbeat";

/// Build a deterministic stable_id for a heartbeat cook.
///
/// Format: `agent:heartbeat:{task_slug}:{unix_ts}`
///
/// The `task_slug` is normalised (lowercase, non-alphanumeric → `-`) so it's
/// safe as a filename component.  The `unix_ts` makes each cook unique while
/// remaining deterministic if callers need to reconstruct the ID (e.g. for
/// cleanup after a crash).
pub fn ephemeral_session_id(task: &str, unix_ts: u64) -> String {
    let slug: String = task
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    // Collapse runs of '-' and trim.
    let slug = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() { "task" } else { slug.as_str() };
    format!("{}:{}:{}", HEARTBEAT_PREFIX, slug, unix_ts)
}

/// Returns true if the given stable_id was produced by `ephemeral_session_id`.
///
/// Useful for sweepers that want to clean up orphaned ephemeral files on
/// startup.
pub fn is_ephemeral_id(stable_id: &str) -> bool {
    stable_id.starts_with(HEARTBEAT_PREFIX)
}

/// RAII guard around an ephemeral session.
///
/// Holds an owned `Session` and deletes the backing `.jsonl` file on drop.
/// If the session was never flushed to disk (empty), drop is a no-op.
///
/// Note: drop is synchronous and best-effort — any I/O error during cleanup
/// is logged but not propagated.  Callers who need guaranteed cleanup should
/// call `HeartbeatSessionGuard::cleanup` explicitly before drop.
pub struct HeartbeatSessionGuard {
    session: Option<Session>,
    path: PathBuf,
}

impl HeartbeatSessionGuard {
    /// Create or resume an ephemeral session for a heartbeat cook.
    pub async fn acquire(
        sessions_dir: impl AsRef<Path>,
        task: &str,
        unix_ts: u64,
    ) -> Self {
        let id = ephemeral_session_id(task, unix_ts);
        let session = Session::resume_or_create(sessions_dir.as_ref(), &id).await;
        let path = session.path().to_path_buf();
        Self {
            session: Some(session),
            path,
        }
    }

    /// Borrow the inner session mutably for the cook's duration.
    pub fn session_mut(&mut self) -> &mut Session {
        self.session
            .as_mut()
            .expect("session present until guard is consumed")
    }

    /// Borrow the inner session immutably.
    pub fn session(&self) -> &Session {
        self.session
            .as_ref()
            .expect("session present until guard is consumed")
    }

    /// Explicit, synchronous cleanup.  Returns whether the file was removed.
    pub fn cleanup(mut self) -> bool {
        // Drop the session first so any open handles are released.
        self.session = None;
        remove_if_exists(&self.path)
    }
}

impl Drop for HeartbeatSessionGuard {
    fn drop(&mut self) {
        // Session is dropped before file removal on the normal path.
        self.session = None;
        let _ = remove_if_exists(&self.path);
    }
}

fn remove_if_exists(path: &Path) -> bool {
    if path.exists() {
        match std::fs::remove_file(path) {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to remove ephemeral heartbeat session"
                );
                false
            }
        }
    } else {
        false
    }
}

/// Sweep any stale ephemeral heartbeat session files left by prior runs.
///
/// Intended for startup: if a cook panicked and its guard never ran, the
/// `.jsonl` file survives.  This removes any file whose stem starts with
/// `agent:heartbeat:`.
///
/// Returns the number of files removed.
pub fn sweep_stale(sessions_dir: impl AsRef<Path>) -> std::io::Result<usize> {
    let dir = sessions_dir.as_ref();
    if !dir.exists() {
        return Ok(0);
    }
    let mut removed = 0usize;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Session files are `{stable_id}.jsonl`.
        if let Some(stem) = name_str.strip_suffix(".jsonl")
            && is_ephemeral_id(stem)
            && std::fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ephemeral_id_format() {
        let id = ephemeral_session_id("push-work", 1_700_000_000);
        assert_eq!(id, "agent:heartbeat:push-work:1700000000");
    }

    #[test]
    fn ephemeral_id_normalises_whitespace_and_case() {
        let id = ephemeral_session_id("Push Work!", 42);
        assert_eq!(id, "agent:heartbeat:push-work:42");
    }

    #[test]
    fn ephemeral_id_handles_empty_task() {
        let id = ephemeral_session_id("", 7);
        assert_eq!(id, "agent:heartbeat:task:7");
    }

    #[test]
    fn ephemeral_id_collapses_separator_runs() {
        let id = ephemeral_session_id("foo   bar___baz", 1);
        assert_eq!(id, "agent:heartbeat:foo-bar-baz:1");
    }

    #[test]
    fn is_ephemeral_id_matches_prefix() {
        assert!(is_ephemeral_id("agent:heartbeat:x:1"));
        assert!(!is_ephemeral_id("agent:discord:123"));
        assert!(!is_ephemeral_id(""));
    }

    #[tokio::test]
    async fn guard_creates_and_cleans_up() {
        let dir = tempdir().unwrap();
        let path = {
            let mut guard = HeartbeatSessionGuard::acquire(dir.path(), "report", 100).await;
            guard.session_mut().init().await.unwrap();
            // Add a message to force a flush.
            guard
                .session_mut()
                .add(zeus_core::Message::user("hi"))
                .await
                .unwrap();
            guard.session().path().to_path_buf()
        };
        // Guard dropped — file should be gone.
        assert!(!path.exists(), "ephemeral session file should be removed on drop");
    }

    #[tokio::test]
    async fn explicit_cleanup_removes_file() {
        let dir = tempdir().unwrap();
        let mut guard = HeartbeatSessionGuard::acquire(dir.path(), "report", 200).await;
        guard.session_mut().init().await.unwrap();
        guard
            .session_mut()
            .add(zeus_core::Message::user("x"))
            .await
            .unwrap();
        let path = guard.session().path().to_path_buf();
        assert!(path.exists());
        assert!(guard.cleanup());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn sweep_stale_removes_only_heartbeat_files() {
        let dir = tempdir().unwrap();
        // Create an ephemeral file manually.
        let eph = dir.path().join("agent:heartbeat:x:1.jsonl");
        std::fs::write(&eph, b"{}\n").unwrap();
        // Create a non-ephemeral file.
        let keep = dir.path().join("agent:discord:123.jsonl");
        std::fs::write(&keep, b"{}\n").unwrap();

        let removed = sweep_stale(dir.path()).unwrap();
        assert_eq!(removed, 1);
        assert!(!eph.exists());
        assert!(keep.exists());
    }

    #[tokio::test]
    async fn sweep_on_missing_dir_is_ok() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert_eq!(sweep_stale(&missing).unwrap(), 0);
    }
}
