//! SQLite-backed Slack message history cache (S50-T4).
//!
//! Mirrors the DiscordHistoryStore pattern to provide Slack channel parity.
//! Caches inbound Slack messages so agents have conversation context
//! after gateway restarts.
//!
//! Table: `slack_messages` — cached channel/DM messages with author metadata.

use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;

/// Versioned schema migrations for the Slack history cache.
const SLACK_HISTORY_MIGRATIONS: &[&str] = &[
    // v1 — initial schema
    "CREATE TABLE IF NOT EXISTS slack_messages (
        id TEXT PRIMARY KEY,
        channel_id TEXT NOT NULL,
        author_id TEXT NOT NULL,
        author_name TEXT NOT NULL DEFAULT '',
        content TEXT NOT NULL DEFAULT '',
        timestamp INTEGER NOT NULL DEFAULT 0,
        thread_ts TEXT DEFAULT NULL,
        is_bot INTEGER NOT NULL DEFAULT 0,
        is_dm INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS idx_slack_channel ON slack_messages(channel_id);
    CREATE INDEX IF NOT EXISTS idx_slack_timestamp ON slack_messages(timestamp);
    CREATE INDEX IF NOT EXISTS idx_slack_author ON slack_messages(author_id);
    CREATE INDEX IF NOT EXISTS idx_slack_thread ON slack_messages(thread_ts);",
];

/// A cached Slack message row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedSlackMessage {
    pub id: String,
    pub channel_id: String,
    pub author_id: String,
    pub author_name: String,
    pub content: String,
    pub timestamp: i64,
    pub thread_ts: Option<String>,
    pub is_bot: bool,
    pub is_dm: bool,
}

// ============================================================================
// SlackHistoryStore
// ============================================================================

#[derive(Clone)]
pub struct SlackHistoryStore {
    db: Arc<Mutex<Connection>>,
}

impl SlackHistoryStore {
    /// Open (or create) the Slack history SQLite database.
    pub fn new(db_path: &PathBuf) -> Result<Self, String> {
        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open slack history db: {}", e))?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;",
        )
        .map_err(|e| format!("Failed to set pragmas: {}", e))?;

        crate::db::run_migrations(&conn, SLACK_HISTORY_MIGRATIONS)
            .map_err(|e| format!("Slack history schema migration failed: {e}"))?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory store (for fallback / tests).
    pub fn in_memory() -> Result<Self, String> {
        let path = PathBuf::from(":memory:");
        Self::new(&path)
    }

    /// Insert a message (upsert — ignores duplicates by ID).
    pub async fn insert(&self, msg: &CachedSlackMessage) {
        let db = self.db.lock().await;
        if let Err(e) = db.execute(
            "INSERT OR IGNORE INTO slack_messages 
             (id, channel_id, author_id, author_name, content, timestamp, thread_ts, is_bot, is_dm)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                msg.id, msg.channel_id, msg.author_id, msg.author_name,
                msg.content, msg.timestamp, msg.thread_ts, 
                msg.is_bot as i32, msg.is_dm as i32,
            ],
        ) {
            warn!("Failed to cache Slack message {}: {}", msg.id, e);
        }
    }

    /// Query recent messages for a channel, newest first.
    pub async fn get_history(
        &self,
        channel_id: &str,
        limit: usize,
    ) -> Vec<CachedSlackMessage> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, channel_id, author_id, author_name, content, timestamp, thread_ts, is_bot, is_dm
             FROM slack_messages
             WHERE channel_id = ?1
             ORDER BY timestamp DESC
             LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(e) => { warn!("Failed to prepare Slack history query: {}", e); return vec![]; }
        };
        let rows = stmt.query_map(params![channel_id, limit as i64], |row| {
            Ok(row_to_cached_slack(row))
        });
        match rows {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(e) => { warn!("Failed to query Slack history: {}", e); vec![] }
        }
    }

    /// Query thread messages for context injection.
    /// Returns messages in the specified thread, excluding the triggering message.
    pub async fn get_thread_history(
        &self,
        channel_id: &str,
        thread_ts: &str,
        limit: usize,
    ) -> Vec<CachedSlackMessage> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, channel_id, author_id, author_name, content, timestamp, thread_ts, is_bot, is_dm
             FROM slack_messages
             WHERE channel_id = ?1 AND thread_ts = ?2
             ORDER BY timestamp ASC
             LIMIT ?3",
        ) {
            Ok(s) => s,
            Err(e) => { warn!("Failed to prepare thread history query: {}", e); return vec![]; }
        };
        let rows = stmt.query_map(params![channel_id, thread_ts, limit as i64], |row| {
            Ok(row_to_cached_slack(row))
        });
        match rows {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(e) => { warn!("Failed to query thread history: {}", e); vec![] }
        }
    }

    /// Search messages by content (LIKE %query%).
    pub async fn search(
        &self,
        query: &str,
        channel_id: Option<&str>,
        limit: usize,
    ) -> Vec<CachedSlackMessage> {
        let db = self.db.lock().await;
        let like_pattern = format!("%{}%", query);
        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(cid) = channel_id {
            (
                "SELECT id, channel_id, author_id, author_name, content, timestamp, thread_ts, is_bot, is_dm
                 FROM slack_messages
                 WHERE channel_id = ?1 AND content LIKE ?2
                 ORDER BY timestamp DESC
                 LIMIT ?3".to_string(),
                vec![
                    Box::new(cid.to_string()) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(like_pattern),
                    Box::new(limit as i64),
                ],
            )
        } else {
            (
                "SELECT id, channel_id, author_id, author_name, content, timestamp, thread_ts, is_bot, is_dm
                 FROM slack_messages
                 WHERE content LIKE ?1
                 ORDER BY timestamp DESC
                 LIMIT ?2".to_string(),
                vec![
                    Box::new(like_pattern) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(limit as i64),
                ],
            )
        };
        let mut stmt = match db.prepare(&sql) {
            Ok(s) => s,
            Err(e) => { warn!("Failed to prepare Slack search query: {}", e); return vec![]; }
        };
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| Ok(row_to_cached_slack(row)));
        match rows {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(e) => { warn!("Failed to search Slack history: {}", e); vec![] }
        }
    }

    /// Count total cached messages.
    pub async fn count(&self) -> usize {
        let db = self.db.lock().await;
        db.query_row("SELECT COUNT(*) FROM slack_messages", [], |row| row.get::<_, usize>(0))
            .unwrap_or(0)
    }

    /// Count messages per channel.
    pub async fn count_by_channel(&self) -> Vec<(String, usize)> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT channel_id, COUNT(*) FROM slack_messages GROUP BY channel_id ORDER BY COUNT(*) DESC"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
        });
        match rows {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// Prune old messages, keeping only the most recent `keep` per channel.
    pub async fn prune(&self, keep_per_channel: usize) -> usize {
        let db = self.db.lock().await;
        // Get channels
        let channels: Vec<String> = {
            let mut stmt = match db.prepare("SELECT DISTINCT channel_id FROM slack_messages") {
                Ok(s) => s,
                Err(_) => return 0,
            };
            stmt.query_map([], |row| row.get::<_, String>(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
        };

        let mut total_deleted = 0;
        for cid in &channels {
            let deleted = db.execute(
                "DELETE FROM slack_messages WHERE channel_id = ?1 AND id NOT IN (
                    SELECT id FROM slack_messages WHERE channel_id = ?1 ORDER BY timestamp DESC LIMIT ?2
                )",
                params![cid, keep_per_channel as i64],
            ).unwrap_or(0);
            total_deleted += deleted;
        }
        total_deleted
    }

    /// Build thread context string for injection (mirrors slack_relay.rs pattern).
    pub async fn build_thread_context(
        &self,
        channel_id: &str,
        thread_ts: &str,
        max_messages: Option<usize>,
    ) -> Option<String> {
        let limit = max_messages.unwrap_or(20);
        let thread_msgs = self.get_thread_history(channel_id, thread_ts, limit).await;
        
        if thread_msgs.len() <= 1 {
            return None; // Only the root message, no context needed
        }

        // Exclude the most recent message (the one that triggered this)
        let history: Vec<String> = thread_msgs[..thread_msgs.len() - 1]
            .iter()
            .map(|msg| format!("{}: {}", msg.author_name, msg.content))
            .collect();

        if history.is_empty() {
            return None;
        }

        Some(format!(
            "[Thread context — {} prior messages]\n{}",
            history.len(),
            history.join("\n")
        ))
    }
}

// ── Helpers ─────────────────────────────────────────────────────

fn row_to_cached_slack(row: &rusqlite::Row) -> CachedSlackMessage {
    CachedSlackMessage {
        id: row.get(0).unwrap_or_default(),
        channel_id: row.get(1).unwrap_or_default(),
        author_id: row.get(2).unwrap_or_default(),
        author_name: row.get(3).unwrap_or_default(),
        content: row.get(4).unwrap_or_default(),
        timestamp: row.get(5).unwrap_or(0),
        thread_ts: row.get(6).unwrap_or(None),
        is_bot: row.get::<_, i32>(7).unwrap_or(0) != 0,
        is_dm: row.get::<_, i32>(8).unwrap_or(0) != 0,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> SlackHistoryStore {
        SlackHistoryStore::in_memory().unwrap()
    }

    #[tokio::test]
    async fn test_slack_insert_and_get() {
        let store = test_store().await;
        let msg = CachedSlackMessage {
            id: "msg1.ts".to_string(),
            channel_id: "C12345".to_string(),
            author_id: "U67890".to_string(),
            author_name: "alice".to_string(),
            content: "Hello Slack!".to_string(),
            timestamp: 1640000000,
            thread_ts: None,
            is_bot: false,
            is_dm: false,
        };
        store.insert(&msg).await;
        let history = store.get_history("C12345", 10).await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "Hello Slack!");
        assert_eq!(history[0].author_name, "alice");
    }

    #[tokio::test]
    async fn test_slack_thread_context() {
        let store = test_store().await;
        let thread_ts = "1234567890.000100";
        
        // Thread root
        store.insert(&CachedSlackMessage {
            id: thread_ts.to_string(),
            channel_id: "C111".to_string(),
            author_id: "U1".to_string(),
            author_name: "alice".to_string(),
            content: "How do we deploy?".to_string(),
            timestamp: 1000,
            thread_ts: Some(thread_ts.to_string()),
            is_bot: false,
            is_dm: false,
        }).await;

        // Thread replies
        store.insert(&CachedSlackMessage {
            id: "reply1.ts".to_string(),
            channel_id: "C111".to_string(),
            author_id: "U2".to_string(),
            author_name: "bob".to_string(),
            content: "Use the deploy script".to_string(),
            timestamp: 1001,
            thread_ts: Some(thread_ts.to_string()),
            is_bot: false,
            is_dm: false,
        }).await;

        store.insert(&CachedSlackMessage {
            id: "reply2.ts".to_string(),
            channel_id: "C111".to_string(),
            author_id: "U1".to_string(),
            author_name: "alice".to_string(),
            content: "Thanks! That worked.".to_string(),
            timestamp: 1002,
            thread_ts: Some(thread_ts.to_string()),
            is_bot: false,
            is_dm: false,
        }).await;

        let context = store.build_thread_context("C111", thread_ts, Some(20)).await;
        assert!(context.is_some());
        let ctx = context.unwrap();
        assert!(ctx.contains("Thread context — 2 prior messages"));
        assert!(ctx.contains("alice: How do we deploy?"));
        assert!(ctx.contains("bob: Use the deploy script"));
        // Should NOT contain the most recent message
        assert!(!ctx.contains("Thanks! That worked."));
    }

    #[tokio::test]
    async fn test_slack_dm_detection() {
        let store = test_store().await;
        store.insert(&CachedSlackMessage {
            id: "dm1".to_string(),
            channel_id: "D12345".to_string(), // DM channel
            author_id: "U1".to_string(),
            author_name: "user1".to_string(),
            content: "Private message".to_string(),
            timestamp: 1000,
            thread_ts: None,
            is_bot: false,
            is_dm: true,
        }).await;

        let history = store.get_history("D12345", 10).await;
        assert_eq!(history.len(), 1);
        assert!(history[0].is_dm);
    }

    #[tokio::test]
    async fn test_slack_search_thread() {
        let store = test_store().await;
        store.insert(&CachedSlackMessage {
            id: "search1".to_string(),
            channel_id: "C111".to_string(),
            author_id: "U1".to_string(),
            author_name: "dev1".to_string(),
            content: "Fix the deployment pipeline".to_string(),
            timestamp: 1000,
            thread_ts: Some("thread1".to_string()),
            is_bot: false,
            is_dm: false,
        }).await;

        let results = store.search("deployment", None, 10).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].thread_ts, Some("thread1".to_string()));
    }

    #[tokio::test]
    async fn test_slack_prune_preserves_recent() {
        let store = test_store().await;
        for i in 0..10 {
            store.insert(&CachedSlackMessage {
                id: format!("p{}.ts", i),
                channel_id: "C111".to_string(),
                author_id: "U1".to_string(),
                author_name: "test".to_string(),
                content: format!("Message {}", i),
                timestamp: 1000 + i,
                thread_ts: None,
                is_bot: false,
                is_dm: false,
            }).await;
        }
        assert_eq!(store.count().await, 10);
        let deleted = store.prune(5).await;
        assert_eq!(deleted, 5);
        assert_eq!(store.count().await, 5);
        
        // Should keep the 5 newest
        let history = store.get_history("C111", 10).await;
        assert_eq!(history[0].content, "Message 9"); // newest kept
        assert_eq!(history[4].content, "Message 5"); // oldest kept
    }
}