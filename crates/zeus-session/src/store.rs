//! Thread-safe session store with per-session locking.
//!
//! Provides `SessionStore` which serializes access to individual sessions,
//! preventing race conditions when multiple WebSocket clients or API handlers
//! access the same session simultaneously.
//!
//! ## Design
//!
//! Uses a two-level locking scheme:
//! - Outer `RwLock<HashMap>` for the session lock registry (held briefly)
//! - Inner per-session `Mutex` for exclusive session access (held during operations)
//!
//! Different sessions can be accessed concurrently. Only concurrent access
//! to the *same* session is serialized.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};

use crate::Session;
use zeus_core::Result;

/// A guard that holds exclusive access to a session.
///
/// Can be moved across `.await` points (owns the lock).
/// Drop the guard when done with the session.
pub struct SessionGuard {
    _guard: OwnedMutexGuard<()>,
    session_id: String,
}

impl SessionGuard {
    /// The session ID this guard protects.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

/// Thread-safe session store that serializes per-session access.
///
/// Each session ID gets its own `Mutex`. Multiple different sessions can be
/// accessed concurrently, but concurrent access to the *same* session is
/// serialized to prevent JSONL file corruption and stale in-memory state.
///
/// # Usage
///
/// ```ignore
/// let store = SessionStore::new("/path/to/sessions");
///
/// // Acquire exclusive access before operating on a session
/// let guard = store.acquire("session-123").await;
/// let session = store.load("session-123").await?;
/// // ... mutate session ...
/// drop(guard); // release lock
/// ```
pub struct SessionStore {
    sessions_dir: PathBuf,
    /// Per-session mutexes keyed by session ID.
    locks: RwLock<HashMap<String, Arc<Mutex<()>>>>,
}

impl SessionStore {
    /// Create a new session store for the given sessions directory.
    pub fn new(sessions_dir: impl AsRef<Path>) -> Self {
        Self {
            sessions_dir: sessions_dir.as_ref().to_path_buf(),
            locks: RwLock::new(HashMap::new()),
        }
    }

    /// Acquire exclusive access to a session by ID.
    ///
    /// Returns a `SessionGuard` that holds the lock. The guard can be moved
    /// across `.await` points. Drop it when done with the session.
    ///
    /// If another task holds the lock for this session, this call will wait
    /// until the lock is released. Different sessions are not blocked.
    pub async fn acquire(&self, session_id: &str) -> SessionGuard {
        let mutex = {
            // Fast path: check if lock already exists (read lock on map)
            let locks = self.locks.read().await;
            if let Some(m) = locks.get(session_id) {
                m.clone()
            } else {
                drop(locks);
                // Slow path: create lock (write lock on map)
                let mut locks = self.locks.write().await;
                locks
                    .entry(session_id.to_string())
                    .or_insert_with(|| Arc::new(Mutex::new(())))
                    .clone()
            }
        };
        let guard = mutex.lock_owned().await;
        SessionGuard {
            _guard: guard,
            session_id: session_id.to_string(),
        }
    }

    /// Load a session from disk.
    ///
    /// Caller should hold the session lock via [`acquire()`] first for
    /// mutation safety. Read-only access can skip locking.
    pub async fn load(&self, id: &str) -> Result<Session> {
        Session::load(&self.sessions_dir, id).await
    }

    /// Create a new session and initialize its JSONL file.
    ///
    /// The new session is automatically registered in the lock table.
    pub async fn create(&self) -> Result<Session> {
        let session = Session::new(&self.sessions_dir);
        session.init().await?;
        // Pre-register the lock for this new session
        let mut locks = self.locks.write().await;
        locks.insert(session.id.clone(), Arc::new(Mutex::new(())));
        Ok(session)
    }

    /// List all sessions from disk (sorted newest first).
    pub async fn list(&self) -> Result<Vec<(String, chrono::DateTime<chrono::Utc>)>> {
        Session::list(&self.sessions_dir).await
    }

    /// Evict a session's lock from the store.
    ///
    /// Use after deleting a session to free memory.
    pub async fn evict(&self, session_id: &str) {
        let mut locks = self.locks.write().await;
        locks.remove(session_id);
    }

    /// Number of tracked session locks.
    pub async fn active_count(&self) -> usize {
        self.locks.read().await.len()
    }

    /// Sessions directory path.
    pub fn sessions_dir(&self) -> &Path {
        &self.sessions_dir
    }

    /// Get or create a labeled persistent session (S53-T8b).
    ///
    /// The label is the agent's persistent identity. Calling
    /// `get_or_create_labeled("pr-monitor")` always returns the same
    /// session, surviving gateway restarts.
    pub async fn get_or_create_labeled(&self, label: &str) -> Result<Session> {
        let session = Session::get_or_create_labeled(&self.sessions_dir, label).await;
        // Initialize the session file if it's new (no messages yet)
        if session.messages.is_empty() {
            session.init().await?;
        }
        // Register the lock
        let stable_id = format!("agent-{}", label);
        let mut locks = self.locks.write().await;
        locks
            .entry(stable_id)
            .or_insert_with(|| Arc::new(Mutex::new(())));
        Ok(session)
    }

    /// List all labeled (agent) sessions.
    ///
    /// Returns `(label, session_id, created)` for each session file
    /// with the `agent-` prefix.
    pub async fn list_labeled(
        &self,
    ) -> Result<Vec<(String, String, chrono::DateTime<chrono::Utc>)>> {
        let all = self.list().await?;
        Ok(all
            .into_iter()
            .filter_map(|(id, created)| {
                id.strip_prefix("agent-")
                    .map(|label| (label.to_string(), id.clone(), created))
            })
            .collect())
    }

    /// Load the last `n` messages from all sessions EXCEPT `except_session_id`,
    /// interleaved in chronological order by their position in each session file.
    ///
    /// Used for cross-channel session-tail injection (#91 Option B).
    /// Returns an empty Vec if no other sessions exist or `n == 0`.
    pub async fn load_cross_channel_tail(
        &self,
        except_session_id: &str,
        n: usize,
    ) -> Vec<zeus_core::Message> {
        if n == 0 {
            return Vec::new();
        }
        // List all sessions
        let all = match self.list().await {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        // Filter out current session
        let other_ids: Vec<String> = all
            .into_iter()
            .filter(|(id, _)| id != except_session_id)
            .map(|(id, _)| id)
            .collect();
        if other_ids.is_empty() {
            return Vec::new();
        }
        // Load each session and collect messages with (session_index, msg_index) ordering
        // We use file-ordering (position in JSONL) as the chronological proxy.
        let mut all_messages: Vec<(usize, usize, zeus_core::Message)> = Vec::new();
        for (sess_idx, id) in other_ids.iter().enumerate() {
            if let Ok(session) = self.load(id).await {
                for (msg_idx, msg) in session.messages.into_iter().enumerate() {
                    all_messages.push((sess_idx, msg_idx, msg));
                }
            }
        }
        if all_messages.is_empty() {
            return Vec::new();
        }
        // Sort by (msg_index, sess_index) for chronological interleave
        all_messages.sort_by_key(|(sess_idx, msg_idx, _)| (*msg_idx, *sess_idx));
        // Take the last n
        let start = all_messages.len().saturating_sub(n);
        all_messages
            .into_iter()
            .skip(start)
            .map(|(_, _, msg)| msg)
            .collect()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use zeus_core::Message;

    #[tokio::test]
    async fn test_store_create_and_load() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());

        let session = store.create().await.expect("should create file");
        let id = session.id.clone();

        let loaded = store.load(&id).await.expect("async operation should succeed");
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.len(), 0);
    }

    #[tokio::test]
    async fn test_store_acquire_serializes_access() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = Arc::new(SessionStore::new(tmp.path()));

        let session = store.create().await.expect("should create file");
        let id = session.id.clone();

        // Acquire lock
        let guard = store.acquire(&id).await;
        assert_eq!(guard.session_id(), id);

        // Verify we can still acquire locks for *different* sessions
        let session2 = store.create().await.expect("should create file");
        let guard2 = store.acquire(&session2.id).await;
        assert_eq!(guard2.session_id(), session2.id);

        drop(guard);
        drop(guard2);
    }

    #[tokio::test]
    async fn test_store_concurrent_different_sessions() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = Arc::new(SessionStore::new(tmp.path()));

        // Create two sessions
        let s1 = store.create().await.expect("should create file");
        let s2 = store.create().await.expect("should create file");
        let id1 = s1.id.clone();
        let id2 = s2.id.clone();

        // Access both concurrently — should not deadlock
        let store1 = store.clone();
        let store2 = store.clone();

        let (r1, r2) = tokio::join!(
            async move {
                let _guard = store1.acquire(&id1).await;
                let mut session = store1.load(&id1).await.expect("async operation should succeed");
                session.add(Message::user("hello from s1")).await.expect("async operation should succeed");
                session.len()
            },
            async move {
                let _guard = store2.acquire(&id2).await;
                let mut session = store2.load(&id2).await.expect("async operation should succeed");
                session.add(Message::user("hello from s2")).await.expect("async operation should succeed");
                session.len()
            },
        );

        assert_eq!(r1, 1);
        assert_eq!(r2, 1);
    }

    #[tokio::test]
    async fn test_store_concurrent_same_session_serialized() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = Arc::new(SessionStore::new(tmp.path()));

        let session = store.create().await.expect("should create file");
        let id = session.id.clone();

        // Counter to verify serialization: each task increments after writing
        let counter = Arc::new(AtomicU32::new(0));

        let mut handles = Vec::new();
        for i in 0..5 {
            let store = store.clone();
            let id = id.clone();
            let counter = counter.clone();
            handles.push(tokio::spawn(async move {
                let _guard = store.acquire(&id).await;
                // While we hold the lock, load-modify-save
                let mut session = store.load(&id).await.expect("async operation should succeed");
                session
                    .add(Message::user(&format!("msg {}", i)))
                    .await
                    .expect("async operation should succeed");
                counter.fetch_add(1, Ordering::SeqCst);
            }));
        }

        for h in handles {
            h.await.expect("async operation should succeed");
        }

        // All 5 tasks completed
        assert_eq!(counter.load(Ordering::SeqCst), 5);

        // Session file should have all 5 messages (no corruption)
        let final_session = store.load(&id).await.expect("async operation should succeed");
        assert_eq!(final_session.len(), 5);
    }

    #[tokio::test]
    async fn test_store_list() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());

        store.create().await.expect("should create file");
        store.create().await.expect("should create file");
        store.create().await.expect("should create file");

        let list = store.list().await.expect("async operation should succeed");
        assert_eq!(list.len(), 3);
    }

    #[tokio::test]
    async fn test_store_evict() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());

        let session = store.create().await.expect("should create file");
        assert_eq!(store.active_count().await, 1);

        store.evict(&session.id).await;
        assert_eq!(store.active_count().await, 0);
    }

    #[tokio::test]
    async fn test_store_active_count() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());

        assert_eq!(store.active_count().await, 0);

        store.create().await.expect("should create file");
        assert_eq!(store.active_count().await, 1);

        // acquire also creates a lock entry
        let _guard = store.acquire("manual-id").await;
        assert_eq!(store.active_count().await, 2);
    }

    #[tokio::test]
    async fn test_store_load_nonexistent() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());

        let result = store.load("nonexistent").await;
        assert!(result.is_err());
    }

    // S53-T8b: Labeled persistent session tests

    #[tokio::test]
    async fn test_store_get_or_create_labeled_new() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());

        let session = store
            .get_or_create_labeled("pr-monitor")
            .await
            .expect("should create labeled session");
        assert_eq!(session.id, "agent-pr-monitor");
        assert_eq!(session.label, Some("pr-monitor".to_string()));
        assert_eq!(session.messages.len(), 0);
    }

    #[tokio::test]
    async fn test_store_get_or_create_labeled_resumes() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());

        // Create and add a message
        let mut session = store
            .get_or_create_labeled("pr-monitor")
            .await
            .expect("should create");
        session
            .add(Message::user("first message"))
            .await
            .expect("should add");

        // Get the same labeled session — should resume with the message
        let resumed = store
            .get_or_create_labeled("pr-monitor")
            .await
            .expect("should resume");
        assert_eq!(resumed.id, "agent-pr-monitor");
        assert_eq!(resumed.label, Some("pr-monitor".to_string()));
        assert_eq!(resumed.messages.len(), 1);
        assert_eq!(resumed.messages[0].content, "first message");
    }

    #[tokio::test]
    async fn test_store_list_labeled() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());

        // Create labeled sessions
        store
            .get_or_create_labeled("pr-monitor")
            .await
            .expect("should create");
        store
            .get_or_create_labeled("log-watcher")
            .await
            .expect("should create");

        // Create a regular (unlabeled) session
        store.create().await.expect("should create");

        // list_labeled should only return agent sessions
        let labeled = store.list_labeled().await.expect("should list");
        assert_eq!(labeled.len(), 2);
        let labels: Vec<&str> = labeled.iter().map(|(l, _, _)| l.as_str()).collect();
        assert!(labels.contains(&"pr-monitor"));
        assert!(labels.contains(&"log-watcher"));
    }

    #[tokio::test]
    async fn test_store_labeled_different_labels_independent() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());

        let mut s1 = store
            .get_or_create_labeled("agent-a")
            .await
            .expect("should create");
        s1.add(Message::user("msg for a"))
            .await
            .expect("should add");

        let s2 = store
            .get_or_create_labeled("agent-b")
            .await
            .expect("should create");

        // agent-b should be empty — independent from agent-a
        assert_eq!(s2.messages.len(), 0);
        assert_ne!(s1.id, s2.id);
    }

    #[tokio::test]
    async fn test_store_acquire_reentrant_different_ids() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());

        // Acquiring locks for different IDs from the same task should work
        let _g1 = store.acquire("a").await;
        let _g2 = store.acquire("b").await;
        let _g3 = store.acquire("c").await;

        assert_eq!(store.active_count().await, 3);
    }

    #[tokio::test]
    async fn test_store_concurrent_writes_no_corruption() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = Arc::new(SessionStore::new(tmp.path()));

        let session = store.create().await.expect("should create file");
        let id = session.id.clone();

        // Spawn 10 concurrent writers, each adding 3 messages
        let mut handles = Vec::new();
        for batch in 0..10 {
            let store = store.clone();
            let id = id.clone();
            handles.push(tokio::spawn(async move {
                let _guard = store.acquire(&id).await;
                let mut session = store.load(&id).await.expect("async operation should succeed");
                for j in 0..3 {
                    session
                        .add(Message::user(&format!("batch {} msg {}", batch, j)))
                        .await
                        .expect("async operation should succeed");
                }
            }));
        }

        for h in handles {
            h.await.expect("async operation should succeed");
        }

        // All 30 messages should be in the file, no corruption
        let final_session = store.load(&id).await.expect("async operation should succeed");
        assert_eq!(final_session.len(), 30);

        // Verify each message is valid (not garbled)
        for msg in &final_session.messages {
            assert!(msg.content.starts_with("batch "));
        }
    }

    // -----------------------------------------------------------------------
    // #91 Cut 1 — load_cross_channel_tail tests
    // -----------------------------------------------------------------------

    /// 1. Off by default — n=0 returns empty immediately.
    #[tokio::test]
    async fn test_cross_channel_tail_off_returns_empty() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());
        let s = store.create().await.expect("create");
        let tail = store.load_cross_channel_tail(&s.id, 0).await;
        assert!(tail.is_empty(), "n=0 must return empty vec");
    }

    /// 2. No other sessions — empty tail, no-op.
    #[tokio::test]
    async fn test_cross_channel_tail_no_other_sessions() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());
        let s = store.create().await.expect("create");
        let tail = store.load_cross_channel_tail(&s.id, 5).await;
        assert!(tail.is_empty(), "sole session → empty tail");
    }

    /// 3. except_session_id correctly excluded.
    #[tokio::test]
    async fn test_cross_channel_tail_excludes_current_session() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());

        let mut current = store.create().await.expect("create current");
        current.add(Message::user("current session msg")).await.expect("add");

        let mut other = store.create().await.expect("create other");
        other.add(Message::user("other session msg")).await.expect("add");

        let tail = store.load_cross_channel_tail(&current.id, 10).await;
        // Must only contain messages from `other`, not `current`
        assert!(!tail.is_empty(), "tail should have other session's messages");
        assert!(
            tail.iter().all(|m| m.content != "current session msg"),
            "current session must be excluded"
        );
        assert!(
            tail.iter().any(|m| m.content == "other session msg"),
            "other session messages must be included"
        );
    }

    /// 4. on with n=5 — tail loaded and injected correctly.
    #[tokio::test]
    async fn test_cross_channel_tail_on_with_n5() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());

        let current = store.create().await.expect("create current");

        // Other session with 8 messages
        let mut other = store.create().await.expect("create other");
        for i in 0..8u32 {
            other.add(Message::user(format!("msg-{}", i))).await.expect("add");
        }

        let tail = store.load_cross_channel_tail(&current.id, 5).await;
        // Should return exactly 5 (the last 5)
        assert_eq!(tail.len(), 5, "n=5 should return exactly 5 messages");
        // Last message should be msg-7
        assert_eq!(tail.last().unwrap().content, "msg-7");
    }

    /// 5. Chronological interleave order across multiple sessions.
    #[tokio::test]
    async fn test_cross_channel_tail_chronological_interleave() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());

        let current = store.create().await.expect("create current");

        let mut s1 = store.create().await.expect("s1");
        let mut s2 = store.create().await.expect("s2");

        // Interleave: s1 and s2 each get 3 messages
        s1.add(Message::user("s1-0")).await.expect("add");
        s2.add(Message::user("s2-0")).await.expect("add");
        s1.add(Message::user("s1-1")).await.expect("add");
        s2.add(Message::user("s2-1")).await.expect("add");
        s1.add(Message::user("s1-2")).await.expect("add");
        s2.add(Message::user("s2-2")).await.expect("add");

        let tail = store.load_cross_channel_tail(&current.id, 10).await;
        // 6 total messages from 2 sessions
        assert_eq!(tail.len(), 6);
        // Interleaved by message index: [s1-0,s2-0, s1-1,s2-1, s1-2,s2-2] or similar
        // Verify all messages present
        let contents: Vec<&str> = tail.iter().map(|m| m.content.as_str()).collect();
        assert!(contents.contains(&"s1-0"));
        assert!(contents.contains(&"s2-0"));
        assert!(contents.contains(&"s1-2"));
        assert!(contents.contains(&"s2-2"));
    }

    /// 6. LLM message-budget cap: n larger than available messages returns all available.
    #[tokio::test]
    async fn test_cross_channel_tail_budget_cap() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let store = SessionStore::new(tmp.path());

        let current = store.create().await.expect("create current");
        let mut other = store.create().await.expect("create other");
        other.add(Message::user("only-msg")).await.expect("add");

        // Request 100 but only 1 available
        let tail = store.load_cross_channel_tail(&current.id, 100).await;
        assert_eq!(tail.len(), 1, "should return all available when n > total");
        assert_eq!(tail[0].content, "only-msg");
    }
}
