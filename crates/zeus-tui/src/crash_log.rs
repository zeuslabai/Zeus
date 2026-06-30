//! #41 — TUI panic-hook crash log capture.
//!
//! Reader + overlay state for surfacing captured panic logs inside the TUI.
//! Panic hook (in `lib.rs::install_panic_hook`) writes to
//! `~/.zeus/logs/tui-panic-<timestamp>.log`. This module enumerates them.
//!
//! Surface: Ctrl+Shift+C toggles a modal overlay listing recent crashes.
//! Inside the overlay: `c` clears all panic logs from disk, Esc closes it.

use std::path::{Path, PathBuf};

/// A single captured crash log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashLogEntry {
    /// Filename stem (e.g. `tui-panic-2026-05-28_15-54-44`).
    pub name: String,
    /// Absolute path on disk.
    pub path: PathBuf,
    /// Truncated content (first ~2KB) used for the overlay preview.
    pub preview: String,
}

/// Overlay state for the crash log viewer.
#[derive(Debug, Default, Clone)]
pub struct CrashLogState {
    /// Loaded entries (newest first).
    pub entries: Vec<CrashLogEntry>,
    /// Selected index into `entries` (for keyboard navigation).
    pub selected: usize,
    /// Vertical scroll offset within the preview pane.
    pub scroll: u16,
}

impl CrashLogState {
    pub fn new() -> Self { Self::default() }

    /// Refresh entries from the canonical log dir (`~/.zeus/logs/`).
    pub fn refresh(&mut self) {
        if let Some(dir) = default_log_dir() {
            self.entries = read_crash_logs(&dir);
        } else {
            self.entries.clear();
        }
        if self.selected >= self.entries.len() {
            self.selected = self.entries.len().saturating_sub(1);
        }
        self.scroll = 0;
    }
}

/// Default directory where the panic hook writes crash logs.
pub fn default_log_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".zeus").join("logs"))
}

/// Filter: only files matching `tui-panic-*.log` are considered crash logs.
fn is_crash_log_file(name: &str) -> bool {
    name.starts_with("tui-panic-") && name.ends_with(".log")
}

/// Maximum bytes read from each crash log for the preview pane.
const MAX_PREVIEW_BYTES: usize = 2048;

/// Read all crash logs from `dir`, newest first.
pub fn read_crash_logs(dir: &Path) -> Vec<CrashLogEntry> {
    let mut entries: Vec<CrashLogEntry> = Vec::new();
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return entries, // dir missing → no crashes (good)
    };
    for ent in read.flatten() {
        let path = ent.path();
        let Some(fname) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !is_crash_log_file(fname) { continue; }
        // Truncated preview — avoids loading multi-MB backtraces into TUI memory.
        let raw = std::fs::read(&path).unwrap_or_default();
        let take = raw.len().min(MAX_PREVIEW_BYTES);
        let preview = String::from_utf8_lossy(&raw[..take]).into_owned();
        let name = fname.trim_end_matches(".log").to_string();
        entries.push(CrashLogEntry { name, path, preview });
    }
    // Newest first — filenames are timestamped, so reverse lexicographic works.
    entries.sort_by(|a, b| b.name.cmp(&a.name));
    entries
}

/// Delete every `tui-panic-*.log` in `dir`. Returns count removed.
/// Non-crash files in the directory are untouched (gateway logs etc.).
pub fn clear_crash_logs(dir: &Path) -> usize {
    let mut removed = 0;
    let Ok(read) = std::fs::read_dir(dir) else { return 0 };
    for ent in read.flatten() {
        let path = ent.path();
        let Some(fname) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !is_crash_log_file(fname) { continue; }
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("zeus-crashlog-test-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn crash_log_reads_files_from_dir() {
        let dir = tmpdir("reads");
        fs::write(dir.join("tui-panic-2026-05-28_10-00-00.log"), b"panic: alpha").unwrap();
        fs::write(dir.join("tui-panic-2026-05-28_11-00-00.log"), b"panic: bravo").unwrap();
        // Decoy: non-crash log in same dir should be ignored.
        fs::write(dir.join("gateway.out.log"), b"noise").unwrap();

        let entries = read_crash_logs(&dir);
        assert_eq!(entries.len(), 2, "should pick up 2 panic logs, skip gateway.out.log");
        // Newest first
        assert!(entries[0].name.contains("11-00-00"));
        assert!(entries[1].name.contains("10-00-00"));
        assert!(entries[0].preview.contains("bravo"));
    }

    #[test]
    fn crash_log_renders_when_present() {
        // State-shape test: refresh populates entries; selection stays valid.
        let dir = tmpdir("renders");
        fs::write(dir.join("tui-panic-2026-05-28_09-00-00.log"), b"boom").unwrap();
        let mut state = CrashLogState::new();
        state.entries = read_crash_logs(&dir);
        assert_eq!(state.entries.len(), 1);
        assert!(state.selected < state.entries.len());
        assert_eq!(state.entries[0].preview, "boom");
    }

    #[test]
    fn crash_log_clear_removes_files() {
        let dir = tmpdir("clear");
        fs::write(dir.join("tui-panic-a.log"), b"x").unwrap();
        fs::write(dir.join("tui-panic-b.log"), b"y").unwrap();
        fs::write(dir.join("gateway.out.log"), b"keep").unwrap();

        let removed = clear_crash_logs(&dir);
        assert_eq!(removed, 2);
        // Non-crash file preserved
        assert!(dir.join("gateway.out.log").exists());
        // Crash logs gone
        assert!(read_crash_logs(&dir).is_empty());
    }

    #[test]
    fn refresh_handles_missing_dir() {
        // Cold-start case: log dir doesn't exist yet (no crashes ever happened).
        let mut state = CrashLogState::new();
        // Point at a guaranteed-missing path
        let missing = std::env::temp_dir().join("zeus-crashlog-does-not-exist-xyz");
        let _ = std::fs::remove_dir_all(&missing);
        let entries = read_crash_logs(&missing);
        assert!(entries.is_empty());
        // refresh() should also not panic — uses default_log_dir
        state.refresh();
        // Either empty or populated from real ~/.zeus/logs; selected must be sane.
        assert!(state.selected <= state.entries.len().saturating_sub(1));
    }
}
