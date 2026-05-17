//! SQLite-backed Slack message history cache (S55-T11).
//!
//! Caches inbound Slack messages so agents have conversation context
//! after gateway restarts. Follows the DiscordHistoryStore pattern.
//!
//! Table: `slack_messages` — cached channel messages with author/thread metadata.

use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;

/// Versioned schema migrations for the Slack history cache.
const HISTORY_MIGRATIONS: &[&str] = &[
    // v1 — initial schema
    "CREATE TABLE IF NOT EXISTS slack_messages (
        id TEXT PRIMARY KEY,
        channel_id TEXT NOT NULL,
        thread_ts TEXT,
        author_id TEXT NOT NULL,
        author_name TEXT NOT NULL DEFAULT '',
        content TEXT NOT NULL DEFAULT '',
        timestamp INTEGER NOT NULL DEFAULT 0,
        is_bot INTEGER NOT NULL DEFAULT 0,
        team_id TEXT NOT NULL DEFAULT ''
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    pub author_id: String,
    pub author_name: String,
    pub content: String,
    pub timestamp: i64,
    pub is_bot: bool,
    #[serde(default)]
    pub team_id: String,
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

        crate::db::run_migrations(&conn, HISTORY_MIGRATIONS)
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
            "INSERT OR IGNORE INTO slack_messages (id, channel_id, thread_ts, author_id, author_name, content, timestamp, is_bot, team_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                msg.id, msg.channel_id, msg.thread_ts, msg.author_id, msg.author_name,
                msg.content, msg.timestamp, msg.is_bot as i32, msg.team_id,
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
            "SELECT id, channel_id, thread_ts, author_id, author_name, content, timestamp, is_bot, team_id
             FROM slack_messages
             WHERE channel_id = ?1
             ORDER BY timestamp DESC
             LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(e) => { warn!("Failed to prepare slack history query: {}", e); return vec![]; }
        };
        let rows = stmt.query_map(params![channel_id, limit as i64], |row| {
            Ok(row_to_cached(row))
        });
        match rows {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(e) => { warn!("Failed to query slack history: {}", e); vec![] }
        }
    }

    /// Query messages in a specific thread.
    pub async fn get_thread(
        &self,
        channel_id: &str,
        thread_ts: &str,
        limit: usize,
    ) -> Vec<CachedSlackMessage> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, channel_id, thread_ts, author_id, author_name, content, timestamp, is_bot, team_id
             FROM slack_messages
             WHERE channel_id = ?1 AND thread_ts = ?2
             ORDER BY timestamp ASC
             LIMIT ?3",
        ) {
            Ok(s) => s,
            Err(e) => { warn!("Failed to prepare slack thread query: {}", e); return vec![]; }
        };
        let rows = stmt.query_map(params![channel_id, thread_ts, limit as i64], |row| {
            Ok(row_to_cached(row))
        });
        match rows {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(e) => { warn!("Failed to query slack thread: {}", e); vec![] }
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
                "SELECT id, channel_id, thread_ts, author_id, author_name, content, timestamp, is_bot, team_id
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
                "SELECT id, channel_id, thread_ts, author_id, author_name, content, timestamp, is_bot, team_id
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
            Err(e) => { warn!("Failed to prepare slack search query: {}", e); return vec![]; }
        };
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| Ok(row_to_cached(row)));
        match rows {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(e) => { warn!("Failed to search slack history: {}", e); vec![] }
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
}

// ── Helpers ─────────────────────────────────────────────────────

fn row_to_cached(row: &rusqlite::Row) -> CachedSlackMessage {
    CachedSlackMessage {
        id: row.get(0).unwrap_or_default(),
        channel_id: row.get(1).unwrap_or_default(),
        thread_ts: row.get(2).ok(),
        author_id: row.get(3).unwrap_or_default(),
        author_name: row.get(4).unwrap_or_default(),
        content: row.get(5).unwrap_or_default(),
        timestamp: row.get(6).unwrap_or(0),
        is_bot: row.get::<_, i32>(7).unwrap_or(0) != 0,
        team_id: row.get(8).unwrap_or_default(),
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

    fn make_msg(id: &str, channel: &str, content: &str, ts: i64) -> CachedSlackMessage {
        CachedSlackMessage {
            id: id.to_string(),
            channel_id: channel.to_string(),
            thread_ts: None,
            author_id: "U123".to_string(),
            author_name: "testuser".to_string(),
            content: content.to_string(),
            timestamp: ts,
            is_bot: false,
            team_id: "T001".to_string(),
        }
    }

    #[tokio::test]
    async fn test_insert_and_get() {
        let store = test_store().await;
        let msg = make_msg("msg1", "C001", "Hello from Slack", 1000);
        store.insert(&msg).await;
        let history = store.get_history("C001", 10).await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "Hello from Slack");
        assert_eq!(history[0].author_name, "testuser");
        assert_eq!(history[0].team_id, "T001");
    }

    #[tokio::test]
    async fn test_dedup_by_id() {
        let store = test_store().await;
        let msg = make_msg("dup1", "C001", "First version", 1000);
        store.insert(&msg).await;
        let msg2 = CachedSlackMessage { content: "Second version".to_string(), ..msg };
        store.insert(&msg2).await;
        let history = store.get_history("C001", 10).await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "First version");
    }

    #[tokio::test]
    async fn test_history_ordering_and_limit() {
        let store = test_store().await;
        for i in 0..5 {
            store.insert(&make_msg(&format!("m{}", i), "C001", &format!("Message {}", i), 1000 + i)).await;
        }
        let history = store.get_history("C001", 3).await;
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].content, "Message 4"); // newest
        assert_eq!(history[2].content, "Message 2");
    }

    #[tokio::test]
    async fn test_channel_isolation() {
        let store = test_store().await;
        store.insert(&make_msg("a1", "C-alpha", "In alpha", 1000)).await;
        store.insert(&CachedSlackMessage {
            id: "b1".to_string(), channel_id: "C-beta".to_string(),
            thread_ts: None, author_id: "U456".to_string(),
            author_name: "otheruser".to_string(), content: "In beta".to_string(),
            timestamp: 1001, is_bot: true, team_id: "T002".to_string(),
        }).await;
        assert_eq!(store.get_history("C-alpha", 10).await.len(), 1);
        assert_eq!(store.get_history("C-beta", 10).await.len(), 1);
        assert_eq!(store.get_history("C-gamma", 10).await.len(), 0);
    }

    #[tokio::test]
    async fn test_thread_query() {
        let store = test_store().await;
        // Parent message
        store.insert(&make_msg("parent1", "C001", "Start thread", 1000)).await;
        // Thread replies
        for i in 1..=3 {
            store.insert(&CachedSlackMessage {
                id: format!("reply{}", i),
                channel_id: "C001".to_string(),
                thread_ts: Some("1000.000000".to_string()),
                author_id: "U123".to_string(),
                author_name: "testuser".to_string(),
                content: format!("Reply {}", i),
                timestamp: 1000 + i,
                is_bot: false,
                team_id: "T001".to_string(),
            }).await;
        }
        let thread = store.get_thread("C001", "1000.000000", 10).await;
        assert_eq!(thread.len(), 3);
        assert_eq!(thread[0].content, "Reply 1"); // oldest first in thread
        assert_eq!(thread[2].content, "Reply 3");
    }

    #[tokio::test]
    async fn test_search() {
        let store = test_store().await;
        store.insert(&make_msg("s1", "C001", "Fix the deploy script", 1000)).await;
        store.insert(&make_msg("s2", "C001", "Unrelated chatter", 1001)).await;

        let results = store.search("deploy", None, 10).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "s1");

        let results = store.search("deploy", Some("C001"), 10).await;
        assert_eq!(results.len(), 1);
        let results = store.search("deploy", Some("C-other"), 10).await;
        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn test_count_and_count_by_channel() {
        let store = test_store().await;
        for (id, ch) in [("c1", "C-a"), ("c2", "C-a"), ("c3", "C-b")] {
            store.insert(&make_msg(id, ch, "test", 1000)).await;
        }
        assert_eq!(store.count().await, 3);
        let by_ch = store.count_by_channel().await;
        assert_eq!(by_ch.len(), 2);
    }

    #[tokio::test]
    async fn test_prune() {
        let store = test_store().await;
        for i in 0..10 {
            store.insert(&make_msg(&format!("p{}", i), "C001", &format!("Msg {}", i), 1000 + i)).await;
        }
        assert_eq!(store.count().await, 10);
        let deleted = store.prune(5).await;
        assert_eq!(deleted, 5);
        assert_eq!(store.count().await, 5);
        let history = store.get_history("C001", 10).await;
        assert_eq!(history[0].content, "Msg 9");
    }
}
