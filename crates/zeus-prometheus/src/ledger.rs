//! LedgerStore - Append-only event ledger for the event-driven heartbeat (R2)
//!
//! Replaces the implicit state tracking that the 5-minute cron relied on.
//! Every wake/skip/run is recorded as an immutable JSONL entry, enabling:
//!
//! - Suppression rules (don't re-run X within Y seconds)
//! - Replay/audit ("why did the heartbeat skip at 14:32?")
//! - Cross-session continuity (read last entries on startup)
//!
//! # Format
//!
//! One JSON object per line, in `{workspace}/heartbeat-ledger.jsonl`:
//!
//! ```jsonl
//! {"ts":1714229862,"kind":"wake","reason":"goal_added","agent":"zeus106"}
//! {"ts":1714229863,"kind":"run","task":"hourly","duration_ms":1240}
//! {"ts":1714229870,"kind":"skip","reason":"suppressed:cooldown","task":"hourly"}
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use zeus_core::Result;

/// A single ledger event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// Unix timestamp (seconds).
    pub ts: u64,
    /// Event kind: "wake", "run", "skip", "error".
    pub kind: String,
    /// Free-form reason / source ("goal_added", "tool_finished", "cooldown", ...).
    pub reason: String,
    /// Optional task identifier (e.g. heartbeat task name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Optional agent identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Optional duration in milliseconds (for "run" events).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl LedgerEntry {
    pub fn now(kind: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            ts: chrono::Utc::now().timestamp() as u64,
            kind: kind.into(),
            reason: reason.into(),
            task: None,
            agent: None,
            duration_ms: None,
        }
    }

    pub fn with_task(mut self, task: impl Into<String>) -> Self {
        self.task = Some(task.into());
        self
    }

    pub fn with_agent(mut self, agent: impl Into<String>) -> Self {
        self.agent = Some(agent.into());
        self
    }

    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }
}

/// Append-only event store. Implementations: JSONL (default), in-memory (tests).
#[async_trait]
pub trait LedgerStore: Send + Sync {
    /// Append a single entry.
    async fn append(&self, entry: LedgerEntry) -> Result<()>;

    /// Read the most recent `n` entries (newest first).
    async fn tail(&self, n: usize) -> Result<Vec<LedgerEntry>>;

    /// Find the most recent entry matching `kind` and (optionally) `task`.
    async fn last_of(&self, kind: &str, task: Option<&str>) -> Result<Option<LedgerEntry>> {
        let entries = self.tail(256).await?;
        Ok(entries
            .into_iter()
            .find(|e| e.kind == kind && (task.is_none() || e.task.as_deref() == task)))
    }
}

// ---------------------------------------------------------------------------
// JSONL implementation
// ---------------------------------------------------------------------------

/// Append-only JSONL ledger backed by a single file.
///
/// Writes are serialized through an async mutex so multiple wake events
/// can't interleave partial lines.
pub struct JsonlLedger {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl JsonlLedger {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            write_lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

#[async_trait]
impl LedgerStore for JsonlLedger {
    async fn append(&self, entry: LedgerEntry) -> Result<()> {
        let line = serde_json::to_string(&entry)
            .map_err(|e| zeus_core::Error::memory(format!("ledger serialize: {e}")))?;
        let _g = self.write_lock.lock().await;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| zeus_core::Error::memory(format!("ledger mkdir: {e}")))?;
        }
        let mut f = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .map_err(|e| zeus_core::Error::memory(format!("ledger open: {e}")))?;
        f.write_all(line.as_bytes())
            .await
            .map_err(|e| zeus_core::Error::memory(format!("ledger write: {e}")))?;
        f.write_all(b"\n")
            .await
            .map_err(|e| zeus_core::Error::memory(format!("ledger write nl: {e}")))?;
        Ok(())
    }

    async fn tail(&self, n: usize) -> Result<Vec<LedgerEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let bytes = tokio::fs::read(&self.path)
            .await
            .map_err(|e| zeus_core::Error::memory(format!("ledger read: {e}")))?;
        let text = String::from_utf8_lossy(&bytes);
        let mut entries: Vec<LedgerEntry> = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(e) = serde_json::from_str::<LedgerEntry>(line) {
                entries.push(e);
            }
            // Silently skip malformed lines — the ledger is best-effort.
        }
        // Newest first
        entries.reverse();
        entries.truncate(n);
        Ok(entries)
    }
}

// ---------------------------------------------------------------------------
// In-memory implementation (for tests / suppression-only callers)
// ---------------------------------------------------------------------------

/// In-memory ring-style store. Useful for tests and ephemeral subsystems.
pub struct MemoryLedger {
    inner: Mutex<Vec<LedgerEntry>>,
    cap: usize,
}

impl MemoryLedger {
    pub fn new(cap: usize) -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
            cap,
        }
    }
}

impl Default for MemoryLedger {
    fn default() -> Self {
        Self::new(1024)
    }
}

#[async_trait]
impl LedgerStore for MemoryLedger {
    async fn append(&self, entry: LedgerEntry) -> Result<()> {
        let mut g = self.inner.lock().await;
        g.push(entry);
        let len = g.len();
        if len > self.cap {
            g.drain(0..(len - self.cap));
        }
        Ok(())
    }

    async fn tail(&self, n: usize) -> Result<Vec<LedgerEntry>> {
        let g = self.inner.lock().await;
        let mut out: Vec<LedgerEntry> = g.iter().rev().take(n).cloned().collect();
        // Newest first already (we reversed). Keep as-is.
        // (collect-with-rev preserves newest-first ordering.)
        out.shrink_to_fit();
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Suppression rules
// ---------------------------------------------------------------------------

/// Suppression policy — decides whether a wake should be acted on or dropped.
///
/// Reads recent ledger history to enforce:
/// - **cooldown**: don't re-run the same task more often than `min_interval_secs`
/// - **dedup**: identical (kind, reason, task) within `dedup_window_secs` is dropped
#[derive(Debug, Clone)]
pub struct SuppressionPolicy {
    pub min_interval_secs: u64,
    pub dedup_window_secs: u64,
}

impl Default for SuppressionPolicy {
    fn default() -> Self {
        Self {
            min_interval_secs: 30,    // hard floor between same-task runs
            dedup_window_secs: 5,     // collapse rapid-fire identical wakes
        }
    }
}

impl SuppressionPolicy {
    /// Returns `Some(reason)` if the proposed entry should be suppressed.
    pub async fn evaluate(
        &self,
        ledger: &Arc<dyn LedgerStore>,
        proposed: &LedgerEntry,
    ) -> Option<String> {
        let now = proposed.ts;
        let recent = ledger.tail(64).await.ok()?;

        // Cooldown: last "run" of the same task within min_interval_secs?
        if proposed.kind == "run" {
            if let Some(last_run) = recent.iter().find(|e| {
                e.kind == "run" && e.task == proposed.task
            }) {
                let delta = now.saturating_sub(last_run.ts);
                if delta < self.min_interval_secs {
                    return Some(format!(
                        "cooldown:{}s<{}s",
                        delta, self.min_interval_secs
                    ));
                }
            }
        }

        // Dedup: identical (kind, reason, task) within dedup_window_secs?
        if let Some(dup) = recent.iter().find(|e| {
            e.kind == proposed.kind
                && e.reason == proposed.reason
                && e.task == proposed.task
        }) {
            let delta = now.saturating_sub(dup.ts);
            if delta < self.dedup_window_secs {
                return Some(format!("dedup:{}s<{}s", delta, self.dedup_window_secs));
            }
        }

        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn jsonl_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        let store = JsonlLedger::new(path);

        store
            .append(LedgerEntry::now("wake", "goal_added").with_agent("zeus106"))
            .await
            .unwrap();
        store
            .append(LedgerEntry::now("run", "hourly").with_task("hourly").with_duration(123))
            .await
            .unwrap();

        let tail = store.tail(10).await.unwrap();
        assert_eq!(tail.len(), 2);
        // Newest first
        assert_eq!(tail[0].kind, "run");
        assert_eq!(tail[1].kind, "wake");
    }

    #[tokio::test]
    async fn memory_ledger_caps() {
        let store = MemoryLedger::new(3);
        for i in 0..10 {
            store
                .append(LedgerEntry::now("wake", format!("r{i}")))
                .await
                .unwrap();
        }
        let tail = store.tail(100).await.unwrap();
        assert_eq!(tail.len(), 3);
        // Newest first: r9, r8, r7
        assert_eq!(tail[0].reason, "r9");
        assert_eq!(tail[2].reason, "r7");
    }

    #[tokio::test]
    async fn suppression_cooldown_blocks_rapid_runs() {
        let store: Arc<dyn LedgerStore> = Arc::new(MemoryLedger::default());
        let policy = SuppressionPolicy {
            min_interval_secs: 60,
            dedup_window_secs: 5,
        };

        // First run goes through.
        let first = LedgerEntry::now("run", "tick").with_task("hourly");
        assert!(policy.evaluate(&store, &first).await.is_none());
        store.append(first).await.unwrap();

        // Immediate second run is blocked by cooldown.
        let second = LedgerEntry::now("run", "tick").with_task("hourly");
        let reason = policy.evaluate(&store, &second).await;
        assert!(reason.is_some());
        assert!(reason.unwrap().starts_with("cooldown"));
    }

    #[tokio::test]
    async fn suppression_dedup_collapses_identical_wakes() {
        let store: Arc<dyn LedgerStore> = Arc::new(MemoryLedger::default());
        let policy = SuppressionPolicy::default();

        let a = LedgerEntry::now("wake", "tool_finished").with_agent("zeus106");
        assert!(policy.evaluate(&store, &a).await.is_none());
        store.append(a).await.unwrap();

        let b = LedgerEntry::now("wake", "tool_finished").with_agent("zeus106");
        let reason = policy.evaluate(&store, &b).await;
        assert!(reason.is_some());
        assert!(reason.unwrap().starts_with("dedup"));
    }
}
