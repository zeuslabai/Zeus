//! Structured JSONL audit writer for tool executions
//!
//! Writes tool execution events to `audit.jsonl` with structured fields:
//! timestamp, user_id, tool_name, args_summary, result, success, duration_ms.
//!
//! This is a simpler, queryable format complementing the tamper-evident
//! hash-chain audit log in `audit.rs`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use zeus_core::Result;

/// A structured tool execution audit entry for JSONL output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolAuditEntry {
    /// ISO 8601 timestamp
    pub timestamp: DateTime<Utc>,
    /// User or agent identifier
    pub user_id: String,
    /// Name of the tool executed
    pub tool_name: String,
    /// Summary of tool arguments (truncated for safety)
    pub args_summary: String,
    /// Result summary (truncated)
    pub result: String,
    /// Whether the execution succeeded
    pub success: bool,
    /// Execution duration in milliseconds
    pub duration_ms: u64,
    /// Optional session ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Source IP or channel
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl ToolAuditEntry {
    /// Create a new audit entry with the current timestamp.
    pub fn new(
        user_id: impl Into<String>,
        tool_name: impl Into<String>,
        args_summary: impl Into<String>,
        result: impl Into<String>,
        success: bool,
        duration_ms: u64,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            user_id: user_id.into(),
            tool_name: tool_name.into(),
            args_summary: truncate_str(&args_summary.into(), 512),
            result: truncate_str(&result.into(), 1024),
            success,
            duration_ms,
            session_id: None,
            source: None,
        }
    }

    /// Set the session ID.
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Set the source (IP, channel, etc).
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }
}

/// Truncate a string to `max_len` characters, appending "..." if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let end = zeus_core::floor_char_boundary(s, max_len.saturating_sub(3));
        format!("{}...", &s[..end])
    }
}

/// JSONL audit writer that appends structured entries to a file.
pub struct AuditJsonlWriter {
    path: PathBuf,
}

impl AuditJsonlWriter {
    /// Create a new writer for the given file path.
    ///
    /// The parent directory is created if it doesn't exist.
    pub async fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                zeus_core::Error::Security(format!("Failed to create audit directory: {}", e))
            })?;
        }
        Ok(Self { path })
    }

    /// Create a writer using the default path (`~/.zeus/audit.jsonl`).
    pub async fn default_path() -> Result<Self> {
        let path = zeus_core::default_config_dir().join("audit.jsonl");
        Self::new(path).await
    }

    /// Append a tool audit entry to the JSONL file.
    pub async fn log(&self, entry: &ToolAuditEntry) -> Result<()> {
        let line = serde_json::to_string(entry).map_err(|e| {
            zeus_core::Error::Security(format!("Failed to serialize audit entry: {}", e))
        })?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .map_err(|e| {
                zeus_core::Error::Security(format!("Failed to open audit.jsonl: {}", e))
            })?;

        file.write_all(format!("{}\n", line).as_bytes())
            .await
            .map_err(|e| {
                zeus_core::Error::Security(format!("Failed to write audit entry: {}", e))
            })?;

        Ok(())
    }

    /// Read all entries from the audit file.
    pub async fn read_all(&self) -> Result<Vec<ToolAuditEntry>> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = tokio::fs::File::open(&self.path).await.map_err(|e| {
            zeus_core::Error::Security(format!("Failed to open audit.jsonl: {}", e))
        })?;

        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut entries = Vec::new();

        while let Ok(Some(line)) = lines.next_line().await {
            if line.is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<ToolAuditEntry>(&line) {
                entries.push(entry);
            }
        }

        Ok(entries)
    }

    /// Read entries filtered by tool name.
    pub async fn read_by_tool(&self, tool_name: &str) -> Result<Vec<ToolAuditEntry>> {
        let all = self.read_all().await?;
        Ok(all
            .into_iter()
            .filter(|e| e.tool_name == tool_name)
            .collect())
    }

    /// Read entries filtered by user ID.
    pub async fn read_by_user(&self, user_id: &str) -> Result<Vec<ToolAuditEntry>> {
        let all = self.read_all().await?;
        Ok(all.into_iter().filter(|e| e.user_id == user_id).collect())
    }

    /// Count total entries.
    pub async fn count(&self) -> Result<usize> {
        Ok(self.read_all().await?.len())
    }

    /// Get the file path.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_audit_entry_serialization() {
        let entry = ToolAuditEntry::new(
            "user-123",
            "shell",
            r#"{"command": "ls -la"}"#,
            "file1.txt\nfile2.txt",
            true,
            42,
        );

        let json = serde_json::to_string(&entry).expect("should serialize");
        assert!(json.contains("\"user_id\":\"user-123\""));
        assert!(json.contains("\"tool_name\":\"shell\""));
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"duration_ms\":42"));

        let parsed: ToolAuditEntry = serde_json::from_str(&json).expect("should parse");
        assert_eq!(parsed.user_id, "user-123");
        assert_eq!(parsed.tool_name, "shell");
        assert!(parsed.success);
        assert_eq!(parsed.duration_ms, 42);
    }

    #[test]
    fn test_tool_audit_entry_with_session_and_source() {
        let entry = ToolAuditEntry::new("agent-a", "web_fetch", "{}", "ok", true, 100)
            .with_session("sess-456")
            .with_source("192.168.1.10");

        assert_eq!(entry.session_id, Some("sess-456".to_string()));
        assert_eq!(entry.source, Some("192.168.1.10".to_string()));

        let json = serde_json::to_string(&entry).expect("should serialize");
        assert!(json.contains("\"session_id\":\"sess-456\""));
        assert!(json.contains("\"source\":\"192.168.1.10\""));
    }

    #[test]
    fn test_tool_audit_entry_skip_none_fields() {
        let entry = ToolAuditEntry::new("u", "t", "{}", "ok", true, 0);
        let json = serde_json::to_string(&entry).expect("should serialize");
        assert!(!json.contains("session_id"));
        assert!(!json.contains("source"));
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("short", 100), "short");
        assert_eq!(truncate_str("hello world", 8), "hello...");
        assert_eq!(truncate_str("", 10), "");
        assert_eq!(truncate_str("exactly10!", 10), "exactly10!");
    }

    #[tokio::test]
    async fn test_audit_jsonl_writer_write_and_read() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let path = tmp.path().join("test-audit.jsonl");

        let writer = AuditJsonlWriter::new(&path)
            .await
            .expect("should create writer");

        // Write entries
        let entry1 = ToolAuditEntry::new(
            "user-1",
            "read_file",
            r#"{"path":"/tmp/x"}"#,
            "content",
            true,
            10,
        );
        let entry2 = ToolAuditEntry::new("user-2", "shell", r#"{"cmd":"ls"}"#, "files", true, 25);
        let entry3 = ToolAuditEntry::new("user-1", "shell", r#"{"cmd":"rm"}"#, "error", false, 5);

        writer.log(&entry1).await.expect("should log entry 1");
        writer.log(&entry2).await.expect("should log entry 2");
        writer.log(&entry3).await.expect("should log entry 3");

        // Read all
        let entries = writer.read_all().await.expect("should read all");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].tool_name, "read_file");
        assert_eq!(entries[1].tool_name, "shell");
        assert_eq!(entries[2].tool_name, "shell");

        // Read by tool
        let shell_entries = writer
            .read_by_tool("shell")
            .await
            .expect("should read by tool");
        assert_eq!(shell_entries.len(), 2);

        // Read by user
        let user1_entries = writer
            .read_by_user("user-1")
            .await
            .expect("should read by user");
        assert_eq!(user1_entries.len(), 2);

        // Count
        let count = writer.count().await.expect("should count");
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn test_audit_jsonl_writer_empty_file() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let path = tmp.path().join("empty-audit.jsonl");

        let writer = AuditJsonlWriter::new(&path)
            .await
            .expect("should create writer");
        let entries = writer.read_all().await.expect("should read empty");
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_audit_jsonl_writer_creates_parent_dirs() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let path = tmp.path().join("nested/dir/audit.jsonl");

        let writer = AuditJsonlWriter::new(&path)
            .await
            .expect("should create writer");
        let entry = ToolAuditEntry::new("test", "test_tool", "{}", "ok", true, 0);
        writer.log(&entry).await.expect("should log to nested path");

        assert!(path.exists());
    }

    #[test]
    fn test_tool_audit_entry_truncates_long_fields() {
        let long_args = "x".repeat(1000);
        let long_result = "y".repeat(2000);

        let entry = ToolAuditEntry::new("u", "t", &long_args, &long_result, true, 0);
        assert!(entry.args_summary.len() <= 512);
        assert!(entry.args_summary.ends_with("..."));
        assert!(entry.result.len() <= 1024);
        assert!(entry.result.ends_with("..."));
    }
}
