//! Obsidian Markdown writer

use crate::{ActionLog, DocumentMatch};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use tokio::fs;
use zeus_core::{Error, Result};

/// Lexically normalise `path` by resolving `.` and `..` components without
/// touching the filesystem (safe to call for paths that don't exist yet).
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut stack: Vec<Component<'_>> = Vec::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                // Only pop a normal component; never pop prefix/root/cur-dir
                match stack.last() {
                    Some(Component::Normal(_)) => {
                        stack.pop();
                    }
                    _ => stack.push(component),
                }
            }
            Component::CurDir => {} // discard
            c => stack.push(c),
        }
    }
    stack.iter().collect()
}

/// Validate that `relative_path` resolves to a location inside `vault_path`.
/// Returns the resolved absolute path on success, or an error if the path
/// would escape the vault.
fn validate_path(vault_path: &Path, relative_path: &str) -> Result<PathBuf> {
    let candidate = vault_path.join(relative_path);
    let resolved = lexical_normalize(&candidate);
    let vault_norm = lexical_normalize(vault_path);

    if !resolved.starts_with(&vault_norm) {
        return Err(Error::Config(format!(
            "path traversal attempt: '{}' escapes vault",
            relative_path
        )));
    }
    Ok(resolved)
}

/// Sanitize a session identifier so it cannot be used to construct a path that
/// escapes the `Sessions/` directory.  Strips `/`, `\`, NUL bytes and collapses
/// any remaining `..` sequences.
pub fn sanitize_session_id(id: &str) -> String {
    let filtered: String = id
        .chars()
        .filter(|&c| c != '/' && c != '\\' && c != '\0')
        .collect();
    // Remove any .. that survived the filter above (e.g. "..foo" style)
    filtered.replace("..", "")
}

/// Statistics for a daily note
#[derive(Debug, Clone, Default)]
pub struct DailyStats {
    /// Number of sessions active on this day
    pub session_count: usize,
    /// Total messages received
    pub message_count: usize,
    /// Number of tool executions
    pub tool_executions: usize,
    /// Number of errors encountered
    pub error_count: usize,
    /// Tags/topics extracted from the day's activities
    pub tags: Vec<String>,
    /// Top tools used (tool name -> count)
    pub top_tools: Vec<(String, usize)>,
    /// Session IDs for linking
    pub session_ids: Vec<String>,
}

impl DailyStats {
    /// Create new daily stats
    pub fn new() -> Self {
        Self::default()
    }

    /// Build stats from a list of actions
    pub fn from_actions(actions: &[ActionLog]) -> Self {
        use crate::ActionType;
        use std::collections::HashSet;

        let mut stats = Self::new();
        let mut tools: HashMap<String, usize> = HashMap::new();
        let mut sessions: HashSet<String> = HashSet::new();
        let mut all_text = String::new();

        for action in actions {
            // Count sessions
            if let Some(session_id) = &action.session_id {
                sessions.insert(session_id.clone());
            }

            // Count by type
            match action.action_type {
                ActionType::MessageReceived => stats.message_count += 1,
                ActionType::ToolExecuted => {
                    stats.tool_executions += 1;
                    if let Some(tool) = &action.tool {
                        *tools.entry(tool.clone()).or_insert(0) += 1;
                    }
                }
                ActionType::Error => stats.error_count += 1,
                _ => {}
            }

            // Collect text for tag extraction
            all_text.push_str(&action.description);
            all_text.push(' ');
        }

        stats.session_count = sessions.len();
        stats.session_ids = sessions.into_iter().collect();
        stats.session_ids.sort();

        // Sort tools by count
        let mut tools_vec: Vec<_> = tools.into_iter().collect();
        tools_vec.sort_by(|a, b| b.1.cmp(&a.1));
        stats.top_tools = tools_vec;

        // Extract tags
        stats.tags = extract_simple_tags(&all_text);

        stats
    }
}

/// Extract simple tags from text
fn extract_simple_tags(text: &str) -> Vec<String> {
    use std::collections::HashSet;

    let keywords = [
        "api",
        "database",
        "test",
        "debug",
        "deploy",
        "build",
        "fix",
        "feature",
        "bug",
        "refactor",
        "documentation",
        "security",
        "performance",
        "config",
    ];

    let text_lower = text.to_lowercase();
    let mut tags: HashSet<String> = HashSet::new();

    for keyword in keywords {
        if text_lower.contains(keyword) {
            tags.insert(keyword.to_string());
        }
    }

    let mut tags_vec: Vec<_> = tags.into_iter().collect();
    tags_vec.sort();
    tags_vec.truncate(5);
    tags_vec
}

/// Generate YAML frontmatter block from key-value pairs.
///
/// Produces Obsidian-compatible frontmatter:
/// ```yaml
/// ---
/// title: "My Note"
/// date: 2026-02-18
/// tags: [api, test]
/// ---
/// ```
fn generate_frontmatter(fields: &[(&str, FrontmatterValue)]) -> String {
    let mut fm = String::from("---\n");
    for (key, value) in fields {
        match value {
            FrontmatterValue::Str(s) => {
                // Quote strings that contain special YAML chars
                if s.contains(':') || s.contains('#') || s.contains('"') || s.starts_with(' ') {
                    fm.push_str(&format!("{}: \"{}\"\n", key, s.replace('"', "\\\"")));
                } else {
                    fm.push_str(&format!("{}: {}\n", key, s));
                }
            }
            FrontmatterValue::List(items) => {
                if items.is_empty() {
                    fm.push_str(&format!("{}: []\n", key));
                } else {
                    let joined: Vec<String> = items.iter().map(|i| i.to_string()).collect();
                    fm.push_str(&format!("{}: [{}]\n", key, joined.join(", ")));
                }
            }
            FrontmatterValue::Int(n) => {
                fm.push_str(&format!("{}: {}\n", key, n));
            }
        }
    }
    fm.push_str("---\n\n");
    fm
}

/// Values that can appear in YAML frontmatter
enum FrontmatterValue<'a> {
    Str(&'a str),
    List(&'a [String]),
    Int(usize),
}

/// Obsidian vault writer
pub struct ObsidianWriter {
    vault_path: PathBuf,
}

impl ObsidianWriter {
    /// Create a new Obsidian writer
    pub fn new(vault_path: &Path) -> Result<Self> {
        Ok(Self {
            vault_path: vault_path.to_path_buf(),
        })
    }

    /// Write a document
    pub async fn write(&self, relative_path: &str, content: &str) -> Result<()> {
        let path = validate_path(&self.vault_path, relative_path)?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(Error::Io)?;
        }

        fs::write(&path, content).await.map_err(Error::Io)?;

        Ok(())
    }

    /// Append to a document
    pub async fn append(&self, relative_path: &str, content: &str) -> Result<()> {
        let path = validate_path(&self.vault_path, relative_path)?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(Error::Io)?;
        }

        // Read existing content — avoids blocking path.exists() (H3) and the
        // TOCTOU race between the existence check and the read (H2).
        let existing = match fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(Error::Io(e)),
        };

        // Append new content
        let new_content = format!("{}{}", existing, content);
        fs::write(&path, new_content).await.map_err(Error::Io)?;

        Ok(())
    }

    /// Create a daily note with YAML frontmatter
    pub async fn create_daily_note(&self, date: DateTime<Utc>) -> Result<String> {
        let filename = date.format("%Y-%m-%d").to_string();
        let path = format!("Daily/{}.md", filename);

        let title = date.format("%A, %B %d, %Y").to_string();
        let date_str = date.format("%Y-%m-%d").to_string();
        let frontmatter = generate_frontmatter(&[
            ("title", FrontmatterValue::Str(&title)),
            ("date", FrontmatterValue::Str(&date_str)),
            ("type", FrontmatterValue::Str("daily-note")),
            ("created_by", FrontmatterValue::Str("zeus-athena")),
        ]);

        let content = format!(
            r#"{}# {}

## Summary

## Actions

## Notes

---
*Generated by Zeus*
"#,
            frontmatter,
            date.format("%A, %B %d, %Y")
        );

        self.write(&path, &content).await?;
        Ok(path)
    }

    /// Create an enhanced daily note with statistics, summaries, and YAML frontmatter
    pub async fn create_enhanced_daily_note(
        &self,
        date: DateTime<Utc>,
        stats: &DailyStats,
    ) -> Result<String> {
        let filename = date.format("%Y-%m-%d").to_string();
        let path = format!("Daily/{}.md", filename);

        let title = date.format("%A, %B %d, %Y").to_string();
        let date_str = date.format("%Y-%m-%d").to_string();
        let frontmatter = generate_frontmatter(&[
            ("title", FrontmatterValue::Str(&title)),
            ("date", FrontmatterValue::Str(&date_str)),
            ("type", FrontmatterValue::Str("daily-note")),
            ("tags", FrontmatterValue::List(&stats.tags)),
            ("sessions", FrontmatterValue::Int(stats.session_count)),
            ("messages", FrontmatterValue::Int(stats.message_count)),
            (
                "tool_executions",
                FrontmatterValue::Int(stats.tool_executions),
            ),
            ("errors", FrontmatterValue::Int(stats.error_count)),
            ("created_by", FrontmatterValue::Str("zeus-athena")),
        ]);

        let mut content = String::new();

        // Frontmatter
        content.push_str(&frontmatter);

        // Header
        content.push_str(&format!("# {}\n\n", date.format("%A, %B %d, %Y")));

        // Quick stats
        content.push_str("## Daily Overview\n\n");
        content.push_str("| Metric | Count |\n");
        content.push_str("|--------|-------|\n");
        content.push_str(&format!("| Sessions | {} |\n", stats.session_count));
        content.push_str(&format!("| Messages | {} |\n", stats.message_count));
        content.push_str(&format!("| Tools Used | {} |\n", stats.tool_executions));
        if stats.error_count > 0 {
            content.push_str(&format!("| Errors | {} |\n", stats.error_count));
        }
        content.push('\n');

        // Tags/Topics
        if !stats.tags.is_empty() {
            content.push_str("## Topics\n\n");
            for tag in &stats.tags {
                content.push_str(&format!("#{} ", tag));
            }
            content.push_str("\n\n");
        }

        // Top tools
        if !stats.top_tools.is_empty() {
            content.push_str("## Most Used Tools\n\n");
            for (tool, count) in stats.top_tools.iter().take(5) {
                content.push_str(&format!("- `{}`: {} times\n", tool, count));
            }
            content.push('\n');
        }

        // Sessions section
        content.push_str("## Sessions\n\n");
        for session_id in &stats.session_ids {
            content.push_str(&format!("- [[Sessions/{}|{}]]\n", session_id, session_id));
        }
        content.push('\n');

        // Action log section (placeholder for append)
        content.push_str("## Action Log\n\n");

        // Footer
        content.push_str("---\n");
        content.push_str(&format!(
            "*Generated by Zeus at {}*\n",
            Utc::now().format("%H:%M:%S UTC")
        ));

        self.write(&path, &content).await?;
        Ok(path)
    }

    /// Update daily note with end-of-day summary
    pub async fn finalize_daily_note(&self, date: DateTime<Utc>, summary: &str) -> Result<()> {
        let filename = date.format("%Y-%m-%d").to_string();
        let path = format!("Daily/{}.md", filename);
        let full_path = self.vault_path.join(&path);

        // Read content — if the file is missing, create it first.
        // Avoids the blocking full_path.exists() call (H3) and the TOCTOU race (H2).
        let mut content = match fs::read_to_string(&full_path).await {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.create_daily_note(date).await?;
                fs::read_to_string(&full_path).await.unwrap_or_default()
            }
            Err(e) => return Err(Error::Io(e)),
        };

        // Add summary section before footer
        let footer_marker = "---\n*Generated by Zeus";
        if let Some(pos) = content.find(footer_marker) {
            let summary_section = format!("\n## End of Day Summary\n\n{}\n\n", summary);
            content.insert_str(pos, &summary_section);
        } else {
            // Just append
            content.push_str(&format!("\n## End of Day Summary\n\n{}\n", summary));
        }

        fs::write(&full_path, content).await.map_err(Error::Io)?;
        Ok(())
    }

    /// Append to daily note
    pub async fn append_to_daily(&self, action: &ActionLog) -> Result<()> {
        let filename = action.timestamp.format("%Y-%m-%d").to_string();
        let path = format!("Daily/{}.md", filename);

        let full_path = self.vault_path.join(&path);

        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).await.map_err(Error::Io)?;
        }

        // Atomically initialise the file with a header if it does not exist.
        // O_CREAT|O_EXCL eliminates the blocking exists() call (H3) and TOCTOU (H2).
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&full_path)
            .await
        {
            Ok(_) => {
                self.create_daily_note(action.timestamp).await?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(Error::Io(e)),
        }

        // Format action as markdown
        let entry = self.format_action(action);
        self.append(&path, &entry).await
    }

    /// Append to session log (creates file with YAML frontmatter if new)
    pub async fn append_to_session(&self, session_id: &str, action: &ActionLog) -> Result<()> {
        let session_id = sanitize_session_id(session_id);
        let session_id = session_id.as_str();
        let path = format!("Sessions/{}.md", session_id);

        let full_path = self.vault_path.join(&path);

        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).await.map_err(Error::Io)?;
        }

        // Atomically create session header — eliminates blocking exists() (H3) and TOCTOU (H2).
        if match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&full_path)
            .await
        {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(e) => return Err(Error::Io(e)),
        } {
            let title = format!("Session {}", session_id);
            let date_str = action.timestamp.format("%Y-%m-%d").to_string();
            let time_str = action.timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string();
            let frontmatter = generate_frontmatter(&[
                ("title", FrontmatterValue::Str(&title)),
                ("date", FrontmatterValue::Str(&date_str)),
                ("type", FrontmatterValue::Str("session-log")),
                ("session_id", FrontmatterValue::Str(session_id)),
                ("started_at", FrontmatterValue::Str(&time_str)),
                ("created_by", FrontmatterValue::Str("zeus-athena")),
            ]);

            let header = format!(
                r#"{}# Session {}

**Started**: {}

---

"#,
                frontmatter,
                session_id,
                action.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
            );
            self.write(&path, &header).await?;
        }

        let entry = self.format_action(action);
        self.append(&path, &entry).await
    }

    /// Format an action as markdown
    fn format_action(&self, action: &ActionLog) -> String {
        let mut entry = String::new();

        entry.push_str(&format!(
            "\n### {} {}\n",
            action.timestamp.format("%H:%M:%S"),
            action.action_type
        ));

        entry.push_str(&format!("{}\n", action.description));

        if let Some(tool) = &action.tool {
            entry.push_str(&format!("- **Tool**: `{}`\n", tool));
        }

        if let Some(result) = &action.result {
            entry.push_str(&format!("- **Result**: {}\n", result));
        }

        if let Some(duration) = action.duration_ms {
            entry.push_str(&format!("- **Duration**: {}ms\n", duration));
        }

        entry.push('\n');
        entry
    }

    /// List all markdown document paths in the vault (non-recursive content search).
    pub async fn list_documents(&self) -> Result<Vec<String>> {
        let mut docs = Vec::new();
        let mut stack = vec![self.vault_path.clone()];
        while let Some(dir) = stack.pop() {
            let mut entries = fs::read_dir(&dir).await.map_err(Error::Io)?;
            while let Some(entry) = entries.next_entry().await.map_err(Error::Io)? {
                let p = entry.path();
                if p.is_dir() {
                    if !p
                        .file_name()
                        .map(|n| n.to_string_lossy().starts_with('.'))
                        .unwrap_or(false)
                    {
                        stack.push(p);
                    }
                } else if p.extension().map(|e| e == "md").unwrap_or(false)
                    && let Ok(rel) = p.strip_prefix(&self.vault_path)
                {
                    docs.push(rel.to_string_lossy().to_string());
                }
            }
        }
        Ok(docs)
    }

    /// Search for documents containing query
    pub async fn search(&self, query: &str) -> Result<Vec<DocumentMatch>> {
        // Reject empty queries — scanning every file is an OOM risk (H4).
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut matches = Vec::new();
        self.search_dir(&self.vault_path, query, &mut matches)
            .await?;
        Ok(matches)
    }

    /// Recursively search directory
    async fn search_dir(
        &self,
        dir: &Path,
        query: &str,
        matches: &mut Vec<DocumentMatch>,
    ) -> Result<()> {
        let mut entries = fs::read_dir(dir).await.map_err(Error::Io)?;

        while let Some(entry) = entries.next_entry().await.map_err(Error::Io)? {
            let path = entry.path();

            if path.is_dir() {
                // Skip hidden directories
                if !path
                    .file_name()
                    .map(|n| n.to_string_lossy().starts_with('.'))
                    .unwrap_or(false)
                {
                    Box::pin(self.search_dir(&path, query, matches)).await?;
                }
            } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                // Search markdown files
                if let Ok(content) = fs::read_to_string(&path).await {
                    let query_lower = query.to_lowercase();
                    for (i, line) in content.lines().enumerate() {
                        if line.to_lowercase().contains(&query_lower) {
                            let relative_path = path
                                .strip_prefix(&self.vault_path)
                                .unwrap_or(&path)
                                .display()
                                .to_string();

                            matches.push(DocumentMatch {
                                path: relative_path,
                                context: line.to_string(),
                                line: i + 1,
                            });
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ActionType;

    fn create_test_writer() -> (ObsidianWriter, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().expect("TempDir::new should succeed");
        let writer = ObsidianWriter::new(tmp.path()).expect("ObsidianWriter::new should succeed");
        (writer, tmp)
    }

    fn create_test_action() -> ActionLog {
        ActionLog {
            timestamp: Utc::now(),
            session_id: Some("test-session".to_string()),
            action_type: ActionType::ToolExecuted,
            description: "Test action description".to_string(),
            tool: Some("test_tool".to_string()),
            result: Some("Success".to_string()),
            duration_ms: Some(42),
        }
    }

    #[tokio::test]
    async fn test_write_document() {
        let (writer, tmp) = create_test_writer();

        writer
            .write("test.md", "# Hello\n\nThis is a test.")
            .await
            .expect("should write file");

        let content = tokio::fs::read_to_string(tmp.path().join("test.md"))
            .await
            .expect("should read file");
        assert!(content.contains("# Hello"));
        assert!(content.contains("This is a test."));
    }

    #[tokio::test]
    async fn test_write_nested_document() {
        let (writer, tmp) = create_test_writer();

        writer
            .write("nested/folder/test.md", "# Nested")
            .await
            .expect("should write file");

        let content = tokio::fs::read_to_string(tmp.path().join("nested/folder/test.md"))
            .await
            .expect("should read file");
        assert!(content.contains("# Nested"));
    }

    #[tokio::test]
    async fn test_append_document() {
        let (writer, tmp) = create_test_writer();

        writer
            .write("append.md", "Line 1\n")
            .await
            .expect("should write file");
        writer
            .append("append.md", "Line 2\n")
            .await
            .expect("async operation should succeed");

        let content = tokio::fs::read_to_string(tmp.path().join("append.md"))
            .await
            .expect("should read file");
        assert!(content.contains("Line 1"));
        assert!(content.contains("Line 2"));
    }

    #[tokio::test]
    async fn test_create_daily_note() {
        let (writer, tmp) = create_test_writer();

        let date = Utc::now();
        let path = writer
            .create_daily_note(date)
            .await
            .expect("async operation should succeed");

        assert!(path.starts_with("Daily/"));
        assert!(path.ends_with(".md"));

        let content = tokio::fs::read_to_string(tmp.path().join(&path))
            .await
            .expect("should read file");
        assert!(content.contains("## Summary"));
        assert!(content.contains("## Actions"));
        assert!(content.contains("Generated by Zeus"));
    }

    #[tokio::test]
    async fn test_append_to_daily() {
        let (writer, tmp) = create_test_writer();

        let action = create_test_action();
        writer
            .append_to_daily(&action)
            .await
            .expect("async operation should succeed");

        // Check daily note was created
        let date_str = action.timestamp.format("%Y-%m-%d").to_string();
        let path = tmp.path().join(format!("Daily/{}.md", date_str));
        let content = tokio::fs::read_to_string(&path)
            .await
            .expect("should read file");

        assert!(content.contains("Test action description"));
        assert!(content.contains("test_tool"));
        assert!(content.contains("42ms"));
    }

    #[tokio::test]
    async fn test_append_to_session() {
        let (writer, tmp) = create_test_writer();

        let action = create_test_action();
        writer
            .append_to_session("test-session", &action)
            .await
            .expect("async operation should succeed");

        // Check session log was created
        let path = tmp.path().join("Sessions/test-session.md");
        let content = tokio::fs::read_to_string(&path)
            .await
            .expect("should read file");

        assert!(content.contains("# Session test-session"));
        assert!(content.contains("Test action description"));
    }

    #[tokio::test]
    async fn test_format_action() {
        let (writer, _tmp) = create_test_writer();

        let action = create_test_action();
        let formatted = writer.format_action(&action);

        assert!(formatted.contains("Test action description"));
        assert!(formatted.contains("**Tool**: `test_tool`"));
        assert!(formatted.contains("**Result**: Success"));
        assert!(formatted.contains("**Duration**: 42ms"));
    }

    #[tokio::test]
    async fn test_search() {
        let (writer, _tmp) = create_test_writer();

        // Create some documents
        writer
            .write("doc1.md", "# Document One\n\nHello world!")
            .await
            .expect("should write file");
        writer
            .write("doc2.md", "# Document Two\n\nGoodbye world!")
            .await
            .expect("should write file");
        writer
            .write("other.md", "# Other\n\nNo match here.")
            .await
            .expect("should write file");

        // Search for "world"
        let results = writer
            .search("world")
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 2);

        // Search for specific text
        let results = writer
            .search("Hello")
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
        assert!(results[0].path.contains("doc1"));
    }

    #[tokio::test]
    async fn test_search_case_insensitive() {
        let (writer, _tmp) = create_test_writer();

        writer
            .write("test.md", "Hello World")
            .await
            .expect("should write file");

        let results = writer
            .search("hello")
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);

        let results = writer
            .search("WORLD")
            .await
            .expect("async operation should succeed");
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_enhanced_daily_note() {
        let (writer, tmp) = create_test_writer();

        let stats = DailyStats {
            session_count: 2,
            message_count: 10,
            tool_executions: 5,
            error_count: 1,
            tags: vec!["api".to_string(), "test".to_string()],
            top_tools: vec![("shell".to_string(), 3), ("read_file".to_string(), 2)],
            session_ids: vec!["sess-1".to_string(), "sess-2".to_string()],
        };

        let date = Utc::now();
        let path = writer
            .create_enhanced_daily_note(date, &stats)
            .await
            .expect("async operation should succeed");

        let content = tokio::fs::read_to_string(tmp.path().join(&path))
            .await
            .expect("should read file");

        assert!(content.contains("## Daily Overview"));
        assert!(content.contains("| Sessions | 2 |"));
        assert!(content.contains("| Messages | 10 |"));
        assert!(content.contains("#api"));
        assert!(content.contains("`shell`: 3 times"));
        assert!(content.contains("[[Sessions/sess-1|sess-1]]"));
    }

    // ── Security tests ────────────────────────────────────────────────────────

    #[test]
    fn test_sanitize_session_id_clean() {
        assert_eq!(sanitize_session_id("abc-123"), "abc-123");
        assert_eq!(sanitize_session_id("sess_2026-01-01"), "sess_2026-01-01");
    }

    #[test]
    fn test_sanitize_session_id_strips_slashes() {
        // "../evil": remove '/' → "..evil", remove ".." → "evil"
        assert_eq!(sanitize_session_id("../evil"), "evil");
        assert_eq!(sanitize_session_id("foo/bar"), "foobar");
        assert_eq!(sanitize_session_id("foo\\bar"), "foobar");
    }

    #[test]
    fn test_sanitize_session_id_strips_dotdot() {
        assert_eq!(sanitize_session_id(".."), "");
        assert_eq!(sanitize_session_id("a..b"), "ab");
        assert_eq!(sanitize_session_id("...."), "");
    }

    #[test]
    fn test_sanitize_session_id_strips_null() {
        assert_eq!(sanitize_session_id("foo\0bar"), "foobar");
    }

    #[test]
    fn test_lexical_normalize_parent() {
        use std::path::PathBuf;
        let base = PathBuf::from("/vault");
        let result = lexical_normalize(&base.join("../../etc/passwd"));
        assert!(!result.starts_with(&base));
    }

    #[tokio::test]
    async fn test_write_rejects_path_traversal() {
        let (writer, _tmp) = create_test_writer();
        let err = writer
            .write("../../etc/shadow", "evil")
            .await
            .expect_err("should reject path traversal");
        let msg = err.to_string();
        assert!(
            msg.contains("path traversal") || msg.contains("escapes vault"),
            "unexpected error: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_append_rejects_path_traversal() {
        let (writer, _tmp) = create_test_writer();
        let err = writer
            .append("../outside.md", "evil")
            .await
            .expect_err("should reject path traversal");
        let msg = err.to_string();
        assert!(
            msg.contains("path traversal") || msg.contains("escapes vault"),
            "unexpected error: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_write_rejects_absolute_escape() {
        let (writer, _tmp) = create_test_writer();
        // An absolute path supplied as relative_path should also be rejected
        // because joining "/etc/passwd" to vault path yields "/etc/passwd" which
        // does not start with the vault directory.
        let err = writer
            .write("/etc/passwd", "evil")
            .await
            .expect_err("should reject absolute path escape");
        let msg = err.to_string();
        assert!(
            msg.contains("path traversal") || msg.contains("escapes vault"),
            "unexpected error: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_append_to_session_sanitizes_id() {
        let (writer, tmp) = create_test_writer();
        let action = create_test_action();

        // "../evil-session" → strip '/' → "..evil-session" → strip ".." → "evil-session"
        writer
            .append_to_session("../evil-session", &action)
            .await
            .expect("should sanitize and succeed");

        // The file must NOT land outside the vault
        let escaped = tmp.path().parent().unwrap().join("evil-session.md");
        assert!(
            !escaped.exists(),
            "file must not be written outside the vault"
        );

        // The sanitised file should exist inside the vault under Sessions/
        let safe_path = tmp.path().join("Sessions/evil-session.md");
        assert!(safe_path.exists(), "sanitised session file should exist");
    }

    #[tokio::test]
    async fn test_daily_stats_from_actions() {
        let actions = vec![
            ActionLog {
                timestamp: Utc::now(),
                session_id: Some("sess-1".to_string()),
                action_type: ActionType::MessageReceived,
                description: "Testing API endpoint".to_string(),
                tool: None,
                result: None,
                duration_ms: None,
            },
            ActionLog {
                timestamp: Utc::now(),
                session_id: Some("sess-1".to_string()),
                action_type: ActionType::ToolExecuted,
                description: "Ran shell command".to_string(),
                tool: Some("shell".to_string()),
                result: Some("success".to_string()),
                duration_ms: Some(100),
            },
        ];

        let stats = DailyStats::from_actions(&actions);

        assert_eq!(stats.session_count, 1);
        assert_eq!(stats.message_count, 1);
        assert_eq!(stats.tool_executions, 1);
        assert!(stats.tags.contains(&"api".to_string()));
    }

    #[tokio::test]
    async fn test_finalize_daily_note() {
        let (writer, tmp) = create_test_writer();

        let date = Utc::now();
        writer
            .create_daily_note(date)
            .await
            .expect("async operation should succeed");

        writer
            .finalize_daily_note(date, "Today was productive!")
            .await
            .expect("async operation should succeed");

        let filename = date.format("%Y-%m-%d").to_string();
        let content = tokio::fs::read_to_string(tmp.path().join(format!("Daily/{}.md", filename)))
            .await
            .expect("should read file");

        assert!(content.contains("## End of Day Summary"));
        assert!(content.contains("Today was productive!"));
    }

    // =========================================================================
    // Frontmatter tests
    // =========================================================================

    #[test]
    fn test_generate_frontmatter_basic() {
        let fm = generate_frontmatter(&[
            ("title", FrontmatterValue::Str("Test Note")),
            ("type", FrontmatterValue::Str("daily-note")),
        ]);
        assert!(fm.starts_with("---\n"));
        assert!(fm.ends_with("---\n\n"));
        assert!(fm.contains("title: Test Note\n"));
        assert!(fm.contains("type: daily-note\n"));
    }

    #[test]
    fn test_generate_frontmatter_with_list() {
        let tags = vec!["api".to_string(), "test".to_string()];
        let fm = generate_frontmatter(&[("tags", FrontmatterValue::List(&tags))]);
        assert!(fm.contains("tags: [api, test]\n"));
    }

    #[test]
    fn test_generate_frontmatter_empty_list() {
        let empty: Vec<String> = vec![];
        let fm = generate_frontmatter(&[("tags", FrontmatterValue::List(&empty))]);
        assert!(fm.contains("tags: []\n"));
    }

    #[test]
    fn test_generate_frontmatter_with_int() {
        let fm = generate_frontmatter(&[
            ("sessions", FrontmatterValue::Int(5)),
            ("errors", FrontmatterValue::Int(0)),
        ]);
        assert!(fm.contains("sessions: 5\n"));
        assert!(fm.contains("errors: 0\n"));
    }

    #[test]
    fn test_generate_frontmatter_special_chars() {
        let fm = generate_frontmatter(&[("title", FrontmatterValue::Str("Session: abc#123"))]);
        // Should be quoted because of special chars
        assert!(fm.contains("title: \"Session: abc#123\"\n"));
    }

    #[tokio::test]
    async fn test_daily_note_has_frontmatter() {
        let (writer, tmp) = create_test_writer();

        let date = Utc::now();
        let path = writer
            .create_daily_note(date)
            .await
            .expect("async operation should succeed");

        let content = tokio::fs::read_to_string(tmp.path().join(&path))
            .await
            .expect("should read file");

        // Verify frontmatter structure
        assert!(
            content.starts_with("---\n"),
            "should start with frontmatter delimiter"
        );
        let fm_end = content[4..]
            .find("---\n")
            .expect("should have closing delimiter");
        let frontmatter = &content[4..fm_end + 4];

        assert!(frontmatter.contains("type: daily-note"));
        assert!(frontmatter.contains("created_by: zeus-athena"));
        assert!(frontmatter.contains(&format!("date: {}", date.format("%Y-%m-%d"))));
    }

    #[tokio::test]
    async fn test_enhanced_daily_note_has_frontmatter() {
        let (writer, tmp) = create_test_writer();

        let stats = DailyStats {
            session_count: 3,
            message_count: 15,
            tool_executions: 8,
            error_count: 2,
            tags: vec!["api".to_string(), "debug".to_string()],
            top_tools: vec![("shell".to_string(), 5)],
            session_ids: vec!["s1".to_string()],
        };

        let date = Utc::now();
        let path = writer
            .create_enhanced_daily_note(date, &stats)
            .await
            .expect("async operation should succeed");

        let content = tokio::fs::read_to_string(tmp.path().join(&path))
            .await
            .expect("should read file");

        assert!(content.starts_with("---\n"));
        assert!(content.contains("type: daily-note"));
        assert!(content.contains("tags: [api, debug]"));
        assert!(content.contains("sessions: 3"));
        assert!(content.contains("messages: 15"));
        assert!(content.contains("tool_executions: 8"));
        assert!(content.contains("errors: 2"));
        assert!(content.contains("created_by: zeus-athena"));
    }

    #[tokio::test]
    async fn test_session_log_has_frontmatter() {
        let (writer, tmp) = create_test_writer();

        let action = create_test_action();
        writer
            .append_to_session("test-fm-session", &action)
            .await
            .expect("async operation should succeed");

        let path = tmp.path().join("Sessions/test-fm-session.md");
        let content = tokio::fs::read_to_string(&path)
            .await
            .expect("should read file");

        assert!(content.starts_with("---\n"));
        assert!(content.contains("type: session-log"));
        assert!(content.contains("session_id: test-fm-session"));
        assert!(content.contains("created_by: zeus-athena"));
        // Still has the body content
        assert!(content.contains("# Session test-fm-session"));
        assert!(content.contains("Test action description"));
    }

    #[tokio::test]
    async fn test_session_log_no_duplicate_frontmatter_on_append() {
        let (writer, tmp) = create_test_writer();

        let action1 = create_test_action();
        let mut action2 = create_test_action();
        action2.description = "Second action".to_string();

        writer
            .append_to_session("dup-test", &action1)
            .await
            .expect("first append");
        writer
            .append_to_session("dup-test", &action2)
            .await
            .expect("second append");

        let path = tmp.path().join("Sessions/dup-test.md");
        let content = tokio::fs::read_to_string(&path)
            .await
            .expect("should read file");

        // Should only have one frontmatter block (from initial creation)
        let delimiter_count = content.matches("---\n").count();
        // Frontmatter has opening "---\n" and closing "---\n", plus the body "---\n" separator = 3
        assert_eq!(
            delimiter_count, 3,
            "should have exactly one frontmatter block plus body separator"
        );
        // Both actions should be present
        assert!(content.contains("Test action description"));
        assert!(content.contains("Second action"));
    }
}
