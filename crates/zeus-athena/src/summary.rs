//! Session summarization for Athena

use crate::{ActionLog, ActionType};
use chrono::{DateTime, Duration, Utc};
use std::collections::{HashMap, HashSet};

/// Summary of a session's activities
#[derive(Debug, Clone)]
pub struct SessionSummary {
    /// Session ID
    pub session_id: String,
    /// When the session started
    pub started_at: DateTime<Utc>,
    /// When the session ended (or last activity)
    pub ended_at: DateTime<Utc>,
    /// Total duration
    pub duration: Duration,
    /// Number of messages received
    pub messages_received: usize,
    /// Number of tools executed
    pub tools_executed: usize,
    /// Number of errors
    pub errors: usize,
    /// Tools used with counts
    pub tools_used: HashMap<String, usize>,
    /// Extracted tags/topics
    pub tags: Vec<String>,
    /// Key outcomes/results
    pub outcomes: Vec<String>,
    /// Brief summary text
    pub summary_text: String,
}

impl SessionSummary {
    /// Generate a summary from a list of actions
    pub fn from_actions(session_id: &str, actions: &[ActionLog]) -> Self {
        let mut messages_received = 0;
        let mut tools_executed = 0;
        let mut errors = 0;
        let mut tools_used: HashMap<String, usize> = HashMap::new();
        let mut outcomes: Vec<String> = Vec::new();
        let mut all_text = String::new();

        let started_at = actions
            .first()
            .map(|a| a.timestamp)
            .unwrap_or_else(Utc::now);
        let ended_at = actions.last().map(|a| a.timestamp).unwrap_or_else(Utc::now);

        for action in actions {
            // Count by type
            match action.action_type {
                ActionType::MessageReceived => messages_received += 1,
                ActionType::ToolExecuted => {
                    tools_executed += 1;
                    if let Some(tool) = &action.tool {
                        *tools_used.entry(tool.clone()).or_insert(0) += 1;
                    }
                }
                ActionType::Error => errors += 1,
                ActionType::ResponseSent => {
                    if let Some(result) = &action.result
                        && !result.is_empty()
                        && result.len() < 200
                    {
                        outcomes.push(result.clone());
                    }
                }
                _ => {}
            }

            // Collect text for tag extraction
            all_text.push_str(&action.description);
            all_text.push(' ');
            if let Some(result) = &action.result {
                all_text.push_str(result);
                all_text.push(' ');
            }
        }

        // Extract tags
        let tags = extract_tags(&all_text);

        // Generate summary text
        let summary_text = generate_summary_text(
            messages_received,
            tools_executed,
            errors,
            &tools_used,
            &outcomes,
        );

        Self {
            session_id: session_id.to_string(),
            started_at,
            ended_at,
            duration: ended_at - started_at,
            messages_received,
            tools_executed,
            errors,
            tools_used,
            tags,
            outcomes,
            summary_text,
        }
    }

    /// Format as Markdown with YAML frontmatter
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        // YAML frontmatter
        md.push_str("---\n");
        md.push_str(&format!("title: Session Summary {}\n", self.session_id));
        md.push_str(&format!("date: {}\n", self.started_at.format("%Y-%m-%d")));
        md.push_str("type: session-summary\n");
        md.push_str(&format!("session_id: {}\n", self.session_id));
        md.push_str(&format!(
            "started_at: {}\n",
            self.started_at.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        md.push_str(&format!(
            "ended_at: {}\n",
            self.ended_at.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        md.push_str(&format!(
            "duration_minutes: {}\n",
            self.duration.num_minutes()
        ));
        md.push_str(&format!("messages: {}\n", self.messages_received));
        md.push_str(&format!("tools_executed: {}\n", self.tools_executed));
        md.push_str(&format!("errors: {}\n", self.errors));
        if !self.tags.is_empty() {
            md.push_str(&format!("tags: [{}]\n", self.tags.join(", ")));
        } else {
            md.push_str("tags: []\n");
        }
        md.push_str("created_by: zeus-athena\n");
        md.push_str("---\n\n");

        md.push_str(&format!("## Session Summary: {}\n\n", self.session_id));

        // Metadata
        md.push_str(&format!(
            "**Duration**: {} minutes\n",
            self.duration.num_minutes()
        ));
        md.push_str(&format!(
            "**Period**: {} to {}\n\n",
            self.started_at.format("%H:%M:%S"),
            self.ended_at.format("%H:%M:%S")
        ));

        // Statistics
        md.push_str("### Statistics\n\n");
        md.push_str(&format!("- Messages: {}\n", self.messages_received));
        md.push_str(&format!("- Tools executed: {}\n", self.tools_executed));
        if self.errors > 0 {
            md.push_str(&format!("- Errors: {}\n", self.errors));
        }
        md.push('\n');

        // Tools used
        if !self.tools_used.is_empty() {
            md.push_str("### Tools Used\n\n");
            let mut tools: Vec<_> = self.tools_used.iter().collect();
            tools.sort_by(|a, b| b.1.cmp(a.1)); // Sort by count descending

            for (tool, count) in tools {
                md.push_str(&format!("- `{}`: {} times\n", tool, count));
            }
            md.push('\n');
        }

        // Tags
        if !self.tags.is_empty() {
            md.push_str("### Topics\n\n");
            for tag in &self.tags {
                md.push_str(&format!("#{}  ", tag));
            }
            md.push_str("\n\n");
        }

        // Outcomes
        if !self.outcomes.is_empty() {
            md.push_str("### Key Outcomes\n\n");
            for outcome in self.outcomes.iter().take(5) {
                md.push_str(&format!("- {}\n", outcome));
            }
            md.push('\n');
        }

        // Summary
        if !self.summary_text.is_empty() {
            md.push_str("### Summary\n\n");
            md.push_str(&self.summary_text);
            md.push_str("\n\n");
        }

        md
    }
}

/// Extract tags/topics from text using simple heuristics
fn extract_tags(text: &str) -> Vec<String> {
    let mut tags = HashSet::new();

    // Common development/task keywords to look for
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
        "setup",
        "install",
        "update",
        "search",
        "file",
        "code",
        "review",
        "analysis",
        "research",
        "planning",
        "design",
        "implementation",
    ];

    let text_lower = text.to_lowercase();

    for keyword in keywords {
        if text_lower.contains(keyword) {
            tags.insert(keyword.to_string());
        }
    }

    // Limit to top 5 tags
    let mut tags_vec: Vec<_> = tags.into_iter().collect();
    tags_vec.sort();
    tags_vec.truncate(5);
    tags_vec
}

/// Generate a brief summary text
fn generate_summary_text(
    messages: usize,
    tools: usize,
    errors: usize,
    tools_used: &HashMap<String, usize>,
    outcomes: &[String],
) -> String {
    let mut summary = String::new();

    // Activity overview
    if messages > 0 || tools > 0 {
        summary.push_str(&format!(
            "Session processed {} message{} and executed {} tool{}. ",
            messages,
            if messages == 1 { "" } else { "s" },
            tools,
            if tools == 1 { "" } else { "s" }
        ));
    }

    // Error status
    if errors > 0 {
        summary.push_str(&format!(
            "Encountered {} error{}. ",
            errors,
            if errors == 1 { "" } else { "s" }
        ));
    }

    // Top tools
    if !tools_used.is_empty() {
        let mut tools_sorted: Vec<_> = tools_used.iter().collect();
        tools_sorted.sort_by(|a, b| b.1.cmp(a.1));

        if let Some((top_tool, _)) = tools_sorted.first() {
            summary.push_str(&format!("Most used tool: `{}`. ", top_tool));
        }
    }

    // Outcomes hint
    if !outcomes.is_empty() {
        summary.push_str(&format!(
            "Completed {} key task{}.",
            outcomes.len(),
            if outcomes.len() == 1 { "" } else { "s" }
        ));
    }

    summary
}

/// Cross-reference link generator
#[derive(Debug)]
pub struct CrossReferenceLinker {
    /// Known document paths
    documents: HashSet<String>,
}

impl CrossReferenceLinker {
    /// Create a new linker
    pub fn new() -> Self {
        Self {
            documents: HashSet::new(),
        }
    }

    /// Add a known document
    pub fn add_document(&mut self, path: String) {
        self.documents.insert(path);
    }

    /// Add multiple documents
    pub fn add_documents(&mut self, paths: impl IntoIterator<Item = String>) {
        self.documents.extend(paths);
    }

    /// Generate Obsidian-style links in text
    ///
    /// Replaces mentions of document names with [[links]]
    pub fn linkify(&self, text: &str) -> String {
        let mut result = text.to_string();

        for doc_path in &self.documents {
            // Extract filename without extension
            let filename = std::path::Path::new(doc_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(doc_path);

            // Look for mentions of the document name (case-insensitive)
            let pattern = format!(r"\b{}\b", regex_escape(filename));
            if let Ok(re) = regex::Regex::new(&pattern) {
                // Only linkify if not already a link
                let check = format!("[[{}", filename);
                if !result.contains(&check) {
                    result = re
                        .replace_all(&result, &format!("[[{}]]", filename))
                        .to_string();
                }
            }
        }

        result
    }
}

impl Default for CrossReferenceLinker {
    fn default() -> Self {
        Self::new()
    }
}

/// Escape special regex characters
fn regex_escape(s: &str) -> String {
    let special = [
        '\\', '.', '+', '*', '?', '(', ')', '[', ']', '{', '}', '|', '^', '$',
    ];
    let mut escaped = String::with_capacity(s.len() * 2);

    for c in s.chars() {
        if special.contains(&c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }

    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_action(action_type: ActionType, description: &str) -> ActionLog {
        ActionLog {
            timestamp: Utc::now(),
            session_id: Some("test-session".to_string()),
            action_type,
            description: description.to_string(),
            tool: None,
            result: None,
            duration_ms: None,
        }
    }

    #[test]
    fn test_session_summary_from_actions() {
        let actions = vec![
            create_test_action(ActionType::MessageReceived, "User asked about API"),
            {
                let mut a = create_test_action(ActionType::ToolExecuted, "Searched files");
                a.tool = Some("search".to_string());
                a
            },
            {
                let mut a = create_test_action(ActionType::ToolExecuted, "Read file");
                a.tool = Some("read_file".to_string());
                a
            },
            {
                let mut a = create_test_action(ActionType::ResponseSent, "Explained API usage");
                a.result = Some("Successfully explained the API".to_string());
                a
            },
        ];

        let summary = SessionSummary::from_actions("test-session", &actions);

        assert_eq!(summary.session_id, "test-session");
        assert_eq!(summary.messages_received, 1);
        assert_eq!(summary.tools_executed, 2);
        assert_eq!(summary.errors, 0);
        assert!(summary.tools_used.contains_key("search"));
        assert!(summary.tools_used.contains_key("read_file"));
    }

    #[test]
    fn test_session_summary_to_markdown() {
        let actions = vec![
            create_test_action(ActionType::MessageReceived, "Testing"),
            {
                let mut a = create_test_action(ActionType::ToolExecuted, "Shell command");
                a.tool = Some("shell".to_string());
                a
            },
        ];

        let summary = SessionSummary::from_actions("test", &actions);
        let md = summary.to_markdown();

        assert!(md.contains("## Session Summary: test"));
        assert!(md.contains("Messages: 1"));
        assert!(md.contains("Tools executed: 1"));
        assert!(md.contains("`shell`"));
    }

    #[test]
    fn test_extract_tags() {
        let text = "Working on API testing and debugging the database connection";
        let tags = extract_tags(text);

        assert!(tags.contains(&"api".to_string()));
        assert!(tags.contains(&"test".to_string()));
        assert!(tags.contains(&"debug".to_string()));
        assert!(tags.contains(&"database".to_string()));
    }

    #[test]
    fn test_cross_reference_linker() {
        let mut linker = CrossReferenceLinker::new();
        linker.add_document("Architecture.md".to_string());
        linker.add_document("Components/zeus-core.md".to_string());

        let text = "See the Architecture document for details about zeus-core";
        let linked = linker.linkify(text);

        assert!(linked.contains("[[Architecture]]"));
        assert!(linked.contains("[[zeus-core]]"));
    }

    #[test]
    fn test_cross_reference_linker_no_double_link() {
        let mut linker = CrossReferenceLinker::new();
        linker.add_document("Architecture.md".to_string());

        let text = "See [[Architecture]] for details about Architecture";
        let linked = linker.linkify(text);

        // Should only have one [[Architecture]], not double-linked
        assert_eq!(linked.matches("[[Architecture]]").count(), 1);
    }

    #[test]
    fn test_session_summary_markdown_has_frontmatter() {
        let actions = vec![
            create_test_action(ActionType::MessageReceived, "User asked about API"),
            {
                let mut a = create_test_action(ActionType::ToolExecuted, "Searched files");
                a.tool = Some("search".to_string());
                a
            },
        ];

        let summary = SessionSummary::from_actions("fm-test", &actions);
        let md = summary.to_markdown();

        // Verify frontmatter
        assert!(md.starts_with("---\n"), "should start with frontmatter");
        assert!(md.contains("type: session-summary"));
        assert!(md.contains("session_id: fm-test"));
        assert!(md.contains("messages: 1"));
        assert!(md.contains("tools_executed: 1"));
        assert!(md.contains("created_by: zeus-athena"));
        // Tags should include api and search
        assert!(md.contains("tags: ["));
        // Body should still exist after frontmatter
        assert!(md.contains("## Session Summary: fm-test"));
    }

    #[test]
    fn test_session_summary_frontmatter_has_dates() {
        let actions = vec![create_test_action(ActionType::MessageReceived, "test")];
        let summary = SessionSummary::from_actions("date-test", &actions);
        let md = summary.to_markdown();

        assert!(md.contains("started_at:"));
        assert!(md.contains("ended_at:"));
        assert!(md.contains("duration_minutes:"));
        assert!(md.contains(&format!("date: {}", Utc::now().format("%Y-%m-%d"))));
    }
}
