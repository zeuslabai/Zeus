//! Session Export — serialize sessions into portable formats.
//!
//! Converts session messages into different output formats for sharing,
//! archival, or integration with external tools:
//!
//! - **SessionExporter** — stateless exporter with configurable options
//! - **ExportFormat** — target format (Markdown, Json, Html, PlainText)
//! - **ExportOptions** — controls what gets included (timestamps, roles, tools, metadata)
//! - **ExportResult** — the exported content with metadata

use chrono::{DateTime, Utc};
use zeus_core::{Message, Role};

// ============================================================================
// ExportFormat
// ============================================================================

/// Target export format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Markdown with headers and code blocks.
    Markdown,
    /// Structured JSON array.
    Json,
    /// HTML document with styling.
    Html,
    /// Plain text, one message per line.
    PlainText,
}

impl ExportFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExportFormat::Markdown => "markdown",
            ExportFormat::Json => "json",
            ExportFormat::Html => "html",
            ExportFormat::PlainText => "plaintext",
        }
    }

    /// File extension for this format.
    pub fn extension(&self) -> &'static str {
        match self {
            ExportFormat::Markdown => "md",
            ExportFormat::Json => "json",
            ExportFormat::Html => "html",
            ExportFormat::PlainText => "txt",
        }
    }
}

// ============================================================================
// ExportOptions
// ============================================================================

/// Controls what content is included in the export.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// Include timestamps on each message.
    pub include_timestamps: bool,
    /// Include role labels (User, Assistant, System).
    pub include_roles: bool,
    /// Include tool call details.
    pub include_tool_calls: bool,
    /// Include system messages.
    pub include_system: bool,
    /// Optional title for the export.
    pub title: Option<String>,
    /// Maximum messages to export (0 = unlimited).
    pub max_messages: usize,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_timestamps: true,
            include_roles: true,
            include_tool_calls: true,
            include_system: false,
            title: None,
            max_messages: 0,
        }
    }
}

impl ExportOptions {
    pub fn with_title(mut self, title: &str) -> Self {
        self.title = Some(title.to_string());
        self
    }

    pub fn with_timestamps(mut self, yes: bool) -> Self {
        self.include_timestamps = yes;
        self
    }

    pub fn with_roles(mut self, yes: bool) -> Self {
        self.include_roles = yes;
        self
    }

    pub fn with_tool_calls(mut self, yes: bool) -> Self {
        self.include_tool_calls = yes;
        self
    }

    pub fn with_system(mut self, yes: bool) -> Self {
        self.include_system = yes;
        self
    }

    pub fn with_max_messages(mut self, max: usize) -> Self {
        self.max_messages = max;
        self
    }

    /// Minimal export: no timestamps, no tools, no system messages.
    pub fn minimal() -> Self {
        Self {
            include_timestamps: false,
            include_roles: true,
            include_tool_calls: false,
            include_system: false,
            title: None,
            max_messages: 0,
        }
    }
}

// ============================================================================
// ExportResult
// ============================================================================

/// The result of an export operation.
#[derive(Debug, Clone)]
pub struct ExportResult {
    /// The exported content string.
    pub content: String,
    /// Format used.
    pub format: ExportFormat,
    /// Number of messages exported.
    pub message_count: usize,
    /// When the export was generated.
    pub exported_at: DateTime<Utc>,
    /// Suggested filename.
    pub suggested_filename: String,
}

impl ExportResult {
    /// Content length in bytes.
    pub fn size_bytes(&self) -> usize {
        self.content.len()
    }
}

// ============================================================================
// SessionExporter
// ============================================================================

/// Stateless session exporter.
pub struct SessionExporter;

impl SessionExporter {
    /// Export messages to the specified format with options.
    pub fn export(
        messages: &[Message],
        format: ExportFormat,
        options: &ExportOptions,
    ) -> ExportResult {
        let filtered = Self::filter_messages(messages, options);
        let content = match format {
            ExportFormat::Markdown => Self::to_markdown(&filtered, options),
            ExportFormat::Json => Self::to_json(&filtered, options),
            ExportFormat::Html => Self::to_html(&filtered, options),
            ExportFormat::PlainText => Self::to_plaintext(&filtered, options),
        };

        let title_slug = options
            .title
            .as_deref()
            .unwrap_or("session")
            .to_lowercase()
            .replace(|c: char| !c.is_alphanumeric(), "-");
        let date = Utc::now().format("%Y%m%d");

        ExportResult {
            content,
            format,
            message_count: filtered.len(),
            exported_at: Utc::now(),
            suggested_filename: format!("{}-{}.{}", title_slug, date, format.extension()),
        }
    }

    /// Export with default options.
    pub fn export_default(messages: &[Message], format: ExportFormat) -> ExportResult {
        Self::export(messages, format, &ExportOptions::default())
    }

    // -- Filter -------------------------------------------------------------

    fn filter_messages<'a>(messages: &'a [Message], options: &ExportOptions) -> Vec<&'a Message> {
        let mut filtered: Vec<&Message> = messages
            .iter()
            .filter(|m| {
                if !options.include_system && m.role == Role::System {
                    return false;
                }
                true
            })
            .collect();

        if options.max_messages > 0 && filtered.len() > options.max_messages {
            filtered.truncate(options.max_messages);
        }

        filtered
    }

    // -- Markdown -----------------------------------------------------------

    fn to_markdown(messages: &[&Message], options: &ExportOptions) -> String {
        let mut parts: Vec<String> = Vec::new();

        if let Some(ref title) = options.title {
            parts.push(format!("# {}\n", title));
        }

        for msg in messages {
            let mut line = String::new();

            if options.include_roles {
                let role = Self::role_label(msg.role);
                line.push_str(&format!("**{}**", role));
            }

            if options.include_timestamps {
                let time_str = msg.timestamp.format("%H:%M:%S").to_string();
                line.push_str(&format!(" _{}_", time_str));
            }

            if !line.is_empty() {
                line.push('\n');
            }

            line.push_str(&msg.content);

            // Tool calls
            if options.include_tool_calls && !msg.tool_calls.is_empty() {
                for call in &msg.tool_calls {
                    line.push_str(&format!(
                        "\n\n```\n[tool: {}] {}\n```",
                        call.name, call.arguments
                    ));
                }
            }

            parts.push(line);
        }

        parts.join("\n\n---\n\n")
    }

    // -- JSON ---------------------------------------------------------------

    fn to_json(messages: &[&Message], options: &ExportOptions) -> String {
        let entries: Vec<serde_json::Value> = messages
            .iter()
            .map(|msg| {
                let mut obj = serde_json::json!({
                    "content": msg.content,
                });

                if options.include_roles {
                    obj["role"] = serde_json::json!(msg.role);
                }

                if options.include_timestamps {
                    obj["timestamp"] = serde_json::json!(msg.timestamp.to_rfc3339());
                }

                if options.include_tool_calls && !msg.tool_calls.is_empty() {
                    let call_objs: Vec<serde_json::Value> = msg
                        .tool_calls
                        .iter()
                        .map(|c| {
                            serde_json::json!({
                                "name": c.name,
                                "arguments": c.arguments,
                            })
                        })
                        .collect();
                    obj["tool_calls"] = serde_json::json!(call_objs);
                }

                obj
            })
            .collect();

        let mut root = serde_json::json!({
            "messages": entries,
            "exported_at": Utc::now().to_rfc3339(),
            "count": entries.len(),
        });

        if let Some(ref title) = options.title {
            root["title"] = serde_json::json!(title);
        }

        serde_json::to_string_pretty(&root).unwrap_or_else(|_| "{}".to_string())
    }

    // -- HTML ---------------------------------------------------------------

    fn to_html(messages: &[&Message], options: &ExportOptions) -> String {
        let mut parts: Vec<String> = vec![
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\">".to_string(),
            "<style>".to_string(),
            "body{font-family:sans-serif;max-width:800px;margin:0 auto;padding:20px}".to_string(),
            ".msg{margin:12px 0;padding:12px;border-radius:8px}".to_string(),
            ".user{background:#e3f2fd}.assistant{background:#f5f5f5}.system{background:#fff3e0}"
                .to_string(),
            ".role{font-weight:bold;margin-bottom:4px}.time{color:#888;font-size:0.85em}"
                .to_string(),
            ".tool{background:#263238;color:#aed581;padding:8px;border-radius:4px;font-family:monospace;margin-top:8px;font-size:0.9em}".to_string(),
            "</style></head><body>".to_string(),
        ];

        if let Some(ref title) = options.title {
            parts.push(format!("<h1>{}</h1>", html_escape(title)));
        }

        for msg in messages {
            let css_class = match msg.role {
                Role::User => "msg user",
                Role::Assistant => "msg assistant",
                Role::System => "msg system",
                Role::Tool => "msg tool",
            };

            parts.push(format!("<div class=\"{}\">", css_class));

            if options.include_roles {
                parts.push(format!(
                    "<div class=\"role\">{}</div>",
                    html_escape(Self::role_label(msg.role))
                ));
            }

            if options.include_timestamps {
                parts.push(format!(
                    "<span class=\"time\">{}</span>",
                    msg.timestamp.format("%H:%M:%S")
                ));
            }

            parts.push(format!("<p>{}</p>", html_escape(&msg.content)));

            if options.include_tool_calls && !msg.tool_calls.is_empty() {
                for call in &msg.tool_calls {
                    parts.push(format!(
                        "<div class=\"tool\">[{}] {}</div>",
                        html_escape(&call.name),
                        html_escape(&call.arguments.to_string())
                    ));
                }
            }

            parts.push("</div>".to_string());
        }

        parts.push("</body></html>".to_string());
        parts.join("\n")
    }

    // -- PlainText ----------------------------------------------------------

    fn to_plaintext(messages: &[&Message], options: &ExportOptions) -> String {
        let mut lines: Vec<String> = Vec::new();

        if let Some(ref title) = options.title {
            lines.push(title.clone());
            lines.push("=".repeat(title.len()));
            lines.push(String::new());
        }

        for msg in messages {
            let mut prefix = String::new();

            if options.include_timestamps {
                prefix.push_str(&format!("[{}] ", msg.timestamp.format("%H:%M:%S")));
            }

            if options.include_roles {
                prefix.push_str(&format!("{}: ", Self::role_label(msg.role)));
            }

            lines.push(format!("{}{}", prefix, msg.content));

            if options.include_tool_calls && !msg.tool_calls.is_empty() {
                for call in &msg.tool_calls {
                    lines.push(format!("  [tool: {}] {}", call.name, call.arguments));
                }
            }
        }

        lines.join("\n")
    }

    // -- Helpers ------------------------------------------------------------

    fn role_label(role: Role) -> &'static str {
        match role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::System => "System",
            Role::Tool => "Tool",
        }
    }
}

/// Basic HTML entity escaping.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use zeus_core::ToolCall;

    fn test_messages() -> Vec<Message> {
        vec![
            Message::user("Hello Zeus"),
            Message::assistant("Hello! How can I help?"),
        ]
    }

    fn messages_with_tools() -> Vec<Message> {
        let mut assistant_msg = Message::assistant("Let me list the files.");
        assistant_msg.tool_calls = vec![ToolCall {
            id: "tc-1".to_string(),
            name: "list_dir".to_string(),
            arguments: serde_json::json!({"path": "."}),
        }];

        vec![Message::user("List files"), assistant_msg]
    }

    fn messages_with_system() -> Vec<Message> {
        vec![Message::system("You are Zeus."), Message::user("Hi")]
    }

    // -- ExportFormat -------------------------------------------------------

    #[test]
    fn test_format_as_str() {
        assert_eq!(ExportFormat::Markdown.as_str(), "markdown");
        assert_eq!(ExportFormat::Json.as_str(), "json");
        assert_eq!(ExportFormat::Html.as_str(), "html");
        assert_eq!(ExportFormat::PlainText.as_str(), "plaintext");
    }

    #[test]
    fn test_format_extension() {
        assert_eq!(ExportFormat::Markdown.extension(), "md");
        assert_eq!(ExportFormat::Json.extension(), "json");
        assert_eq!(ExportFormat::Html.extension(), "html");
        assert_eq!(ExportFormat::PlainText.extension(), "txt");
    }

    // -- ExportOptions ------------------------------------------------------

    #[test]
    fn test_options_default() {
        let opts = ExportOptions::default();
        assert!(opts.include_timestamps);
        assert!(opts.include_roles);
        assert!(opts.include_tool_calls);
        assert!(!opts.include_system);
        assert!(opts.title.is_none());
        assert_eq!(opts.max_messages, 0);
    }

    #[test]
    fn test_options_minimal() {
        let opts = ExportOptions::minimal();
        assert!(!opts.include_timestamps);
        assert!(opts.include_roles);
        assert!(!opts.include_tool_calls);
    }

    #[test]
    fn test_options_builders() {
        let opts = ExportOptions::default()
            .with_title("My Session")
            .with_timestamps(false)
            .with_roles(false)
            .with_tool_calls(false)
            .with_system(true)
            .with_max_messages(10);

        assert_eq!(opts.title.as_deref(), Some("My Session"));
        assert!(!opts.include_timestamps);
        assert!(!opts.include_roles);
        assert!(!opts.include_tool_calls);
        assert!(opts.include_system);
        assert_eq!(opts.max_messages, 10);
    }

    // -- ExportResult -------------------------------------------------------

    #[test]
    fn test_export_result_size() {
        let result = ExportResult {
            content: "hello".to_string(),
            format: ExportFormat::PlainText,
            message_count: 1,
            exported_at: Utc::now(),
            suggested_filename: "test.txt".to_string(),
        };
        assert_eq!(result.size_bytes(), 5);
    }

    // -- Markdown export ----------------------------------------------------

    #[test]
    fn test_markdown_basic() {
        let msgs = test_messages();
        let result = SessionExporter::export_default(&msgs, ExportFormat::Markdown);
        assert_eq!(result.message_count, 2);
        assert_eq!(result.format, ExportFormat::Markdown);
        assert!(result.content.contains("**User**"));
        assert!(result.content.contains("**Assistant**"));
        assert!(result.content.contains("Hello Zeus"));
        assert!(result.content.contains("How can I help"));
    }

    #[test]
    fn test_markdown_with_title() {
        let msgs = test_messages();
        let opts = ExportOptions::default().with_title("Test Session");
        let result = SessionExporter::export(&msgs, ExportFormat::Markdown, &opts);
        assert!(result.content.starts_with("# Test Session"));
    }

    #[test]
    fn test_markdown_with_tools() {
        let msgs = messages_with_tools();
        let result = SessionExporter::export_default(&msgs, ExportFormat::Markdown);
        assert!(result.content.contains("list_dir"));
        assert!(result.content.contains("```"));
    }

    #[test]
    fn test_markdown_no_tools() {
        let msgs = messages_with_tools();
        let opts = ExportOptions::default().with_tool_calls(false);
        let result = SessionExporter::export(&msgs, ExportFormat::Markdown, &opts);
        assert!(!result.content.contains("list_dir"));
    }

    #[test]
    fn test_markdown_no_timestamps() {
        let msgs = test_messages();
        let opts = ExportOptions::default().with_timestamps(false);
        let result = SessionExporter::export(&msgs, ExportFormat::Markdown, &opts);
        // No italic timestamp markers — look for the pattern " _HH:" which is time
        assert!(!result.content.contains(" _0"));
        assert!(!result.content.contains(" _1"));
        assert!(!result.content.contains(" _2"));
    }

    #[test]
    fn test_markdown_separator() {
        let msgs = test_messages();
        let result = SessionExporter::export_default(&msgs, ExportFormat::Markdown);
        assert!(result.content.contains("---"));
    }

    // -- JSON export --------------------------------------------------------

    #[test]
    fn test_json_basic() {
        let msgs = test_messages();
        let result = SessionExporter::export_default(&msgs, ExportFormat::Json);
        assert_eq!(result.format, ExportFormat::Json);

        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["count"], 2);
        assert!(parsed["messages"].is_array());
        assert_eq!(parsed["messages"][0]["content"], "Hello Zeus");
    }

    #[test]
    fn test_json_with_title() {
        let msgs = test_messages();
        let opts = ExportOptions::default().with_title("My Session");
        let result = SessionExporter::export(&msgs, ExportFormat::Json, &opts);
        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["title"], "My Session");
    }

    #[test]
    fn test_json_with_tools() {
        let msgs = messages_with_tools();
        let result = SessionExporter::export_default(&msgs, ExportFormat::Json);
        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        let tool_calls = &parsed["messages"][1]["tool_calls"];
        assert!(tool_calls.is_array());
        assert_eq!(tool_calls[0]["name"], "list_dir");
    }

    #[test]
    fn test_json_no_roles() {
        let msgs = test_messages();
        let opts = ExportOptions::default().with_roles(false);
        let result = SessionExporter::export(&msgs, ExportFormat::Json, &opts);
        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert!(parsed["messages"][0].get("role").is_none());
    }

    // -- HTML export --------------------------------------------------------

    #[test]
    fn test_html_basic() {
        let msgs = test_messages();
        let result = SessionExporter::export_default(&msgs, ExportFormat::Html);
        assert_eq!(result.format, ExportFormat::Html);
        assert!(result.content.contains("<!DOCTYPE html>"));
        assert!(result.content.contains("Hello Zeus"));
        assert!(result.content.contains("class=\"msg user\""));
        assert!(result.content.contains("class=\"msg assistant\""));
    }

    #[test]
    fn test_html_with_title() {
        let msgs = test_messages();
        let opts = ExportOptions::default().with_title("HTML Session");
        let result = SessionExporter::export(&msgs, ExportFormat::Html, &opts);
        assert!(result.content.contains("<h1>HTML Session</h1>"));
    }

    #[test]
    fn test_html_escaping() {
        let msgs = vec![Message::user("<script>alert('xss')</script>")];
        let result = SessionExporter::export_default(&msgs, ExportFormat::Html);
        assert!(!result.content.contains("<script>alert"));
        assert!(result.content.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_html_with_tools() {
        let msgs = messages_with_tools();
        let result = SessionExporter::export_default(&msgs, ExportFormat::Html);
        assert!(result.content.contains("class=\"tool\""));
        assert!(result.content.contains("list_dir"));
    }

    // -- PlainText export ---------------------------------------------------

    #[test]
    fn test_plaintext_basic() {
        let msgs = test_messages();
        let result = SessionExporter::export_default(&msgs, ExportFormat::PlainText);
        assert_eq!(result.format, ExportFormat::PlainText);
        assert!(result.content.contains("User: Hello Zeus"));
        assert!(result.content.contains("Assistant: Hello! How can I help?"));
    }

    #[test]
    fn test_plaintext_with_title() {
        let msgs = test_messages();
        let opts = ExportOptions::default().with_title("My Chat");
        let result = SessionExporter::export(&msgs, ExportFormat::PlainText, &opts);
        assert!(result.content.starts_with("My Chat"));
        assert!(result.content.contains("======="));
    }

    #[test]
    fn test_plaintext_with_tools() {
        let msgs = messages_with_tools();
        let result = SessionExporter::export_default(&msgs, ExportFormat::PlainText);
        assert!(result.content.contains("[tool: list_dir]"));
    }

    #[test]
    fn test_plaintext_minimal() {
        let msgs = test_messages();
        let opts = ExportOptions::minimal();
        let result = SessionExporter::export(&msgs, ExportFormat::PlainText, &opts);
        assert!(result.content.contains("User:"));
        // No timestamp brackets in minimal mode
        assert!(!result.content.starts_with("["));
    }

    // -- Filtering ----------------------------------------------------------

    #[test]
    fn test_filter_system_messages() {
        let msgs = messages_with_system();
        let result = SessionExporter::export_default(&msgs, ExportFormat::PlainText);
        assert_eq!(result.message_count, 1);
        assert!(!result.content.contains("You are Zeus"));
    }

    #[test]
    fn test_include_system_messages() {
        let msgs = messages_with_system();
        let opts = ExportOptions::default().with_system(true);
        let result = SessionExporter::export(&msgs, ExportFormat::PlainText, &opts);
        assert_eq!(result.message_count, 2);
        assert!(result.content.contains("You are Zeus"));
    }

    #[test]
    fn test_max_messages() {
        let msgs = test_messages();
        let opts = ExportOptions::default().with_max_messages(1);
        let result = SessionExporter::export(&msgs, ExportFormat::PlainText, &opts);
        assert_eq!(result.message_count, 1);
    }

    // -- Empty export -------------------------------------------------------

    #[test]
    fn test_export_empty() {
        let result = SessionExporter::export_default(&[], ExportFormat::Markdown);
        assert_eq!(result.message_count, 0);
        assert!(result.content.is_empty());
    }

    #[test]
    fn test_export_empty_json() {
        let result = SessionExporter::export_default(&[], ExportFormat::Json);
        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["count"], 0);
        assert!(parsed["messages"].as_array().unwrap().is_empty());
    }

    // -- Suggested filename -------------------------------------------------

    #[test]
    fn test_suggested_filename_default() {
        let result = SessionExporter::export_default(&[], ExportFormat::Markdown);
        assert!(result.suggested_filename.starts_with("session-"));
        assert!(result.suggested_filename.ends_with(".md"));
    }

    #[test]
    fn test_suggested_filename_with_title() {
        let opts = ExportOptions::default().with_title("My Chat Session");
        let result = SessionExporter::export(&[], ExportFormat::Json, &opts);
        assert!(result.suggested_filename.starts_with("my-chat-session-"));
        assert!(result.suggested_filename.ends_with(".json"));
    }

    // -- Role labels --------------------------------------------------------

    #[test]
    fn test_role_labels() {
        assert_eq!(SessionExporter::role_label(Role::User), "User");
        assert_eq!(SessionExporter::role_label(Role::Assistant), "Assistant");
        assert_eq!(SessionExporter::role_label(Role::System), "System");
        assert_eq!(SessionExporter::role_label(Role::Tool), "Tool");
    }

    // -- html_escape --------------------------------------------------------

    #[test]
    fn test_html_escape_entities() {
        assert_eq!(html_escape("<b>bold</b>"), "&lt;b&gt;bold&lt;/b&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("\"quoted\""), "&quot;quoted&quot;");
    }

    #[test]
    fn test_html_escape_safe_text() {
        assert_eq!(html_escape("hello world"), "hello world");
    }
}
