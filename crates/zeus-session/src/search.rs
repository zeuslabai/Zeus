//! Full-text search across JSONL session files.
//!
//! Provides [`SessionSearcher`] which scans all `.jsonl` session files in a
//! directory, parses message entries, and performs substring or regex matching
//! on message content.  Results are returned sorted by relevance (exact match
//! density, then recency).

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use tracing::debug;
use zeus_core::{Error, Message, Result, Role};

// ============================================================================
// Types
// ============================================================================

/// Parameters for a session search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    /// The search string (plain substring or regex pattern).
    pub query: String,
    /// When `true`, matching is case-sensitive.  Defaults to `false`.
    #[serde(default)]
    pub case_sensitive: bool,
    /// Maximum number of results to return.  `None` means unlimited.
    #[serde(default)]
    pub max_results: Option<usize>,
    /// Restrict search to these session IDs.  `None` means all sessions.
    #[serde(default)]
    pub session_ids: Option<Vec<String>>,
    /// Only include messages whose role matches one of these strings
    /// (e.g. `["user", "assistant"]`).  `None` means all roles.
    #[serde(default)]
    pub role_filter: Option<Vec<String>>,
}

/// A single search hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The session this match was found in.
    pub session_id: String,
    /// Zero-based index of the message within the session.
    pub message_index: usize,
    /// Role of the matched message (lowercase string).
    pub role: String,
    /// Snippet of the content surrounding the match, truncated to a
    /// reasonable length.
    pub content_snippet: String,
    /// Byte offset of the first match within `content`.
    pub match_offset: usize,
    /// Timestamp of the message, if available.
    pub timestamp: Option<DateTime<Utc>>,
}

// ============================================================================
// SessionSearcher
// ============================================================================

/// Searches JSONL session files for messages matching a query.
pub struct SessionSearcher {
    sessions_dir: PathBuf,
}

/// Internal session entry mirroring the JSONL schema used by `Session`.
#[derive(Debug, Deserialize)]
struct SessionEntry {
    #[serde(rename = "type")]
    entry_type: String,
    #[serde(flatten)]
    data: serde_json::Value,
}

impl SessionSearcher {
    /// Create a new searcher rooted at `sessions_dir`.
    pub fn new(sessions_dir: impl Into<PathBuf>) -> Self {
        Self {
            sessions_dir: sessions_dir.into(),
        }
    }

    /// Execute a search across all (or filtered) sessions.
    ///
    /// Returns results sorted by relevance: messages with more match density
    /// are ranked higher, with ties broken by recency (newest first).
    pub async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        if query.query.is_empty() {
            return Ok(Vec::new());
        }

        let pattern = self.build_pattern(&query.query, query.case_sensitive)?;

        let dir = &self.sessions_dir;
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        let mut entries = fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }

            let session_id = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };

            // If session_ids filter is set, skip sessions not in the list.
            if let Some(ref ids) = query.session_ids
                && !ids.iter().any(|id| id == &session_id)
            {
                continue;
            }

            // Quick pre-filter: read the raw text and skip the file entirely
            // if the query substring does not appear anywhere.
            let content = match fs::read_to_string(&path).await {
                Ok(c) => c,
                Err(e) => {
                    debug!("Skipping unreadable session file {}: {}", session_id, e);
                    continue;
                }
            };

            if !Self::quick_match(&content, &query.query, query.case_sensitive) {
                continue;
            }

            // Parse line-by-line.
            let mut msg_index: usize = 0;
            for line in content.lines() {
                if line.trim().is_empty() {
                    continue;
                }

                let entry: SessionEntry = match serde_json::from_str(line) {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                if entry.entry_type != "message" {
                    continue;
                }

                let msg: Message = match serde_json::from_value(entry.data) {
                    Ok(m) => m,
                    Err(_) => {
                        msg_index += 1;
                        continue;
                    }
                };

                // Role filter
                let role_str = role_to_string(msg.role);
                if let Some(ref roles) = query.role_filter
                    && !roles.iter().any(|r| r.eq_ignore_ascii_case(&role_str))
                {
                    msg_index += 1;
                    continue;
                }

                // Match against content
                if let Some(mat) = pattern.find(&msg.content) {
                    let snippet = build_snippet(&msg.content, mat.start(), 120);

                    results.push(SearchResult {
                        session_id: session_id.clone(),
                        message_index: msg_index,
                        role: role_str,
                        content_snippet: snippet,
                        match_offset: mat.start(),
                        timestamp: Some(msg.timestamp),
                    });
                }

                msg_index += 1;
            }
        }

        // Sort by relevance: shorter content (more concentrated match) first,
        // then by recency.
        results.sort_by(|a, b| {
            let len_a = a.content_snippet.len();
            let len_b = b.content_snippet.len();
            len_a.cmp(&len_b).then_with(|| {
                let ts_b = b.timestamp.unwrap_or(DateTime::UNIX_EPOCH);
                let ts_a = a.timestamp.unwrap_or(DateTime::UNIX_EPOCH);
                ts_b.cmp(&ts_a)
            })
        });

        // Apply max_results limit.
        if let Some(max) = query.max_results {
            results.truncate(max);
        }

        Ok(results)
    }

    // -- helpers -------------------------------------------------------------

    /// Build a compiled regex from the query string.
    fn build_pattern(&self, query: &str, case_sensitive: bool) -> Result<Regex> {
        let escaped = regex::escape(query);
        let pattern = if case_sensitive {
            escaped
        } else {
            format!("(?i){}", escaped)
        };
        Regex::new(&pattern).map_err(|e| Error::Session(format!("Invalid search pattern: {}", e)))
    }

    /// Fast pre-check: does the raw file text contain the query at all?
    fn quick_match(haystack: &str, needle: &str, case_sensitive: bool) -> bool {
        if case_sensitive {
            haystack.contains(needle)
        } else {
            haystack.to_lowercase().contains(&needle.to_lowercase())
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Convert a `Role` enum to its lowercase string representation.
fn role_to_string(role: Role) -> String {
    match role {
        Role::User => "user".to_string(),
        Role::Assistant => "assistant".to_string(),
        Role::System => "system".to_string(),
        Role::Tool => "tool".to_string(),
    }
}

/// Build a snippet of `max_len` characters centered around `offset`.
fn build_snippet(content: &str, offset: usize, max_len: usize) -> String {
    if content.len() <= max_len {
        return content.to_string();
    }

    let half = max_len / 2;
    let start = offset.saturating_sub(half);
    let end = (start + max_len).min(content.len());
    // Adjust start if end is clamped.
    let start = if end - start < max_len {
        end.saturating_sub(max_len)
    } else {
        start
    };

    // Snap to char boundaries.
    let start = content
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| i >= start)
        .unwrap_or(start);
    let end = content
        .char_indices()
        .map(|(i, c)| i + c.len_utf8())
        .rfind(|&i| i <= end)
        .unwrap_or(end);

    let mut snippet = String::new();
    if start > 0 {
        snippet.push_str("...");
    }
    snippet.push_str(&content[start..end]);
    if end < content.len() {
        snippet.push_str("...");
    }
    snippet
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use zeus_core::Message;

    /// Helper: create a minimal .jsonl session file with given messages.
    async fn create_test_session(dir: &std::path::Path, session_id: &str, messages: Vec<Message>) {
        let path = dir.join(format!("{}.jsonl", session_id));
        let mut lines = Vec::new();

        // session_start entry
        lines.push(
            serde_json::to_string(&serde_json::json!({
                "type": "session_start",
                "id": session_id,
                "created": Utc::now().to_rfc3339()
            }))
            .expect("operation should succeed"),
        );

        for msg in messages {
            let val = serde_json::to_value(&msg).expect("should serialize to JSON");
            let mut entry = serde_json::Map::new();
            entry.insert(
                "type".to_string(),
                serde_json::Value::String("message".to_string()),
            );
            for (k, v) in val.as_object().expect("should be an object") {
                entry.insert(k.clone(), v.clone());
            }
            lines.push(serde_json::to_string(&entry).expect("should serialize to JSON"));
        }

        fs::write(&path, lines.join("\n") + "\n")
            .await
            .expect("should write file");
    }

    #[tokio::test]
    async fn test_basic_search() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        create_test_session(
            tmp.path(),
            "s1",
            vec![
                Message::user("Hello world"),
                Message::assistant("Hi there, how can I help?"),
            ],
        )
        .await;

        let searcher = SessionSearcher::new(tmp.path());
        let results = searcher
            .search(&SearchQuery {
                query: "Hello".to_string(),
                case_sensitive: false,
                max_results: None,
                session_ids: None,
                role_filter: None,
            })
            .await
            .expect("async operation should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "s1");
        assert_eq!(results[0].role, "user");
        assert_eq!(results[0].message_index, 0);
    }

    #[tokio::test]
    async fn test_case_insensitive() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        create_test_session(tmp.path(), "s1", vec![Message::user("HELLO WORLD")]).await;

        let searcher = SessionSearcher::new(tmp.path());

        // Case-insensitive should match
        let results = searcher
            .search(&SearchQuery {
                query: "hello".to_string(),
                case_sensitive: false,
                max_results: None,
                session_ids: None,
                role_filter: None,
            })
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);

        // Case-sensitive should NOT match
        let results = searcher
            .search(&SearchQuery {
                query: "hello".to_string(),
                case_sensitive: true,
                max_results: None,
                session_ids: None,
                role_filter: None,
            })
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn test_role_filter() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        create_test_session(
            tmp.path(),
            "s1",
            vec![
                Message::user("the quick brown fox"),
                Message::assistant("the quick brown fox"),
            ],
        )
        .await;

        let searcher = SessionSearcher::new(tmp.path());
        let results = searcher
            .search(&SearchQuery {
                query: "fox".to_string(),
                case_sensitive: false,
                max_results: None,
                session_ids: None,
                role_filter: Some(vec!["assistant".to_string()]),
            })
            .await
            .expect("async operation should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].role, "assistant");
    }

    #[tokio::test]
    async fn test_session_ids_filter() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        create_test_session(tmp.path(), "s1", vec![Message::user("needle in s1")]).await;
        create_test_session(tmp.path(), "s2", vec![Message::user("needle in s2")]).await;

        let searcher = SessionSearcher::new(tmp.path());
        let results = searcher
            .search(&SearchQuery {
                query: "needle".to_string(),
                case_sensitive: false,
                max_results: None,
                session_ids: Some(vec!["s1".to_string()]),
                role_filter: None,
            })
            .await
            .expect("async operation should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "s1");
    }

    #[tokio::test]
    async fn test_max_results() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        create_test_session(
            tmp.path(),
            "s1",
            vec![
                Message::user("match one"),
                Message::user("match two"),
                Message::user("match three"),
            ],
        )
        .await;

        let searcher = SessionSearcher::new(tmp.path());
        let results = searcher
            .search(&SearchQuery {
                query: "match".to_string(),
                case_sensitive: false,
                max_results: Some(2),
                session_ids: None,
                role_filter: None,
            })
            .await
            .expect("async operation should succeed");

        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_empty_query() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        create_test_session(tmp.path(), "s1", vec![Message::user("anything")]).await;

        let searcher = SessionSearcher::new(tmp.path());
        let results = searcher
            .search(&SearchQuery {
                query: String::new(),
                case_sensitive: false,
                max_results: None,
                session_ids: None,
                role_filter: None,
            })
            .await
            .expect("async operation should succeed");

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_no_sessions_dir() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let non_existent = tmp.path().join("nope");

        let searcher = SessionSearcher::new(&non_existent);
        let results = searcher
            .search(&SearchQuery {
                query: "anything".to_string(),
                case_sensitive: false,
                max_results: None,
                session_ids: None,
                role_filter: None,
            })
            .await
            .expect("async operation should succeed");

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_multiple_sessions() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        create_test_session(
            tmp.path(),
            "alpha",
            vec![
                Message::user("Rust is great"),
                Message::assistant("Indeed, Rust is wonderful"),
            ],
        )
        .await;
        create_test_session(
            tmp.path(),
            "beta",
            vec![
                Message::user("Python is nice"),
                Message::assistant("Rust can be nice too"),
            ],
        )
        .await;

        let searcher = SessionSearcher::new(tmp.path());
        let results = searcher
            .search(&SearchQuery {
                query: "Rust".to_string(),
                case_sensitive: true,
                max_results: None,
                session_ids: None,
                role_filter: None,
            })
            .await
            .expect("async operation should succeed");

        // "Rust" appears in: alpha/user, alpha/assistant, beta/assistant
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_snippet_truncation() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let long_content = format!("{} needle {}", "a".repeat(200), "b".repeat(200));
        create_test_session(tmp.path(), "s1", vec![Message::user(&long_content)]).await;

        let searcher = SessionSearcher::new(tmp.path());
        let results = searcher
            .search(&SearchQuery {
                query: "needle".to_string(),
                case_sensitive: false,
                max_results: None,
                session_ids: None,
                role_filter: None,
            })
            .await
            .expect("async operation should succeed");

        assert_eq!(results.len(), 1);
        assert!(results[0].content_snippet.contains("needle"));
        assert!(results[0].content_snippet.len() < long_content.len());
    }

    #[tokio::test]
    async fn test_match_offset() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        create_test_session(
            tmp.path(),
            "s1",
            vec![Message::user("prefix_TARGET_suffix")],
        )
        .await;

        let searcher = SessionSearcher::new(tmp.path());
        let results = searcher
            .search(&SearchQuery {
                query: "TARGET".to_string(),
                case_sensitive: true,
                max_results: None,
                session_ids: None,
                role_filter: None,
            })
            .await
            .expect("async operation should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].match_offset, 7); // "prefix_" is 7 bytes
    }

    #[tokio::test]
    async fn test_skips_non_jsonl_files() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        create_test_session(tmp.path(), "real", vec![Message::user("findme")]).await;
        // Write a non-jsonl file that contains the query
        fs::write(tmp.path().join("notes.txt"), "findme")
            .await
            .expect("should write file");

        let searcher = SessionSearcher::new(tmp.path());
        let results = searcher
            .search(&SearchQuery {
                query: "findme".to_string(),
                case_sensitive: false,
                max_results: None,
                session_ids: None,
                role_filter: None,
            })
            .await
            .expect("async operation should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "real");
    }
}
