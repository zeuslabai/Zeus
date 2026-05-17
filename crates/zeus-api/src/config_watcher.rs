//! Config hot-reload: file watcher + change history
//!
//! Watches `~/.zeus/config.toml` for changes and reloads the running
//! AppState atomically. Maintains a ring buffer of recent config changes.

use chrono::{DateTime, Utc};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::path::PathBuf;
use tracing::{debug, error, info, warn};
use zeus_core::Config;

use crate::SharedState;

/// Maximum number of config change entries to keep in history.
const MAX_HISTORY: usize = 10;

/// Debounce interval for file system events.
const DEBOUNCE_MS: u64 = 500;

/// A record of a single config reload event.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigChangeEntry {
    pub timestamp: DateTime<Utc>,
    pub source: String,
    pub changed_keys: Vec<String>,
}

/// Ring buffer of recent config changes, shared across handlers.
#[derive(Debug, Default)]
pub struct ConfigHistory {
    entries: VecDeque<ConfigChangeEntry>,
}

impl ConfigHistory {
    pub fn push(&mut self, entry: ConfigChangeEntry) {
        if self.entries.len() >= MAX_HISTORY {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn entries(&self) -> &VecDeque<ConfigChangeEntry> {
        &self.entries
    }
}

/// Reload config from disk and swap it into the running AppState.
///
/// Returns the list of top-level keys that changed, or an error.
pub async fn reload_config(state: &SharedState, source: &str) -> Result<Vec<String>, String> {
    let new_config =
        Config::load().map_err(|e| format!("Failed to load config from disk: {}", e))?;

    let mut state_guard = state.write().await;

    let old_val = serde_json::to_value(&state_guard.config).unwrap_or_default();
    let new_val = serde_json::to_value(&new_config).unwrap_or_default();

    let changed = diff_top_level_keys(&old_val, &new_val);

    if changed.is_empty() {
        debug!("Config reload: no changes detected");
        return Ok(changed);
    }

    info!(
        source = source,
        changed_keys = ?changed,
        "Config reloaded"
    );

    // Swap config
    state_guard.config = new_config;

    // Record in history
    state_guard.config_history.push(ConfigChangeEntry {
        timestamp: Utc::now(),
        source: source.to_string(),
        changed_keys: changed.clone(),
    });

    Ok(changed)
}

/// Compare two JSON objects and return the list of top-level keys that differ.
fn diff_top_level_keys(old: &Value, new: &Value) -> Vec<String> {
    let mut changed = Vec::new();

    let old_obj = match old.as_object() {
        Some(o) => o,
        None => return vec!["(root)".to_string()],
    };
    let new_obj = match new.as_object() {
        Some(o) => o,
        None => return vec!["(root)".to_string()],
    };

    // Check keys in old
    for (key, old_val) in old_obj {
        match new_obj.get(key) {
            Some(new_val) if old_val != new_val => {
                changed.push(key.clone());
            }
            None => {
                changed.push(key.clone());
            }
            _ => {}
        }
    }

    // Check keys only in new
    for key in new_obj.keys() {
        if !old_obj.contains_key(key) {
            changed.push(key.clone());
        }
    }

    changed.sort();
    changed
}

/// Start a background file watcher on `~/.zeus/config.toml`.
///
/// Returns the watcher handle (must be kept alive) and a join handle for the
/// debounce task. Dropping the watcher stops watching.
pub fn start_config_watcher(
    state: SharedState,
) -> Result<(RecommendedWatcher, tokio::task::JoinHandle<()>), String> {
    let config_path = config_file_path();

    let config_dir = config_path
        .parent()
        .ok_or_else(|| "Cannot determine config directory".to_string())?
        .to_path_buf();

    // Channel for debouncing filesystem events
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(16);

    let watched_path = config_path.clone();
    let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if let Ok(event) = res {
            let dominated = matches!(
                event.kind,
                notify::EventKind::Modify(_) | notify::EventKind::Create(_)
            );
            if dominated && event.paths.contains(&watched_path) {
                // Use try_send to avoid panics on FreeBSD kqueue
                // (blocking_send panics if tokio runtime isn't available on this thread)
                let _ = tx.try_send(());
            }
        }
    })
    .map_err(|e| format!("Failed to create config watcher: {}", e))?;

    // Watch the parent directory (some editors write to temp then rename)
    watcher
        .watch(&config_dir, RecursiveMode::NonRecursive)
        .map_err(|e| format!("Failed to watch {}: {}", config_dir.display(), e))?;

    info!(path = %config_path.display(), "Config file watcher started");

    // Debounce + reload task
    let handle = tokio::spawn(async move {
        loop {
            // Wait for first event
            if rx.recv().await.is_none() {
                break; // channel closed
            }
            // Debounce: wait a bit, drain any queued events
            tokio::time::sleep(std::time::Duration::from_millis(DEBOUNCE_MS)).await;
            while rx.try_recv().is_ok() {}

            // Reload
            match reload_config(&state, "file_watcher").await {
                Ok(changed) if changed.is_empty() => {
                    debug!("Config file changed but no effective differences");
                }
                Ok(changed) => {
                    info!(changed = ?changed, "Config auto-reloaded from file watcher");
                }
                Err(e) => {
                    error!(error = %e, "Failed to auto-reload config");
                }
            }
        }
        warn!("Config watcher channel closed, stopping");
    });

    Ok((watcher, handle))
}

/// Path to the Zeus config file.
fn config_file_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zeus")
        .join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_top_level_keys_no_change() {
        let a = serde_json::json!({"model": "a", "workspace": "/tmp"});
        let b = serde_json::json!({"model": "a", "workspace": "/tmp"});
        assert!(diff_top_level_keys(&a, &b).is_empty());
    }

    #[test]
    fn test_diff_top_level_keys_changed() {
        let a = serde_json::json!({"model": "a", "workspace": "/tmp"});
        let b = serde_json::json!({"model": "b", "workspace": "/tmp"});
        let diff = diff_top_level_keys(&a, &b);
        assert_eq!(diff, vec!["model"]);
    }

    #[test]
    fn test_diff_top_level_keys_added() {
        let a = serde_json::json!({"model": "a"});
        let b = serde_json::json!({"model": "a", "workspace": "/tmp"});
        let diff = diff_top_level_keys(&a, &b);
        assert_eq!(diff, vec!["workspace"]);
    }

    #[test]
    fn test_diff_top_level_keys_removed() {
        let a = serde_json::json!({"model": "a", "workspace": "/tmp"});
        let b = serde_json::json!({"model": "a"});
        let diff = diff_top_level_keys(&a, &b);
        assert_eq!(diff, vec!["workspace"]);
    }

    #[test]
    fn test_diff_top_level_keys_multiple() {
        let a = serde_json::json!({"model": "a", "workspace": "/tmp", "max_iterations": 10});
        let b = serde_json::json!({"model": "b", "workspace": "/var", "max_iterations": 10});
        let diff = diff_top_level_keys(&a, &b);
        assert_eq!(diff, vec!["model", "workspace"]);
    }

    #[test]
    fn test_config_history_ring_buffer() {
        let mut history = ConfigHistory::default();
        for i in 0..15 {
            history.push(ConfigChangeEntry {
                timestamp: Utc::now(),
                source: format!("test_{}", i),
                changed_keys: vec!["model".to_string()],
            });
        }
        assert_eq!(history.entries().len(), MAX_HISTORY);
        // Oldest should be test_5 (0..4 were evicted)
        assert_eq!(history.entries()[0].source, "test_5");
    }

    #[test]
    fn test_config_file_path() {
        let path = config_file_path();
        assert!(path.ends_with("config.toml"));
        assert!(path.to_string_lossy().contains(".zeus"));
    }
}
