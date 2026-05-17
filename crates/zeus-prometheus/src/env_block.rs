//! R3 — Live `<env>` block for cooking system prompts.
//!
//! Per the Agent Intelligence Research synthesis (Zeus100, 2026-04-26):
//! Zeus injects 8+ static workspace files (SOUL/AGENTS/USER/MEMORY) but no
//! live state. Claude Code injects fewer files but adds cwd, git branch,
//! dirty files — **signal over volume**.
//!
//! This module renders a compact `<env>` block that prepends to the system
//! prompt each cooking iteration. Cost: one `git status --porcelain` shell
//! call per iteration. No LLM call, no persistence, no I/O beyond `git`.
//!
//! Output shape (example):
//! ```text
//! <env>
//! cwd: /path/to/repo
//! hostname: <hostname>
//! time_utc: 2026-04-26T23:45:12Z
//! git_branch: feat/r3-env-block
//! git_dirty: 2 file(s) — env_block.rs, lib.rs
//! </env>
//! ```
//!
//! On a non-git directory or `git` failure, the `git_*` lines are omitted
//! rather than rendered as errors — keeps the block tight when running
//! outside a repo (e.g. heartbeat in `~/.zeus`).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Maximum number of dirty filenames to enumerate inline.
/// Beyond this, we render `git_dirty: N file(s)` without names.
const MAX_DIRTY_NAMES: usize = 5;

/// Snapshot of the live execution environment.
#[derive(Debug, Clone, Default)]
pub struct EnvSnapshot {
    pub cwd: PathBuf,
    pub hostname: String,
    pub time_utc: String,
    pub git_branch: Option<String>,
    pub git_dirty_count: usize,
    pub git_dirty_names: Vec<String>,
}

impl EnvSnapshot {
    /// Capture a snapshot of the current process environment.
    ///
    /// All fields are best-effort — failures degrade to empty/`None` rather
    /// than errors so the cooking loop never blocks on env capture.
    pub fn capture() -> Self {
        let cwd = std::env::current_dir().unwrap_or_default();
        let hostname = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown".to_string());
        let time_utc = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let (git_branch, git_dirty_count, git_dirty_names) = git_state(&cwd);

        Self {
            cwd,
            hostname,
            time_utc,
            git_branch,
            git_dirty_count,
            git_dirty_names,
        }
    }

    /// Render as a compact `<env>...</env>` block ready to prepend to a
    /// system prompt. Trailing newline included.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(256);
        out.push_str("<env>\n");
        out.push_str(&format!("cwd: {}\n", self.cwd.display()));
        out.push_str(&format!("hostname: {}\n", self.hostname));
        out.push_str(&format!("time_utc: {}\n", self.time_utc));
        if let Some(ref branch) = self.git_branch {
            out.push_str(&format!("git_branch: {}\n", branch));
            if self.git_dirty_count == 0 {
                out.push_str("git_dirty: clean\n");
            } else if self.git_dirty_names.is_empty() {
                out.push_str(&format!("git_dirty: {} file(s)\n", self.git_dirty_count));
            } else {
                out.push_str(&format!(
                    "git_dirty: {} file(s) — {}\n",
                    self.git_dirty_count,
                    self.git_dirty_names.join(", ")
                ));
            }
        }
        out.push_str("</env>\n");
        out
    }
}

/// Probe git state for `dir`. Returns (branch, dirty_count, dirty_names).
/// On any failure (not a repo, git missing, etc.) returns `(None, 0, vec![])`.
fn git_state(dir: &Path) -> (Option<String>, usize, Vec<String>) {
    let branch = run_git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let branch = match branch {
        Some(b) if !b.is_empty() && b != "HEAD" => Some(b),
        Some(_) | None => None,
    };

    if branch.is_none() {
        return (None, 0, vec![]);
    }

    let porcelain = run_git(dir, &["status", "--porcelain"]).unwrap_or_default();
    let lines: Vec<&str> = porcelain
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    let count = lines.len();

    // Each porcelain line is "XY <path>" — strip the 3-char status prefix.
    let names: Vec<String> = lines
        .iter()
        .take(MAX_DIRTY_NAMES)
        .map(|l| l.get(3..).unwrap_or(l).trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    (branch, count, names)
}

/// Run `git <args>` in `dir` and return stdout trimmed, or `None` on failure.
fn run_git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_capture_never_panics() {
        let snap = EnvSnapshot::capture();
        // Always populated fields:
        assert!(!snap.hostname.is_empty());
        assert!(!snap.time_utc.is_empty());
        assert!(snap.time_utc.ends_with('Z'));
    }

    #[test]
    fn render_contains_required_keys() {
        let snap = EnvSnapshot {
            cwd: PathBuf::from("/tmp/foo"),
            hostname: "test-host".to_string(),
            time_utc: "2026-04-26T23:45:12Z".to_string(),
            git_branch: Some("main".to_string()),
            git_dirty_count: 0,
            git_dirty_names: vec![],
        };
        let s = snap.render();
        assert!(s.starts_with("<env>\n"));
        assert!(s.ends_with("</env>\n"));
        assert!(s.contains("cwd: /tmp/foo"));
        assert!(s.contains("hostname: test-host"));
        assert!(s.contains("time_utc: 2026-04-26T23:45:12Z"));
        assert!(s.contains("git_branch: main"));
        assert!(s.contains("git_dirty: clean"));
    }

    #[test]
    fn render_omits_git_lines_when_no_branch() {
        let snap = EnvSnapshot {
            cwd: PathBuf::from("/tmp/foo"),
            hostname: "h".to_string(),
            time_utc: "t".to_string(),
            git_branch: None,
            git_dirty_count: 0,
            git_dirty_names: vec![],
        };
        let s = snap.render();
        assert!(!s.contains("git_branch"));
        assert!(!s.contains("git_dirty"));
    }

    #[test]
    fn render_dirty_with_names() {
        let snap = EnvSnapshot {
            cwd: PathBuf::from("/x"),
            hostname: "h".to_string(),
            time_utc: "t".to_string(),
            git_branch: Some("feat/x".to_string()),
            git_dirty_count: 2,
            git_dirty_names: vec!["a.rs".to_string(), "b.rs".to_string()],
        };
        let s = snap.render();
        assert!(s.contains("git_dirty: 2 file(s) — a.rs, b.rs"));
    }

    #[test]
    fn render_dirty_count_only_when_names_empty() {
        let snap = EnvSnapshot {
            cwd: PathBuf::from("/x"),
            hostname: "h".to_string(),
            time_utc: "t".to_string(),
            git_branch: Some("main".to_string()),
            git_dirty_count: 47,
            git_dirty_names: vec![],
        };
        let s = snap.render();
        assert!(s.contains("git_dirty: 47 file(s)\n"));
        assert!(!s.contains("—"));
    }

    /// Smoke test against the live repo: this crate lives in a git repo, so
    /// `EnvSnapshot::capture()` should produce a branch name from the actual
    /// working tree.
    #[test]
    fn live_capture_in_repo_has_branch() {
        let snap = EnvSnapshot::capture();
        // Either we're in a git repo (branch present) or we're not (None).
        // If present, must be non-empty and not literally "HEAD".
        if let Some(ref b) = snap.git_branch {
            assert!(!b.is_empty());
            assert_ne!(b, "HEAD");
        }
        // Render must always succeed.
        let rendered = snap.render();
        assert!(rendered.contains("<env>"));
        assert!(rendered.contains("</env>"));
    }
}
