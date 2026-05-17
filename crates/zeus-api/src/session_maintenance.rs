//! Session maintenance system for automatic cleanup on save
//!
//! Runs lightweight checks after every session create/save to enforce
//! session hygiene: prune stale sessions, cap total count, and rotate
//! oversized session files.
//!
//! Supports two modes:
//! - **Enforce**: actually deletes/rotates files
//! - **WarnOnly**: logs warnings but takes no destructive action

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, info};

// Re-export config types from zeus-core
pub use zeus_core::{MaintenanceMode, SessionMaintenanceConfig as MaintenanceConfig};

// ============================================================================
// Maintenance Result
// ============================================================================

/// Summary of what maintenance did (or would do in warn-only mode)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MaintenanceResult {
    /// Sessions pruned by age
    pub stale_pruned: usize,
    /// Sessions pruned to enforce count cap
    pub count_pruned: usize,
    /// Session files rotated due to size
    pub files_rotated: usize,
    /// Total bytes freed (0 in warn-only mode)
    pub bytes_freed: u64,
    /// Warnings generated (always populated)
    pub warnings: Vec<String>,
    /// Errors encountered
    pub errors: Vec<String>,
    /// Whether maintenance was in enforce mode
    pub enforced: bool,
}

// ============================================================================
// SessionMaintenance
// ============================================================================

/// Lightweight session maintenance that runs on every session save.
///
/// Unlike the background `SessionPruner` (which runs on a timer), this
/// checks inline during API operations for immediate enforcement.
pub struct SessionMaintenance {
    pub config: MaintenanceConfig,
    pub sessions_dir: PathBuf,
}

impl SessionMaintenance {
    pub fn new(config: MaintenanceConfig, sessions_dir: impl Into<PathBuf>) -> Self {
        Self {
            config,
            sessions_dir: sessions_dir.into(),
        }
    }

    /// Run all maintenance checks. Call after session create/save.
    pub fn run(&self) -> MaintenanceResult {
        if !self.config.enabled {
            return MaintenanceResult::default();
        }

        let enforce = self.config.mode == MaintenanceMode::Enforce;
        let mut result = MaintenanceResult {
            enforced: enforce,
            ..Default::default()
        };

        // Collect session file metadata
        let files = match Self::scan_sessions(&self.sessions_dir) {
            Ok(f) => f,
            Err(e) => {
                result
                    .errors
                    .push(format!("Failed to scan sessions dir: {}", e));
                return result;
            }
        };

        // 1. Prune stale sessions (older than max_age_days)
        self.check_stale(&files, enforce, &mut result);

        // 2. Cap session count (keep newest, remove oldest)
        // Re-scan after potential stale prune
        let files = match Self::scan_sessions(&self.sessions_dir) {
            Ok(f) => f,
            Err(_) => return result,
        };
        self.check_count(&files, enforce, &mut result);

        // 3. Rotate oversized session files
        let files = match Self::scan_sessions(&self.sessions_dir) {
            Ok(f) => f,
            Err(_) => return result,
        };
        self.check_file_sizes(&files, enforce, &mut result);

        if !result.warnings.is_empty() || result.stale_pruned > 0 || result.count_pruned > 0 {
            debug!(
                "Session maintenance: stale={} count={} rotated={} freed={}B mode={}",
                result.stale_pruned,
                result.count_pruned,
                result.files_rotated,
                result.bytes_freed,
                if enforce { "enforce" } else { "warn" }
            );
        }

        result
    }

    /// Scan sessions directory for .jsonl files
    fn scan_sessions(dir: &Path) -> std::io::Result<Vec<SessionFileEntry>> {
        let mut entries = Vec::new();
        if !dir.exists() {
            return Ok(entries);
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                let meta = entry.metadata()?;
                let modified: DateTime<Utc> = meta
                    .modified()
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                    .into();
                let created: DateTime<Utc> = meta
                    .created()
                    .unwrap_or_else(|_| {
                        meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                    })
                    .into();
                let id = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                entries.push(SessionFileEntry {
                    path,
                    id,
                    size_bytes: meta.len(),
                    modified,
                    created,
                });
            }
        }
        // Sort by creation time (oldest first)
        entries.sort_by_key(|e| e.created);
        Ok(entries)
    }

    /// Check for stale sessions older than max_age_days
    fn check_stale(
        &self,
        files: &[SessionFileEntry],
        enforce: bool,
        result: &mut MaintenanceResult,
    ) {
        let cutoff = Utc::now() - Duration::days(self.config.max_age_days as i64);
        for f in files {
            if f.modified < cutoff {
                if enforce {
                    match std::fs::remove_file(&f.path) {
                        Ok(_) => {
                            result.stale_pruned += 1;
                            result.bytes_freed += f.size_bytes;
                            info!(
                                "Session maintenance: pruned stale session {} ({}d old)",
                                f.id,
                                (Utc::now() - f.modified).num_days()
                            );
                        }
                        Err(e) => {
                            result
                                .errors
                                .push(format!("Failed to prune {}: {}", f.id, e));
                        }
                    }
                } else {
                    result.warnings.push(format!(
                        "Stale session {} is {}d old (max {}d)",
                        f.id,
                        (Utc::now() - f.modified).num_days(),
                        self.config.max_age_days
                    ));
                }
            }
        }
    }

    /// Check if session count exceeds max_sessions cap
    fn check_count(
        &self,
        files: &[SessionFileEntry],
        enforce: bool,
        result: &mut MaintenanceResult,
    ) {
        if files.len() <= self.config.max_sessions {
            return;
        }

        let excess = files.len() - self.config.max_sessions;
        // files is sorted oldest-first, so remove from the front
        for f in files.iter().take(excess) {
            if enforce {
                match std::fs::remove_file(&f.path) {
                    Ok(_) => {
                        result.count_pruned += 1;
                        result.bytes_freed += f.size_bytes;
                        info!(
                            "Session maintenance: pruned excess session {} ({})",
                            f.id,
                            humanize_bytes(f.size_bytes)
                        );
                    }
                    Err(e) => {
                        result
                            .errors
                            .push(format!("Failed to prune {}: {}", f.id, e));
                    }
                }
            } else {
                result.warnings.push(format!(
                    "Session count {} exceeds cap of {} — {} would be removed (oldest: {})",
                    files.len(),
                    self.config.max_sessions,
                    excess,
                    f.id
                ));
            }
        }
    }

    /// Check individual session files for exceeding max_file_size_mb
    fn check_file_sizes(
        &self,
        files: &[SessionFileEntry],
        enforce: bool,
        result: &mut MaintenanceResult,
    ) {
        let max_bytes = self.config.max_file_size_mb * 1024 * 1024;
        for f in files {
            if f.size_bytes > max_bytes {
                if enforce {
                    match self.rotate_session_file(f) {
                        Ok(freed) => {
                            result.files_rotated += 1;
                            result.bytes_freed += freed;
                            info!(
                                "Session maintenance: rotated oversized session {} ({} -> truncated)",
                                f.id,
                                humanize_bytes(f.size_bytes)
                            );
                        }
                        Err(e) => {
                            result
                                .errors
                                .push(format!("Failed to rotate {}: {}", f.id, e));
                        }
                    }
                } else {
                    result.warnings.push(format!(
                        "Session {} is {} (max {}MB)",
                        f.id,
                        humanize_bytes(f.size_bytes),
                        self.config.max_file_size_mb
                    ));
                }
            }
        }
    }

    /// Rotate an oversized session file:
    /// 1. Rename current file to {id}.rotated.jsonl
    /// 2. Create new file keeping only the session_start entry and last 50 messages
    fn rotate_session_file(&self, file: &SessionFileEntry) -> std::io::Result<u64> {
        let content = std::fs::read_to_string(&file.path)?;
        let lines: Vec<&str> = content.lines().collect();

        // Keep the first line (session_start) and last 50 message lines
        let keep_tail = 50;
        let mut kept_lines = Vec::new();
        if let Some(first) = lines.first() {
            kept_lines.push(*first);
        }
        let start = if lines.len() > keep_tail + 1 {
            lines.len() - keep_tail
        } else {
            1 // skip only the first line we already added
        };
        for line in &lines[start..] {
            kept_lines.push(line);
        }

        // Write rotated archive
        let rotated_path = file.path.with_extension("rotated.jsonl");
        std::fs::rename(&file.path, &rotated_path)?;

        // Write trimmed version
        let new_content = kept_lines.join("\n") + "\n";
        std::fs::write(&file.path, &new_content)?;

        let new_size = new_content.len() as u64;
        let freed = file.size_bytes.saturating_sub(new_size);
        Ok(freed)
    }
}

/// Internal file entry for maintenance scanning
#[derive(Debug, Clone)]
struct SessionFileEntry {
    path: PathBuf,
    id: String,
    size_bytes: u64,
    modified: DateTime<Utc>,
    created: DateTime<Utc>,
}

/// Format bytes as human-readable string
fn humanize_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use zeus_core::MaintenanceMode;

    fn test_dir() -> tempfile::TempDir {
        tempfile::TempDir::new().unwrap()
    }

    fn write_session(dir: &Path, id: &str, content: &str) {
        fs::write(dir.join(format!("{}.jsonl", id)), content).unwrap();
    }

    fn session_start_line(id: &str) -> String {
        format!(
            r#"{{"type":"session_start","id":"{}","created":"2025-01-01T00:00:00Z"}}"#,
            id
        )
    }

    fn message_line(content: &str) -> String {
        format!(
            r#"{{"type":"message","role":"user","content":"{}"}}"#,
            content
        )
    }

    #[test]
    fn test_maintenance_disabled() {
        let tmp = test_dir();
        write_session(tmp.path(), "s1", "line1\nline2\n");

        let config = MaintenanceConfig {
            enabled: false,
            ..Default::default()
        };
        let m = SessionMaintenance::new(config, tmp.path());
        let result = m.run();
        assert_eq!(result.stale_pruned, 0);
        assert_eq!(result.count_pruned, 0);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_prune_stale_enforce() {
        let tmp = test_dir();
        let path = tmp.path().join("old.jsonl");
        fs::write(&path, "line\n").unwrap();
        // Set modified time to 60 days ago
        let old_time =
            std::time::SystemTime::now() - std::time::Duration::from_secs(60 * 24 * 3600);
        filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(old_time)).unwrap();

        let config = MaintenanceConfig {
            enabled: true,
            mode: MaintenanceMode::Enforce,
            max_age_days: 30,
            max_sessions: 500,
            max_file_size_mb: 10,
        };
        let m = SessionMaintenance::new(config, tmp.path());
        let result = m.run();
        assert_eq!(result.stale_pruned, 1);
        assert!(result.enforced);
        assert!(!path.exists());
    }

    #[test]
    fn test_prune_stale_warn_only() {
        let tmp = test_dir();
        let path = tmp.path().join("old.jsonl");
        fs::write(&path, "line\n").unwrap();
        let old_time =
            std::time::SystemTime::now() - std::time::Duration::from_secs(60 * 24 * 3600);
        filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(old_time)).unwrap();

        let config = MaintenanceConfig {
            enabled: true,
            mode: MaintenanceMode::WarnOnly,
            max_age_days: 30,
            max_sessions: 500,
            max_file_size_mb: 10,
        };
        let m = SessionMaintenance::new(config, tmp.path());
        let result = m.run();
        assert_eq!(result.stale_pruned, 0);
        assert!(!result.enforced);
        assert!(!result.warnings.is_empty());
        assert!(path.exists()); // Not deleted
    }

    #[test]
    fn test_prune_count_cap() {
        let tmp = test_dir();
        // Create 5 sessions
        for i in 0..5 {
            let content = format!(
                "{}\n{}\n",
                session_start_line(&format!("s{}", i)),
                message_line("hi")
            );
            write_session(tmp.path(), &format!("s{}", i), &content);
        }

        let config = MaintenanceConfig {
            enabled: true,
            mode: MaintenanceMode::Enforce,
            max_age_days: 365,
            max_sessions: 3,
            max_file_size_mb: 100,
        };
        let m = SessionMaintenance::new(config, tmp.path());
        let result = m.run();
        assert_eq!(result.count_pruned, 2);
        // 3 should remain
        let remaining = fs::read_dir(tmp.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .ok()
                    .and_then(|e| e.path().extension().map(|ext| ext == "jsonl"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(remaining, 3);
    }

    #[test]
    fn test_rotate_oversized() {
        let tmp = test_dir();
        // Create a session with lots of content
        let mut content = session_start_line("big");
        content.push('\n');
        for i in 0..200 {
            content.push_str(&message_line(&format!("message number {}", i)));
            content.push('\n');
        }
        write_session(tmp.path(), "big", &content);

        let config = MaintenanceConfig {
            enabled: true,
            mode: MaintenanceMode::Enforce,
            max_age_days: 365,
            max_sessions: 500,
            // Use a tiny limit so our test file exceeds it
            max_file_size_mb: 0,
        };
        let m = SessionMaintenance::new(config, tmp.path());
        let result = m.run();
        assert_eq!(result.files_rotated, 1);
        // Rotated file should exist
        assert!(tmp.path().join("big.rotated.jsonl").exists());
        // Original file should be smaller
        let new_size = fs::metadata(tmp.path().join("big.jsonl")).unwrap().len();
        assert!(new_size < content.len() as u64);
    }

    #[test]
    fn test_default_config() {
        let config = MaintenanceConfig::default();
        assert!(config.enabled);
        assert_eq!(config.mode, MaintenanceMode::Enforce);
        assert_eq!(config.max_age_days, 30);
        assert_eq!(config.max_sessions, 500);
        assert_eq!(config.max_file_size_mb, 10);
    }

    #[test]
    fn test_empty_directory() {
        let tmp = test_dir();
        let config = MaintenanceConfig::default();
        let m = SessionMaintenance::new(config, tmp.path());
        let result = m.run();
        assert_eq!(result.stale_pruned, 0);
        assert_eq!(result.count_pruned, 0);
        assert_eq!(result.files_rotated, 0);
    }

    #[test]
    fn test_nonexistent_directory() {
        let config = MaintenanceConfig::default();
        let m = SessionMaintenance::new(config, "/nonexistent/path/sessions");
        let result = m.run();
        assert_eq!(result.stale_pruned, 0);
    }

    #[test]
    fn test_humanize_bytes() {
        assert_eq!(humanize_bytes(500), "500B");
        assert_eq!(humanize_bytes(1500), "1.5KB");
        assert_eq!(humanize_bytes(5 * 1024 * 1024), "5.0MB");
    }
}
