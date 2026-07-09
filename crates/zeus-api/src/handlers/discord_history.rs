//! SQLite-backed channel message history cache (S52-T2, #317).
//!
//! Caches inbound messages from all channels (Discord, IRC, Telegram, Slack)
//! so agents have conversation context after gateway restarts.
//! Follows the TaskStore/DeployStore pattern.
//!
//! Table: `discord_messages` — cached channel messages with author metadata.
//! Channel IDs are prefixed with `"{channel_type}:"` to avoid cross-channel collisions.

use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;

/// Versioned schema migrations for the Discord history cache.
const HISTORY_MIGRATIONS: &[&str] = &[
    // v1 — initial schema
    "CREATE TABLE IF NOT EXISTS discord_messages (
        id TEXT PRIMARY KEY,
        channel_id TEXT NOT NULL,
        author_id TEXT NOT NULL,
        author_name TEXT NOT NULL DEFAULT '',
        content TEXT NOT NULL DEFAULT '',
        timestamp INTEGER NOT NULL DEFAULT 0,
        is_bot INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS idx_discord_channel ON discord_messages(channel_id);
    CREATE INDEX IF NOT EXISTS idx_discord_timestamp ON discord_messages(timestamp);
    CREATE INDEX IF NOT EXISTS idx_discord_author ON discord_messages(author_id);",
];

/// A cached Discord message row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedMessage {
    pub id: String,
    pub channel_id: String,
    pub author_id: String,
    pub author_name: String,
    pub content: String,
    pub timestamp: i64,
    pub is_bot: bool,
}

// ============================================================================
// DiscordHistoryStore
// ============================================================================

#[derive(Clone)]
pub struct DiscordHistoryStore {
    db: Arc<Mutex<Connection>>,
}

impl DiscordHistoryStore {
    /// Open (or create) the Discord history SQLite database.
    pub fn new(db_path: &PathBuf) -> Result<Self, String> {
        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open discord history db: {}", e))?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;",
        )
        .map_err(|e| format!("Failed to set pragmas: {}", e))?;

        crate::db::run_migrations(&conn, HISTORY_MIGRATIONS)
            .map_err(|e| format!("Discord history schema migration failed: {e}"))?;

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
    pub async fn insert(&self, msg: &CachedMessage) {
        let db = self.db.lock().await;
        if let Err(e) = db.execute(
            "INSERT OR IGNORE INTO discord_messages (id, channel_id, author_id, author_name, content, timestamp, is_bot)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                msg.id, msg.channel_id, msg.author_id, msg.author_name,
                msg.content, msg.timestamp, msg.is_bot as i32,
            ],
        ) {
            warn!("Failed to cache Discord message {}: {}", msg.id, e);
        }
    }

    /// Query recent messages for a channel, newest first.
    ///
    /// If `since_timestamp` > 0, only messages after that epoch are returned.
    pub async fn get_history(
        &self,
        channel_id: &str,
        limit: usize,
    ) -> Vec<CachedMessage> {
        self.get_history_since(channel_id, limit, 0).await
    }

    /// Query recent messages for a channel since a given timestamp, newest first.
    pub async fn get_history_since(
        &self,
        channel_id: &str,
        limit: usize,
        since_timestamp: i64,
    ) -> Vec<CachedMessage> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, channel_id, author_id, author_name, content, timestamp, is_bot
             FROM discord_messages
             WHERE channel_id = ?1 AND timestamp > ?2
             ORDER BY timestamp DESC
             LIMIT ?3",
        ) {
            Ok(s) => s,
            Err(e) => { warn!("Failed to prepare history query: {}", e); return vec![]; }
        };
        let rows = stmt.query_map(params![channel_id, since_timestamp, limit as i64], |row| {
            Ok(row_to_cached(row))
        });
        match rows {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(e) => { warn!("Failed to query history: {}", e); vec![] }
        }
    }

    /// Get the timestamp of the most recent bot message in a channel.
    pub async fn last_bot_response_timestamp(
        &self,
        channel_id: &str,
    ) -> Option<i64> {
        let db = self.db.lock().await;
        db.query_row(
            "SELECT timestamp FROM discord_messages
             WHERE channel_id = ?1 AND is_bot = 1
             ORDER BY timestamp DESC LIMIT 1",
            params![channel_id],
            |row| row.get::<_, i64>(0),
        ).ok()
    }

    /// Search messages by content (LIKE %query%).
    pub async fn search(
        &self,
        query: &str,
        channel_id: Option<&str>,
        limit: usize,
    ) -> Vec<CachedMessage> {
        let db = self.db.lock().await;
        let like_pattern = format!("%{}%", query);
        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(cid) = channel_id {
            (
                "SELECT id, channel_id, author_id, author_name, content, timestamp, is_bot
                 FROM discord_messages
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
                "SELECT id, channel_id, author_id, author_name, content, timestamp, is_bot
                 FROM discord_messages
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
            Err(e) => { warn!("Failed to prepare search query: {}", e); return vec![]; }
        };
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| Ok(row_to_cached(row)));
        match rows {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(e) => { warn!("Failed to search history: {}", e); vec![] }
        }
    }

    /// Count total cached messages.
    pub async fn count(&self) -> usize {
        let db = self.db.lock().await;
        db.query_row("SELECT COUNT(*) FROM discord_messages", [], |row| row.get::<_, usize>(0))
            .unwrap_or(0)
    }

    /// Count messages per channel.
    pub async fn count_by_channel(&self) -> Vec<(String, usize)> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT channel_id, COUNT(*) FROM discord_messages GROUP BY channel_id ORDER BY COUNT(*) DESC"
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
            let mut stmt = match db.prepare("SELECT DISTINCT channel_id FROM discord_messages") {
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
                "DELETE FROM discord_messages WHERE channel_id = ?1 AND id NOT IN (
                    SELECT id FROM discord_messages WHERE channel_id = ?1 ORDER BY timestamp DESC LIMIT ?2
                )",
                params![cid, keep_per_channel as i64],
            ).unwrap_or(0);
            total_deleted += deleted;
        }
        total_deleted
    }
}

// ── Helpers ─────────────────────────────────────────────────────

fn row_to_cached(row: &rusqlite::Row) -> CachedMessage {
    CachedMessage {
        id: row.get(0).unwrap_or_default(),
        channel_id: row.get(1).unwrap_or_default(),
        author_id: row.get(2).unwrap_or_default(),
        author_name: row.get(3).unwrap_or_default(),
        content: row.get(4).unwrap_or_default(),
        timestamp: row.get(5).unwrap_or(0),
        is_bot: row.get::<_, i32>(6).unwrap_or(0) != 0,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> DiscordHistoryStore {
        DiscordHistoryStore::in_memory().unwrap()
    }

    #[tokio::test]
    async fn test_insert_and_get() {
        let store = test_store().await;
        let msg = CachedMessage {
            id: "msg1".to_string(),
            channel_id: "ch1".to_string(),
            author_id: "user1".to_string(),
            author_name: "Alice".to_string(),
            content: "Hello world".to_string(),
            timestamp: 1000,
            is_bot: false,
        };
        store.insert(&msg).await;
        let history = store.get_history("ch1", 10).await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "Hello world");
        assert_eq!(history[0].author_name, "Alice");
    }

    #[tokio::test]
    async fn test_dedup_by_id() {
        let store = test_store().await;
        let msg = CachedMessage {
            id: "dup1".to_string(),
            channel_id: "ch1".to_string(),
            author_id: "user1".to_string(),
            author_name: "Bob".to_string(),
            content: "First version".to_string(),
            timestamp: 1000,
            is_bot: false,
        };
        store.insert(&msg).await;
        // Insert same ID again — should be ignored
        let msg2 = CachedMessage { content: "Second version".to_string(), ..msg };
        store.insert(&msg2).await;
        let history = store.get_history("ch1", 10).await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "First version"); // Original kept
    }

    #[tokio::test]
    async fn test_history_ordering_and_limit() {
        let store = test_store().await;
        for i in 0..5 {
            store.insert(&CachedMessage {
                id: format!("m{}", i),
                channel_id: "ch1".to_string(),
                author_id: "user1".to_string(),
                author_name: "Test".to_string(),
                content: format!("Message {}", i),
                timestamp: 1000 + i,
                is_bot: false,
            }).await;
        }
        // Should return newest first, limited to 3
        let history = store.get_history("ch1", 3).await;
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].content, "Message 4"); // newest
        assert_eq!(history[2].content, "Message 2");
    }

    #[tokio::test]
    async fn test_channel_isolation() {
        let store = test_store().await;
        store.insert(&CachedMessage {
            id: "a1".to_string(), channel_id: "ch-a".to_string(),
            author_id: "u1".to_string(), author_name: "A".to_string(),
            content: "In channel A".to_string(), timestamp: 1000, is_bot: false,
        }).await;
        store.insert(&CachedMessage {
            id: "b1".to_string(), channel_id: "ch-b".to_string(),
            author_id: "u2".to_string(), author_name: "B".to_string(),
            content: "In channel B".to_string(), timestamp: 1001, is_bot: true,
        }).await;
        assert_eq!(store.get_history("ch-a", 10).await.len(), 1);
        assert_eq!(store.get_history("ch-b", 10).await.len(), 1);
        assert_eq!(store.get_history("ch-c", 10).await.len(), 0);
    }

    #[tokio::test]
    async fn test_search() {
        let store = test_store().await;
        store.insert(&CachedMessage {
            id: "s1".to_string(), channel_id: "ch1".to_string(),
            author_id: "u1".to_string(), author_name: "Dev".to_string(),
            content: "Fix the deploy script".to_string(), timestamp: 1000, is_bot: false,
        }).await;
        store.insert(&CachedMessage {
            id: "s2".to_string(), channel_id: "ch1".to_string(),
            author_id: "u2".to_string(), author_name: "Dev2".to_string(),
            content: "Unrelated chatter".to_string(), timestamp: 1001, is_bot: false,
        }).await;

        let results = store.search("deploy", None, 10).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "s1");

        // Search with channel filter
        let results = store.search("deploy", Some("ch1"), 10).await;
        assert_eq!(results.len(), 1);
        let results = store.search("deploy", Some("ch-other"), 10).await;
        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn test_count_and_count_by_channel() {
        let store = test_store().await;
        for (id, ch) in [("c1", "ch-a"), ("c2", "ch-a"), ("c3", "ch-b")] {
            store.insert(&CachedMessage {
                id: id.to_string(), channel_id: ch.to_string(),
                author_id: "u1".to_string(), author_name: "T".to_string(),
                content: "test".to_string(), timestamp: 1000, is_bot: false,
            }).await;
        }
        assert_eq!(store.count().await, 3);
        let by_ch = store.count_by_channel().await;
        assert_eq!(by_ch.len(), 2);
    }

    #[tokio::test]
    async fn test_prune() {
        let store = test_store().await;
        for i in 0..10 {
            store.insert(&CachedMessage {
                id: format!("p{}", i), channel_id: "ch1".to_string(),
                author_id: "u1".to_string(), author_name: "T".to_string(),
                content: format!("Msg {}", i), timestamp: 1000 + i, is_bot: false,
            }).await;
        }
        assert_eq!(store.count().await, 10);
        let deleted = store.prune(5).await;
        assert_eq!(deleted, 5);
        assert_eq!(store.count().await, 5);
        // Should keep the 5 newest
        let history = store.get_history("ch1", 10).await;
        assert_eq!(history[0].content, "Msg 9"); // newest kept
    }

    #[tokio::test]
    async fn test_get_history_since() {
        let store = test_store().await;
        for i in 0..5 {
            store.insert(&CachedMessage {
                id: format!("ts{}", i), channel_id: "ch1".to_string(),
                author_id: "u1".to_string(), author_name: "T".to_string(),
                content: format!("Msg {}", i), timestamp: 1000 + i * 100, is_bot: false,
            }).await;
        }
        // Only messages after timestamp 1200
        let history = store.get_history_since("ch1", 10, 1200).await;
        assert_eq!(history.len(), 2); // ts=1300 and ts=1400
        assert_eq!(history[0].content, "Msg 4"); // newest first
        assert_eq!(history[1].content, "Msg 3");
        // All messages (since=0 behaves like get_history)
        let all = store.get_history_since("ch1", 10, 0).await;
        assert_eq!(all.len(), 5);
    }

    #[tokio::test]
    async fn test_last_bot_response_timestamp() {
        let store = test_store().await;
        // User message
        store.insert(&CachedMessage {
            id: "u1".to_string(), channel_id: "ch1".to_string(),
            author_id: "user".to_string(), author_name: "Human".to_string(),
            content: "hello".to_string(), timestamp: 1000, is_bot: false,
        }).await;
        // No bot messages yet
        assert_eq!(store.last_bot_response_timestamp("ch1").await, None);
        // Bot message
        store.insert(&CachedMessage {
            id: "b1".to_string(), channel_id: "ch1".to_string(),
            author_id: "bot".to_string(), author_name: "Zeus".to_string(),
            content: "response".to_string(), timestamp: 1500, is_bot: true,
        }).await;
        assert_eq!(store.last_bot_response_timestamp("ch1").await, Some(1500));
        // Channel isolation
        assert_eq!(store.last_bot_response_timestamp("ch-other").await, None);
    }

    #[tokio::test]
    async fn test_prefixed_channel_key_cross_channel() {
        // Contract test for #317: insert a telegram-typed CachedMessage with
        // prefixed channel_id, assert get_history_since returns it under that key.
        let store = test_store().await;
        let telegram_msg = CachedMessage {
            id: "tg1".to_string(),
            channel_id: "telegram:12345".to_string(), // prefixed key
            author_id: "user_tg".to_string(),
            author_name: "TelegramUser".to_string(),
            content: "Hello from Telegram".to_string(),
            timestamp: 2000,
            is_bot: false,
        };
        store.insert(&telegram_msg).await;

        // Should be retrievable under the prefixed key
        let history = store.get_history_since("telegram:12345", 10, 0).await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "Hello from Telegram");
        assert_eq!(history[0].channel_id, "telegram:12345");

        // Should NOT be retrievable under a discord-prefixed key for same numeric id
        let discord_history = store.get_history_since("discord:12345", 10, 0).await;
        assert_eq!(discord_history.len(), 0);

        // Should NOT be retrievable under unprefixed key
        let unprefixed_history = store.get_history_since("12345", 10, 0).await;
        assert_eq!(unprefixed_history.len(), 0);
    }
}
