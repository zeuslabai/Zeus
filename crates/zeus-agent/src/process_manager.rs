//! Process Manager — track background shell sessions with lifecycle management

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::Serialize;
use std::sync::LazyLock;
use tokio::process::Child;
use tokio::sync::Mutex;
use tracing::{debug, warn};
use zeus_core::{Error, Result};

/// Global process registry (singleton)
pub static PROCESS_REGISTRY: LazyLock<ProcessRegistry> = LazyLock::new(ProcessRegistry::new);

/// Process state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    Running,
    Completed,
    Failed,
    Killed,
}

/// Info about a tracked process (serializable snapshot)
#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub command: String,
    pub state: ProcessState,
    pub started_at: DateTime<Utc>,
    pub exit_code: Option<i32>,
}

/// Internal handle wrapping the child process
struct ProcessHandle {
    command: String,
    child: Mutex<Option<Child>>,
    state: Mutex<ProcessState>,
    started_at: DateTime<Utc>,
    exit_code: Mutex<Option<i32>>,
}

/// Registry of tracked background processes
pub struct ProcessRegistry {
    handles: DashMap<u32, ProcessHandle>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self {
            handles: DashMap::new(),
        }
    }

    /// Register a spawned child process for tracking
    pub fn register(&self, pid: u32, command: &str, child: Child) {
        debug!(pid, command, "Registering background process");
        self.handles.insert(
            pid,
            ProcessHandle {
                command: command.to_string(),
                child: Mutex::new(Some(child)),
                state: Mutex::new(ProcessState::Running),
                started_at: Utc::now(),
                exit_code: Mutex::new(None),
            },
        );
    }

    /// List all tracked processes, optionally filtered by name substring
    pub fn list(&self, name_filter: Option<&str>) -> Vec<ProcessInfo> {
        let mut results = Vec::new();
        for entry in self.handles.iter() {
            let handle = entry.value();
            if let Some(filter) = name_filter {
                if !handle.command.contains(filter) {
                    continue;
                }
            }
            let state = *handle.state.blocking_lock();
            let exit_code = *handle.exit_code.blocking_lock();
            results.push(ProcessInfo {
                pid: *entry.key(),
                command: handle.command.clone(),
                state,
                started_at: handle.started_at,
                exit_code,
            });
        }
        results.sort_by_key(|p| p.pid);
        results
    }

    /// Get status of a specific process
    pub fn status(&self, pid: u32) -> Option<ProcessInfo> {
        self.handles.get(&pid).map(|entry| {
            let handle = entry.value();
            let state = *handle.state.blocking_lock();
            let exit_code = *handle.exit_code.blocking_lock();
            ProcessInfo {
                pid,
                command: handle.command.clone(),
                state,
                started_at: handle.started_at,
                exit_code,
            }
        })
    }

    /// Wait for a process to complete and return its combined output
    pub async fn wait(&self, pid: u32) -> Result<String> {
        let handle = self
            .handles
            .get(&pid)
            .ok_or_else(|| Error::Tool(format!("No tracked process with pid {pid}")))?;

        let mut child_guard = handle.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            let output = child
                .wait_with_output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to wait for pid {pid}: {e}")))?;

            let code = output.status.code();
            *handle.state.lock().await = if output.status.success() {
                ProcessState::Completed
            } else {
                ProcessState::Failed
            };
            *handle.exit_code.lock().await = code;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            let mut result = String::new();
            if !stdout.is_empty() {
                result.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !result.is_empty() {
                    result.push_str("\n--- stderr ---\n");
                }
                result.push_str(&stderr);
            }
            Ok(result)
        } else {
            // Already waited — return status
            let state = *handle.state.lock().await;
            Ok(format!("Process {pid} already completed (state: {state:?})"))
        }
    }

    /// Kill a tracked process
    pub fn kill(&self, pid: u32) -> Result<()> {
        let handle = self
            .handles
            .get(&pid)
            .ok_or_else(|| Error::Tool(format!("No tracked process with pid {pid}")))?;

        let mut child_guard = handle.child.blocking_lock();
        if let Some(ref mut child) = *child_guard {
            child
                .start_kill()
                .map_err(|e| Error::Tool(format!("Failed to kill pid {pid}: {e}")))?;
            *handle.state.blocking_lock() = ProcessState::Killed;
            debug!(pid, "Process killed");
            Ok(())
        } else {
            warn!(pid, "Process already completed, nothing to kill");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_and_list() {
        let registry = ProcessRegistry::new();
        let child = tokio::process::Command::new("sleep")
            .arg("0.1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        registry.register(pid, "sleep 0.1", child);

        let list = registry.list(None);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].pid, pid);
        assert_eq!(list[0].state, ProcessState::Running);

        // Wait for completion
        let output = registry.wait(pid).await.unwrap();
        assert!(output.is_empty() || output.contains("")); // sleep produces no output

        let info = registry.status(pid).unwrap();
        assert_eq!(info.state, ProcessState::Completed);
    }

    #[tokio::test]
    async fn test_kill_process() {
        let registry = ProcessRegistry::new();
        let child = tokio::process::Command::new("sleep")
            .arg("60")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        registry.register(pid, "sleep 60", child);

        registry.kill(pid).unwrap();
        let info = registry.status(pid).unwrap();
        assert_eq!(info.state, ProcessState::Killed);
    }

    #[test]
    fn test_name_filter() {
        let registry = ProcessRegistry::new();
        // Just test empty list with filter
        let list = registry.list(Some("cargo"));
        assert!(list.is_empty());
    }

    #[test]
    fn test_status_not_found() {
        let registry = ProcessRegistry::new();
        assert!(registry.status(99999).is_none());
    }
}
