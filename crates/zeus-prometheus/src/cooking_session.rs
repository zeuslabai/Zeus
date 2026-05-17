//! Cooking Session - JSONL Session Persistence
//!
//! Provides JSONL-based session logging for replay and recovery.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use zeus_core::Result;

/// JSONL entry for session logging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonlEntry {
    /// Entry type (e.g., "message", "tool_call", "event")
    pub entry_type: String,
    /// Session ID
    pub session_id: String,
    /// Entry content
    pub content: serde_json::Value,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Optional metadata
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

impl JsonlEntry {
    /// Create a new entry
    pub fn new(
        entry_type: impl Into<String>,
        session_id: impl Into<String>,
        content: serde_json::Value,
    ) -> Self {
        Self {
            entry_type: entry_type.into(),
            session_id: session_id.into(),
            content,
            timestamp: Utc::now(),
            metadata: None,
        }
    }

    /// Create with metadata
    pub fn with_metadata(
        entry_type: impl Into<String>,
        session_id: impl Into<String>,
        content: serde_json::Value,
        metadata: serde_json::Value,
    ) -> Self {
        Self {
            entry_type: entry_type.into(),
            session_id: session_id.into(),
            content,
            timestamp: Utc::now(),
            metadata: Some(metadata),
        }
    }
}

/// Session persistence manager
pub struct SessionPersistence {
    sessions_dir: PathBuf,
}

impl SessionPersistence {
    /// Create a new session persistence manager
    pub fn new(sessions_dir: impl AsRef<Path>) -> Result<Self> {
        let sessions_dir = sessions_dir.as_ref().to_path_buf();

        // Ensure directory exists
        std::fs::create_dir_all(&sessions_dir).map_err(|e| {
            zeus_core::Error::Session(format!("Failed to create sessions directory: {}", e))
        })?;

        Ok(Self { sessions_dir })
    }

    /// Get path for a session file
    fn session_path(&self, session_id: &str) -> PathBuf {
        self.sessions_dir.join(format!("{}.jsonl", session_id))
    }

    /// Append an entry to a session
    pub async fn append(&self, entry: JsonlEntry) -> Result<()> {
        let path = self.session_path(&entry.session_id);

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|e| {
                zeus_core::Error::Session(format!("Failed to open session file: {}", e))
            })?;

        let line = serde_json::to_string(&entry)
            .map_err(|e| zeus_core::Error::Session(format!("Failed to serialize entry: {}", e)))?;

        file.write_all(format!("{}\n", line).as_bytes())
            .await
            .map_err(|e| zeus_core::Error::Session(format!("Failed to write entry: {}", e)))?;

        file.flush()
            .await
            .map_err(|e| zeus_core::Error::Session(format!("Failed to flush file: {}", e)))?;

        Ok(())
    }

    /// Read all entries from a session
    pub async fn read_session(&self, session_id: &str) -> Result<Vec<JsonlEntry>> {
        let path = self.session_path(session_id);

        if !path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&path).await.map_err(|e| {
            zeus_core::Error::Session(format!("Failed to read session file: {}", e))
        })?;

        let mut entries = Vec::new();

        for (line_num, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }

            let entry: JsonlEntry = serde_json::from_str(line).map_err(|e| {
                zeus_core::Error::Session(format!("Failed to parse line {}: {}", line_num + 1, e))
            })?;

            entries.push(entry);
        }

        Ok(entries)
    }

    /// List all session IDs
    pub async fn list_sessions(&self) -> Result<Vec<String>> {
        let mut sessions = Vec::new();

        let mut entries = fs::read_dir(&self.sessions_dir).await.map_err(|e| {
            zeus_core::Error::Session(format!("Failed to read sessions directory: {}", e))
        })?;

        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            zeus_core::Error::Session(format!("Failed to read directory entry: {}", e))
        })? {
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("jsonl")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                sessions.push(stem.to_string());
            }
        }

        sessions.sort();
        Ok(sessions)
    }

    /// Delete a session
    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        let path = self.session_path(session_id);

        if path.exists() {
            fs::remove_file(&path).await.map_err(|e| {
                zeus_core::Error::Session(format!("Failed to delete session: {}", e))
            })?;
        }

        Ok(())
    }

    /// Get session entry count
    pub async fn entry_count(&self, session_id: &str) -> Result<usize> {
        let entries = self.read_session(session_id).await?;
        Ok(entries.len())
    }

    /// Check if session exists
    pub fn session_exists(&self, session_id: &str) -> bool {
        self.session_path(session_id).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn create_test_persistence() -> (SessionPersistence, TempDir) {
        let temp = TempDir::new().unwrap();
        let persistence = SessionPersistence::new(temp.path()).unwrap();
        (persistence, temp)
    }

    #[tokio::test]
    async fn test_persistence_creation() {
        let temp = TempDir::new().unwrap();
        let result = SessionPersistence::new(temp.path());
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_append_entry() {
        let (persistence, _temp) = create_test_persistence().await;

        let entry = JsonlEntry::new(
            "message",
            "test-session",
            serde_json::json!({"text": "hello"}),
        );

        let result = persistence.append(entry).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_read_session() {
        let (persistence, _temp) = create_test_persistence().await;

        let entry1 = JsonlEntry::new(
            "message",
            "test-session",
            serde_json::json!({"text": "hello"}),
        );

        let entry2 = JsonlEntry::new(
            "message",
            "test-session",
            serde_json::json!({"text": "world"}),
        );

        persistence
            .append(entry1)
            .await
            .expect("Failed to append entry1");
        persistence
            .append(entry2)
            .await
            .expect("Failed to append entry2");

        let entries = persistence
            .read_session("test-session")
            .await
            .expect("Failed to read test session");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].content["text"], "hello");
        assert_eq!(entries[1].content["text"], "world");
    }

    #[tokio::test]
    async fn test_list_sessions() {
        let (persistence, _temp) = create_test_persistence().await;

        let entry1 = JsonlEntry::new("message", "session1", serde_json::json!({}));
        let entry2 = JsonlEntry::new("message", "session2", serde_json::json!({}));

        persistence
            .append(entry1)
            .await
            .expect("Failed to append session1 entry");
        persistence
            .append(entry2)
            .await
            .expect("Failed to append session2 entry");

        let sessions = persistence
            .list_sessions()
            .await
            .expect("Failed to list sessions");
        assert_eq!(sessions.len(), 2);
        assert!(sessions.contains(&"session1".to_string()));
        assert!(sessions.contains(&"session2".to_string()));
    }

    #[tokio::test]
    async fn test_delete_session() {
        let (persistence, _temp) = create_test_persistence().await;

        let entry = JsonlEntry::new("message", "test-session", serde_json::json!({}));
        persistence
            .append(entry)
            .await
            .expect("Failed to append entry for deletion test");

        assert!(persistence.session_exists("test-session"));

        persistence
            .delete_session("test-session")
            .await
            .expect("Failed to delete test session");

        assert!(!persistence.session_exists("test-session"));
    }

    #[tokio::test]
    async fn test_entry_count() {
        let (persistence, _temp) = create_test_persistence().await;

        let entry1 = JsonlEntry::new("message", "test-session", serde_json::json!({}));
        let entry2 = JsonlEntry::new("message", "test-session", serde_json::json!({}));

        persistence
            .append(entry1)
            .await
            .expect("Failed to append entry1 for count test");
        persistence
            .append(entry2)
            .await
            .expect("Failed to append entry2 for count test");

        let count = persistence
            .entry_count("test-session")
            .await
            .expect("Failed to get entry count");
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_read_nonexistent_session() {
        let (persistence, _temp) = create_test_persistence().await;

        let entries = persistence.read_session("nonexistent").await.unwrap();
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_entry_creation() {
        let entry = JsonlEntry::new("test", "session1", serde_json::json!({"key": "value"}));
        assert_eq!(entry.entry_type, "test");
        assert_eq!(entry.session_id, "session1");
        assert_eq!(entry.content["key"], "value");
    }
}
