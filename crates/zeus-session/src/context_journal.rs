//! Context Journal — captures structured task state before compaction
//!
//! When the agent's context window fills up, compaction summarizes old messages
//! but loses structured task state. The Context Journal captures this state as a
//! markdown file *before* compaction (no LLM call — direct extraction), then
//! injects it back after, enabling seamless work continuation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::debug;
use zeus_core::{Message, Result, Role};

use crate::context_manager::ContextManager;

/// Manages context journal files for a session
pub struct ContextJournal {
    /// Directory where journal files are stored
    journal_dir: PathBuf,
    /// Remaining context % that triggers journal capture
    threshold_pct: u8,
}

/// A single journal entry capturing workflow state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    /// When this journal was created
    pub timestamp: DateTime<Utc>,
    /// Session ID
    pub session_id: String,
    /// Estimated tokens at capture time
    pub estimated_tokens: usize,
    /// Max tokens for the context window
    pub max_tokens: usize,
    /// The active task being worked on
    pub active_task: Option<String>,
    /// Progress notes extracted from assistant messages
    pub progress: Vec<String>,
    /// Files that were modified (from write_file/edit_file tool calls)
    pub files_modified: Vec<String>,
    /// Files that were read (from read_file tool calls)
    pub files_read: Vec<String>,
    /// Tool call frequency counts
    pub tool_calls: HashMap<String, usize>,
    /// Decisions made during the session
    pub decisions: Vec<String>,
    /// Next steps identified
    pub next_steps: Vec<String>,
    /// Blockers or errors encountered
    pub blockers: Vec<String>,
}

/// Lightweight summary for listing journals
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalSummary {
    /// Filename of the journal
    pub filename: String,
    /// Session ID
    pub session_id: String,
    /// When the journal was created
    pub timestamp: DateTime<Utc>,
    /// Preview of the active task (first 80 chars)
    pub active_task_preview: Option<String>,
}

impl ContextJournal {
    /// Create a new context journal manager
    pub fn new(journal_dir: PathBuf, threshold_pct: u8) -> Self {
        Self {
            journal_dir,
            threshold_pct,
        }
    }

    /// Check if a journal should be written based on remaining context %
    pub fn needs_journal(&self, messages: &[Message], max_tokens: usize) -> bool {
        if max_tokens == 0 {
            return false;
        }
        let estimated = ContextManager::estimate_tokens(messages);
        let remaining_pct = if estimated >= max_tokens {
            0
        } else {
            ((max_tokens - estimated) * 100) / max_tokens
        };
        remaining_pct <= self.threshold_pct as usize
    }

    /// Extract a journal entry from the current message history (deterministic, no LLM)
    pub fn extract_journal(
        &self,
        session_id: &str,
        messages: &[Message],
        max_tokens: usize,
    ) -> JournalEntry {
        let estimated_tokens = ContextManager::estimate_tokens(messages);

        let active_task = Self::extract_active_task(messages);
        let progress = Self::extract_progress(messages);
        let (files_modified, files_read) = Self::extract_files(messages);
        let tool_calls = Self::extract_tool_calls(messages);
        let decisions = Self::extract_decisions(messages);
        let next_steps = Self::extract_next_steps(messages);
        let blockers = Self::extract_blockers(messages);

        JournalEntry {
            timestamp: Utc::now(),
            session_id: session_id.to_string(),
            estimated_tokens,
            max_tokens,
            active_task,
            progress,
            files_modified,
            files_read,
            tool_calls,
            decisions,
            next_steps,
            blockers,
        }
    }

    /// Render a journal entry as structured markdown
    pub fn render_markdown(entry: &JournalEntry) -> String {
        let mut md = String::new();

        md.push_str(&format!(
            "# Context Journal — {}\n\n",
            entry.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        md.push_str(&format!("**Session:** `{}`\n", entry.session_id));
        md.push_str(&format!(
            "**Tokens:** {} / {} ({:.0}% used)\n\n",
            entry.estimated_tokens,
            entry.max_tokens,
            if entry.max_tokens > 0 {
                (entry.estimated_tokens as f64 / entry.max_tokens as f64) * 100.0
            } else {
                0.0
            }
        ));

        if let Some(ref task) = entry.active_task {
            md.push_str(&format!("## Active Task\n\n{}\n\n", task));
        }

        if !entry.progress.is_empty() {
            md.push_str("## Progress\n\n");
            for item in &entry.progress {
                md.push_str(&format!("- {}\n", item));
            }
            md.push('\n');
        }

        if !entry.files_modified.is_empty() {
            md.push_str("## Files Modified\n\n");
            for f in &entry.files_modified {
                md.push_str(&format!("- `{}`\n", f));
            }
            md.push('\n');
        }

        if !entry.files_read.is_empty() {
            md.push_str("## Files Read\n\n");
            for f in &entry.files_read {
                md.push_str(&format!("- `{}`\n", f));
            }
            md.push('\n');
        }

        if !entry.tool_calls.is_empty() {
            md.push_str("## Tool Usage\n\n");
            let mut sorted: Vec<_> = entry.tool_calls.iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(a.1));
            for (name, count) in sorted {
                md.push_str(&format!("- `{}`: {} calls\n", name, count));
            }
            md.push('\n');
        }

        if !entry.decisions.is_empty() {
            md.push_str("## Decisions\n\n");
            for d in &entry.decisions {
                md.push_str(&format!("- {}\n", d));
            }
            md.push('\n');
        }

        if !entry.next_steps.is_empty() {
            md.push_str("## Next Steps\n\n");
            for s in &entry.next_steps {
                md.push_str(&format!("- {}\n", s));
            }
            md.push('\n');
        }

        if !entry.blockers.is_empty() {
            md.push_str("## Blockers\n\n");
            for b in &entry.blockers {
                md.push_str(&format!("- {}\n", b));
            }
            md.push('\n');
        }

        md
    }

    /// Write a journal file and return the path
    pub fn write_journal(
        &self,
        session_id: &str,
        messages: &[Message],
        max_tokens: usize,
    ) -> Result<PathBuf> {
        let entry = self.extract_journal(session_id, messages, max_tokens);
        let markdown = Self::render_markdown(&entry);

        std::fs::create_dir_all(&self.journal_dir)?;

        let short_id = if session_id.len() >= 8 {
            &session_id[..8]
        } else {
            session_id
        };
        let filename = format!(
            "{}-{}.md",
            entry.timestamp.format("%Y%m%d-%H%M%S"),
            short_id
        );
        let path = self.journal_dir.join(&filename);

        std::fs::write(&path, &markdown)?;
        debug!("Wrote context journal: {}", path.display());

        Ok(path)
    }

    /// Read the latest journal for a given session
    pub fn read_latest_journal(&self, session_id: &str) -> Result<Option<String>> {
        let short_id = if session_id.len() >= 8 {
            &session_id[..8]
        } else {
            session_id
        };

        if !self.journal_dir.exists() {
            return Ok(None);
        }

        let mut matching: Vec<PathBuf> = Vec::new();
        for entry in std::fs::read_dir(&self.journal_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".md") && name_str.contains(short_id) {
                matching.push(entry.path());
            }
        }

        // Sort by filename (timestamps sort lexicographically)
        matching.sort();

        match matching.last() {
            Some(path) => {
                let content = std::fs::read_to_string(path)?;
                Ok(Some(content))
            }
            None => Ok(None),
        }
    }

    /// List all journal files
    pub fn list_journals(&self) -> Result<Vec<JournalSummary>> {
        let mut summaries = Vec::new();

        if !self.journal_dir.exists() {
            return Ok(summaries);
        }

        for entry in std::fs::read_dir(&self.journal_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy().to_string();
            if !name_str.ends_with(".md") {
                continue;
            }

            if let Some(summary) = Self::parse_journal_filename(&name_str) {
                summaries.push(summary);
            }
        }

        summaries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Ok(summaries)
    }

    /// Get the journal directory path
    pub fn journal_dir(&self) -> &Path {
        &self.journal_dir
    }

    // ========================================================================
    // Private extraction helpers
    // ========================================================================

    /// Extract the active task: last substantive user message (>20 chars, not trivial)
    fn extract_active_task(messages: &[Message]) -> Option<String> {
        let trivial = [
            "continue", "yes", "ok", "y", "no", "n", "go", "done", "thanks",
        ];

        messages
            .iter()
            .rev()
            .filter(|m| m.role == Role::User)
            .filter(|m| m.content.len() > 20)
            .filter(|m| !trivial.contains(&m.content.trim().to_lowercase().as_str()))
            .map(|m| {
                let content = m.content.trim();
                if content.len() > 200 {
                    format!("{}...", zeus_core::truncate_str(content, 200))
                } else {
                    content.to_string()
                }
            })
            .next()
    }

    /// Extract progress: action-verb sentences from recent assistant messages
    fn extract_progress(messages: &[Message]) -> Vec<String> {
        let action_verbs = [
            "added",
            "created",
            "fixed",
            "updated",
            "removed",
            "implemented",
            "modified",
            "refactored",
            "completed",
            "wrote",
            "built",
            "configured",
            "installed",
            "moved",
            "renamed",
            "deleted",
        ];
        let mut progress = Vec::new();

        // Look at last 20 assistant messages
        for msg in messages
            .iter()
            .rev()
            .filter(|m| m.role == Role::Assistant)
            .take(20)
        {
            for sentence in split_sentences(&msg.content) {
                let lower = sentence.to_lowercase();
                if action_verbs.iter().any(|v| lower.contains(v)) && sentence.len() > 10 {
                    let trimmed = sentence.trim();
                    if trimmed.len() > 150 {
                        progress.push(format!("{}...", zeus_core::truncate_str(trimmed, 150)));
                    } else {
                        progress.push(trimmed.to_string());
                    }
                }
            }
        }

        progress.truncate(15);
        progress
    }

    /// Extract file paths from tool calls
    fn extract_files(messages: &[Message]) -> (Vec<String>, Vec<String>) {
        let mut modified = Vec::new();
        let mut read = Vec::new();

        for msg in messages {
            for tc in &msg.tool_calls {
                let path = tc
                    .arguments
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                match tc.name.as_str() {
                    "write_file" | "edit_file" => {
                        if let Some(p) = path
                            && !modified.contains(&p)
                        {
                            modified.push(p);
                        }
                    }
                    "read_file" => {
                        if let Some(p) = path
                            && !read.contains(&p)
                            && !modified.contains(&p)
                        {
                            read.push(p);
                        }
                    }
                    _ => {}
                }
            }
        }

        (modified, read)
    }

    /// Extract tool call frequency counts
    fn extract_tool_calls(messages: &[Message]) -> HashMap<String, usize> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for msg in messages {
            for tc in &msg.tool_calls {
                *counts.entry(tc.name.clone()).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Extract decisions: sentences containing decision-related keywords
    fn extract_decisions(messages: &[Message]) -> Vec<String> {
        let keywords = [
            "decided",
            "chose",
            "approach",
            "because",
            "instead of",
            "strategy",
        ];
        let mut decisions = Vec::new();

        for msg in messages.iter().filter(|m| m.role == Role::Assistant) {
            for sentence in split_sentences(&msg.content) {
                let lower = sentence.to_lowercase();
                if keywords.iter().any(|k| lower.contains(k)) && sentence.len() > 15 {
                    let trimmed = sentence.trim();
                    if trimmed.len() > 200 {
                        decisions.push(format!("{}...", zeus_core::truncate_str(trimmed, 200)));
                    } else {
                        decisions.push(trimmed.to_string());
                    }
                }
            }
        }

        decisions.truncate(10);
        decisions
    }

    /// Extract next steps: bulleted items or sentences with forward-looking keywords
    fn extract_next_steps(messages: &[Message]) -> Vec<String> {
        let keywords = ["next", "will", "need to", "should", "todo", "remaining"];
        let mut steps = Vec::new();

        // Focus on recent assistant messages
        for msg in messages
            .iter()
            .rev()
            .filter(|m| m.role == Role::Assistant)
            .take(5)
        {
            for line in msg.content.lines() {
                let trimmed = line.trim();
                // Bulleted or numbered items
                let is_list_item = trimmed.starts_with("- ")
                    || trimmed.starts_with("* ")
                    || (trimmed.len() > 2
                        && trimmed.chars().next().is_some_and(|c| c.is_ascii_digit())
                        && trimmed.contains(". "));

                let lower = trimmed.to_lowercase();
                let has_keyword = keywords.iter().any(|k| lower.contains(k));

                if (is_list_item || has_keyword) && trimmed.len() > 10 {
                    let clean = trimmed
                        .trim_start_matches("- ")
                        .trim_start_matches("* ")
                        .trim();
                    if clean.len() > 150 {
                        steps.push(format!("{}...", zeus_core::truncate_str(clean, 150)));
                    } else {
                        steps.push(clean.to_string());
                    }
                }
            }
        }

        steps.truncate(10);
        steps
    }

    /// Extract blockers: failed tool results + error-related sentences
    fn extract_blockers(messages: &[Message]) -> Vec<String> {
        let mut blockers = Vec::new();

        // Failed tool results
        for msg in messages {
            for tr in &msg.tool_results {
                if !tr.success {
                    let output = if tr.output.len() > 150 {
                        format!("{}...", zeus_core::truncate_str(&tr.output, 150))
                    } else {
                        tr.output.clone()
                    };
                    blockers.push(format!("Tool error: {}", output));
                }
            }
        }

        // Error-related sentences from recent assistant messages
        let error_keywords = ["error", "failed", "blocked", "cannot", "issue", "problem"];
        for msg in messages
            .iter()
            .rev()
            .filter(|m| m.role == Role::Assistant)
            .take(5)
        {
            for sentence in split_sentences(&msg.content) {
                let lower = sentence.to_lowercase();
                if error_keywords.iter().any(|k| lower.contains(k)) && sentence.len() > 15 {
                    let trimmed = sentence.trim();
                    if trimmed.len() > 200 {
                        blockers.push(format!("{}...", zeus_core::truncate_str(trimmed, 200)));
                    } else {
                        blockers.push(trimmed.to_string());
                    }
                }
            }
        }

        blockers.truncate(10);
        blockers
    }

    /// Parse a journal filename into a summary (without reading the file)
    fn parse_journal_filename(filename: &str) -> Option<JournalSummary> {
        // Format: YYYYMMDD-HHMMSS-{session_id_prefix}.md
        let stem = filename.strip_suffix(".md")?;
        let parts: Vec<&str> = stem.splitn(3, '-').collect();
        if parts.len() < 3 {
            return None;
        }

        let date_str = parts[0];
        let time_str = parts[1];
        let session_prefix = parts[2];

        let datetime_str = format!(
            "{}-{}-{}T{}:{}:{}Z",
            &date_str[..4],
            &date_str[4..6],
            &date_str[6..8],
            &time_str[..2],
            &time_str[2..4],
            &time_str[4..6],
        );

        let timestamp = datetime_str.parse::<DateTime<Utc>>().ok()?;

        Some(JournalSummary {
            filename: filename.to_string(),
            session_id: session_prefix.to_string(),
            timestamp,
            active_task_preview: None,
        })
    }
}

/// Split text into sentences (simple heuristic)
fn split_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;

    for (i, c) in text.char_indices() {
        if (c == '.' || c == '!' || c == '?') && i + 1 < text.len() {
            let next = text[i + 1..].chars().next();
            if next == Some(' ') || next == Some('\n') {
                let sentence = &text[start..=i];
                if !sentence.trim().is_empty() {
                    sentences.push(sentence.trim());
                }
                start = i + 1;
            }
        }
    }

    // Remaining text
    if start < text.len() {
        let rest = text[start..].trim();
        if !rest.is_empty() {
            sentences.push(rest);
        }
    }

    sentences
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;
    use tempfile::TempDir;
    use zeus_core::{ToolCall, ToolResult};

    fn make_journal(dir: &Path) -> ContextJournal {
        ContextJournal::new(dir.to_path_buf(), 10)
    }

    #[test]
    fn test_needs_journal_under_threshold() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let journal = make_journal(tmp.path());

        // Small messages — plenty of room remaining
        let messages = vec![Message::user("hello")];
        assert!(!journal.needs_journal(&messages, 10000));
    }

    #[test]
    fn test_needs_journal_over_threshold() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let journal = make_journal(tmp.path());

        // Fill up 95% of context (remaining 5% < threshold 10%)
        // 950 tokens = 3800 chars
        let messages = vec![Message::user("x".repeat(3800))];
        assert!(journal.needs_journal(&messages, 1000));
    }

    #[test]
    fn test_needs_journal_exactly_at_threshold() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let journal = make_journal(tmp.path());

        // 90% used = 10% remaining = exactly at threshold (should trigger)
        // 900 tokens = 3600 chars
        let messages = vec![Message::user("x".repeat(3600))];
        assert!(journal.needs_journal(&messages, 1000));
    }

    #[test]
    fn test_needs_journal_zero_max() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let journal = make_journal(tmp.path());
        let messages = vec![Message::user("hello")];
        assert!(!journal.needs_journal(&messages, 0));
    }

    #[test]
    fn test_extract_active_task() {
        let messages = vec![
            Message::user("ok"),
            Message::user("yes"),
            Message::user("Implement the context journal feature with markdown output"),
        ];
        let task = ContextJournal::extract_active_task(&messages);
        assert!(task.is_some());
        assert!(
            task.expect("operation should succeed")
                .contains("context journal")
        );
    }

    #[test]
    fn test_extract_active_task_skips_trivial() {
        let messages = vec![
            Message::user("Implement the context journal feature with markdown output"),
            Message::user("yes"),
            Message::user("continue"),
            Message::user("ok"),
        ];
        let task = ContextJournal::extract_active_task(&messages);
        assert!(task.is_some());
        assert!(
            task.expect("operation should succeed")
                .contains("context journal")
        );
    }

    #[test]
    fn test_extract_files() {
        let mut msg = Message::assistant("I'll write the file");
        msg.tool_calls = vec![
            ToolCall {
                id: "1".to_string(),
                name: "write_file".to_string(),
                arguments: serde_json::json!({"path": "/tmp/test.rs", "content": "fn main() {}"}),
            },
            ToolCall {
                id: "2".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({"path": "/tmp/other.rs"}),
            },
            ToolCall {
                id: "3".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({"path": "/tmp/test.rs"}),
            },
        ];

        let (modified, read) = ContextJournal::extract_files(&[msg]);
        assert_eq!(modified, vec!["/tmp/test.rs"]);
        // /tmp/test.rs is in modified, so it shouldn't appear in read
        assert_eq!(read, vec!["/tmp/other.rs"]);
    }

    #[test]
    fn test_extract_tool_calls() {
        let mut msg1 = Message::assistant("calling tools");
        msg1.tool_calls = vec![
            ToolCall {
                id: "1".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({}),
            },
            ToolCall {
                id: "2".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({}),
            },
            ToolCall {
                id: "3".to_string(),
                name: "shell".to_string(),
                arguments: serde_json::json!({}),
            },
        ];

        let counts = ContextJournal::extract_tool_calls(&[msg1]);
        assert_eq!(counts.get("read_file"), Some(&2));
        assert_eq!(counts.get("shell"), Some(&1));
    }

    #[test]
    fn test_extract_decisions() {
        let msg = Message::assistant(
            "I decided to use a HashMap instead of a Vec because lookup performance is important. \
             The approach of writing directly to disk avoids extra memory usage.",
        );
        let decisions = ContextJournal::extract_decisions(&[msg]);
        assert!(!decisions.is_empty());
        assert!(decisions.iter().any(|d| d.contains("decided")));
    }

    #[test]
    fn test_extract_next_steps() {
        let msg = Message::assistant(
            "Here's the plan:\n\
             - Add tests for the new module\n\
             - Wire it into the agent loop\n\
             - Run cargo clippy\n",
        );
        let steps = ContextJournal::extract_next_steps(&[msg]);
        assert!(steps.len() >= 3);
    }

    #[test]
    fn test_extract_blockers() {
        let mut msg = Message::tool("1", false, "compilation error: missing field");
        msg.tool_results = vec![ToolResult {
            call_id: "1".to_string(),
            success: false,
            output: "compilation error: missing field `context_journal`".to_string(),
        }];

        let blockers = ContextJournal::extract_blockers(&[msg]);
        assert!(!blockers.is_empty());
        assert!(blockers[0].contains("compilation error"));
    }

    #[test]
    fn test_render_markdown() {
        let entry = JournalEntry {
            timestamp: Utc::now(),
            session_id: "test-session-1234".to_string(),
            estimated_tokens: 9000,
            max_tokens: 10000,
            active_task: Some("Implement context journal".to_string()),
            progress: vec!["Added config struct".to_string()],
            files_modified: vec!["/tmp/lib.rs".to_string()],
            files_read: vec!["/tmp/other.rs".to_string()],
            tool_calls: HashMap::from([
                ("read_file".to_string(), 5),
                ("write_file".to_string(), 2),
            ]),
            decisions: vec!["Chose markdown over JSON because human readability".to_string()],
            next_steps: vec!["Add tests".to_string()],
            blockers: vec![],
        };

        let md = ContextJournal::render_markdown(&entry);
        assert!(md.contains("# Context Journal"));
        assert!(md.contains("test-session-1234"));
        assert!(md.contains("## Active Task"));
        assert!(md.contains("Implement context journal"));
        assert!(md.contains("## Progress"));
        assert!(md.contains("## Files Modified"));
        assert!(md.contains("`/tmp/lib.rs`"));
        assert!(md.contains("## Tool Usage"));
        assert!(md.contains("## Decisions"));
        assert!(md.contains("## Next Steps"));
        // No blockers section since empty
        assert!(!md.contains("## Blockers"));
    }

    #[test]
    fn test_write_and_read_journal() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let journal = make_journal(tmp.path());

        let messages = vec![
            Message::user("Implement the context journal feature with markdown output and tests"),
            Message::assistant(
                "I'll implement the context journal feature. I added the config struct.",
            ),
        ];

        let path = journal
            .write_journal("session-abcdef12-3456", &messages, 10000)
            .expect("write_journal should succeed");
        assert!(path.exists());

        // Read it back
        let content = journal
            .read_latest_journal("session-abcdef12-3456")
            .expect("read_latest_journal should succeed");
        assert!(content.is_some());
        assert!(
            content
                .expect("operation should succeed")
                .contains("# Context Journal")
        );
    }

    #[test]
    fn test_list_journals_empty() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let journal = make_journal(tmp.path());
        let list = journal
            .list_journals()
            .expect("list_journals should succeed");
        assert!(list.is_empty());
    }

    #[test]
    fn test_list_journals_with_entries() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let journal = make_journal(tmp.path());

        let messages = vec![Message::user(
            "Implement the context journal feature with tests",
        )];

        journal
            .write_journal("session-aaaa1111", &messages, 10000)
            .expect("write_journal should succeed");
        // Small delay to ensure different timestamp
        std::thread::sleep(std::time::Duration::from_millis(1100));
        journal
            .write_journal("session-bbbb2222", &messages, 10000)
            .expect("write_journal should succeed");

        let list = journal
            .list_journals()
            .expect("list_journals should succeed");
        assert_eq!(list.len(), 2);
        // Sorted newest first
        assert!(list[0].timestamp >= list[1].timestamp);
    }

    #[test]
    fn test_parse_journal_filename() {
        let summary = ContextJournal::parse_journal_filename("20260210-143022-abcdef12.md")
            .expect("should parse successfully");
        assert_eq!(summary.session_id, "abcdef12");
        assert_eq!(summary.filename, "20260210-143022-abcdef12.md");
        assert_eq!(summary.timestamp.year(), 2026);
    }

    #[test]
    fn test_parse_journal_filename_invalid() {
        assert!(ContextJournal::parse_journal_filename("invalid.md").is_none());
        assert!(ContextJournal::parse_journal_filename("not-a-journal.txt").is_none());
    }

    #[test]
    fn test_split_sentences() {
        let text = "First sentence. Second sentence! Third one? And some more text";
        let sentences = split_sentences(text);
        assert_eq!(sentences.len(), 4);
        assert_eq!(sentences[0], "First sentence.");
        assert_eq!(sentences[1], "Second sentence!");
        assert_eq!(sentences[2], "Third one?");
    }
}
