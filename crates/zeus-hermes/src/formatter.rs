//! Notification Message Formatter
//!
//! Produces styled notification messages for different output targets:
//!
//! - **NotificationFormatter** — format notifications into styled messages
//! - **Template** — reusable message templates with variable substitution
//! - **OutputFormat** — target format (Markdown, PlainText, Html)
//! - **MessageSection** — composable message sections (header, body, footer, fields)

use std::collections::HashMap;

use chrono::{DateTime, Utc};

// ============================================================================
// Output format
// ============================================================================

/// Target output format for formatted messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputFormat {
    /// Markdown (Telegram, Discord, Slack)
    Markdown,
    /// Plain text (console, SMS, logs)
    PlainText,
    /// HTML (email, web)
    Html,
}

impl OutputFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            OutputFormat::Markdown => "markdown",
            OutputFormat::PlainText => "plaintext",
            OutputFormat::Html => "html",
        }
    }
}

// ============================================================================
// Message section
// ============================================================================

/// A composable section of a notification message.
#[derive(Debug, Clone)]
pub enum MessageSection {
    /// Title/header line
    Header(String),
    /// Body text paragraph
    Body(String),
    /// Key-value fields (label, value)
    Fields(Vec<(String, String)>),
    /// A divider/separator
    Divider,
    /// Footer text (smaller/muted)
    Footer(String),
    /// Code block with optional language
    CodeBlock {
        code: String,
        language: Option<String>,
    },
    /// Quoted text
    Quote(String),
    /// Bulleted list
    List(Vec<String>),
}

// ============================================================================
// Template
// ============================================================================

/// A reusable notification template with variable placeholders.
///
/// Variables use `{{name}}` syntax and are substituted at render time.
#[derive(Debug, Clone)]
pub struct Template {
    /// Template name for identification.
    pub name: String,
    /// Ordered sections composing the template.
    pub sections: Vec<TemplateSection>,
}

/// A section in a template (may contain `{{variable}}` placeholders).
#[derive(Debug, Clone)]
pub enum TemplateSection {
    Header(String),
    Body(String),
    Fields(Vec<(String, String)>),
    Divider,
    Footer(String),
    CodeBlock {
        code: String,
        language: Option<String>,
    },
    Quote(String),
    List(Vec<String>),
}

impl Template {
    /// Create a new empty template.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            sections: Vec::new(),
        }
    }

    /// Add a header section.
    pub fn header(mut self, text: &str) -> Self {
        self.sections
            .push(TemplateSection::Header(text.to_string()));
        self
    }

    /// Add a body section.
    pub fn body(mut self, text: &str) -> Self {
        self.sections.push(TemplateSection::Body(text.to_string()));
        self
    }

    /// Add key-value fields.
    pub fn fields(mut self, fields: Vec<(&str, &str)>) -> Self {
        self.sections.push(TemplateSection::Fields(
            fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ));
        self
    }

    /// Add a divider.
    pub fn divider(mut self) -> Self {
        self.sections.push(TemplateSection::Divider);
        self
    }

    /// Add a footer.
    pub fn footer(mut self, text: &str) -> Self {
        self.sections
            .push(TemplateSection::Footer(text.to_string()));
        self
    }

    /// Add a code block.
    pub fn code_block(mut self, code: &str, language: Option<&str>) -> Self {
        self.sections.push(TemplateSection::CodeBlock {
            code: code.to_string(),
            language: language.map(|s| s.to_string()),
        });
        self
    }

    /// Add a quoted block.
    pub fn quote(mut self, text: &str) -> Self {
        self.sections.push(TemplateSection::Quote(text.to_string()));
        self
    }

    /// Add a bulleted list.
    pub fn list(mut self, items: Vec<&str>) -> Self {
        self.sections.push(TemplateSection::List(
            items.into_iter().map(|s| s.to_string()).collect(),
        ));
        self
    }

    /// Render the template with variable substitution into MessageSections.
    pub fn render(&self, vars: &HashMap<String, String>) -> Vec<MessageSection> {
        self.sections
            .iter()
            .map(|s| match s {
                TemplateSection::Header(t) => MessageSection::Header(substitute(t, vars)),
                TemplateSection::Body(t) => MessageSection::Body(substitute(t, vars)),
                TemplateSection::Fields(fields) => MessageSection::Fields(
                    fields
                        .iter()
                        .map(|(k, v)| (substitute(k, vars), substitute(v, vars)))
                        .collect(),
                ),
                TemplateSection::Divider => MessageSection::Divider,
                TemplateSection::Footer(t) => MessageSection::Footer(substitute(t, vars)),
                TemplateSection::CodeBlock { code, language } => MessageSection::CodeBlock {
                    code: substitute(code, vars),
                    language: language.clone(),
                },
                TemplateSection::Quote(t) => MessageSection::Quote(substitute(t, vars)),
                TemplateSection::List(items) => {
                    MessageSection::List(items.iter().map(|i| substitute(i, vars)).collect())
                }
            })
            .collect()
    }
}

/// Substitute `{{key}}` placeholders in text with values from vars map.
fn substitute(text: &str, vars: &HashMap<String, String>) -> String {
    let mut result = text.to_string();
    for (key, value) in vars {
        let placeholder = format!("{{{{{}}}}}", key);
        result = result.replace(&placeholder, value);
    }
    result
}

// ============================================================================
// NotificationFormatter
// ============================================================================

/// Formats notification messages for different output targets.
pub struct NotificationFormatter {
    templates: HashMap<String, Template>,
    default_format: OutputFormat,
}

impl NotificationFormatter {
    /// Create a new formatter with built-in templates.
    pub fn new() -> Self {
        let mut f = Self {
            templates: HashMap::new(),
            default_format: OutputFormat::Markdown,
        };
        f.register_builtin_templates();
        f
    }

    /// Create with a specific default output format.
    pub fn with_format(format: OutputFormat) -> Self {
        let mut f = Self {
            templates: HashMap::new(),
            default_format: format,
        };
        f.register_builtin_templates();
        f
    }

    /// Register a custom template.
    pub fn register_template(&mut self, template: Template) {
        self.templates.insert(template.name.clone(), template);
    }

    /// Get a template by name.
    pub fn get_template(&self, name: &str) -> Option<&Template> {
        self.templates.get(name)
    }

    /// List registered template names.
    pub fn template_names(&self) -> Vec<&str> {
        self.templates.keys().map(|s| s.as_str()).collect()
    }

    /// Format sections into the default output format.
    pub fn format(&self, sections: &[MessageSection]) -> String {
        self.format_as(sections, self.default_format)
    }

    /// Format sections into a specific output format.
    pub fn format_as(&self, sections: &[MessageSection], format: OutputFormat) -> String {
        let mut parts: Vec<String> = Vec::new();

        for section in sections {
            match (section, format) {
                // -- Markdown -----------------------------------------------
                (MessageSection::Header(text), OutputFormat::Markdown) => {
                    parts.push(format!("**{}**", text));
                }
                (MessageSection::Body(text), OutputFormat::Markdown) => {
                    parts.push(text.clone());
                }
                (MessageSection::Fields(fields), OutputFormat::Markdown) => {
                    for (k, v) in fields {
                        parts.push(format!("- **{}**: {}", k, v));
                    }
                }
                (MessageSection::Divider, OutputFormat::Markdown) => {
                    parts.push("---".to_string());
                }
                (MessageSection::Footer(text), OutputFormat::Markdown) => {
                    parts.push(format!("_{}_", text));
                }
                (MessageSection::CodeBlock { code, language }, OutputFormat::Markdown) => {
                    let lang = language.as_deref().unwrap_or("");
                    parts.push(format!("```{}\n{}\n```", lang, code));
                }
                (MessageSection::Quote(text), OutputFormat::Markdown) => {
                    for line in text.lines() {
                        parts.push(format!("> {}", line));
                    }
                }
                (MessageSection::List(items), OutputFormat::Markdown) => {
                    for item in items {
                        parts.push(format!("- {}", item));
                    }
                }

                // -- PlainText ----------------------------------------------
                (MessageSection::Header(text), OutputFormat::PlainText) => {
                    parts.push(text.to_uppercase());
                }
                (MessageSection::Body(text), OutputFormat::PlainText) => {
                    parts.push(text.clone());
                }
                (MessageSection::Fields(fields), OutputFormat::PlainText) => {
                    for (k, v) in fields {
                        parts.push(format!("  {}: {}", k, v));
                    }
                }
                (MessageSection::Divider, OutputFormat::PlainText) => {
                    parts.push("────────────────────────".to_string());
                }
                (MessageSection::Footer(text), OutputFormat::PlainText) => {
                    parts.push(format!("-- {}", text));
                }
                (MessageSection::CodeBlock { code, .. }, OutputFormat::PlainText) => {
                    parts.push(code.clone());
                }
                (MessageSection::Quote(text), OutputFormat::PlainText) => {
                    for line in text.lines() {
                        parts.push(format!("| {}", line));
                    }
                }
                (MessageSection::List(items), OutputFormat::PlainText) => {
                    for item in items {
                        parts.push(format!("* {}", item));
                    }
                }

                // -- HTML ---------------------------------------------------
                (MessageSection::Header(text), OutputFormat::Html) => {
                    parts.push(format!("<h3>{}</h3>", html_escape(text)));
                }
                (MessageSection::Body(text), OutputFormat::Html) => {
                    parts.push(format!("<p>{}</p>", html_escape(text)));
                }
                (MessageSection::Fields(fields), OutputFormat::Html) => {
                    parts.push("<dl>".to_string());
                    for (k, v) in fields {
                        parts.push(format!(
                            "<dt>{}</dt><dd>{}</dd>",
                            html_escape(k),
                            html_escape(v)
                        ));
                    }
                    parts.push("</dl>".to_string());
                }
                (MessageSection::Divider, OutputFormat::Html) => {
                    parts.push("<hr/>".to_string());
                }
                (MessageSection::Footer(text), OutputFormat::Html) => {
                    parts.push(format!(
                        "<footer><small>{}</small></footer>",
                        html_escape(text)
                    ));
                }
                (MessageSection::CodeBlock { code, language }, OutputFormat::Html) => {
                    let cls = language
                        .as_ref()
                        .map(|l| format!(" class=\"language-{}\"", l))
                        .unwrap_or_default();
                    parts.push(format!(
                        "<pre><code{}>{}</code></pre>",
                        cls,
                        html_escape(code)
                    ));
                }
                (MessageSection::Quote(text), OutputFormat::Html) => {
                    parts.push(format!("<blockquote>{}</blockquote>", html_escape(text)));
                }
                (MessageSection::List(items), OutputFormat::Html) => {
                    parts.push("<ul>".to_string());
                    for item in items {
                        parts.push(format!("<li>{}</li>", html_escape(item)));
                    }
                    parts.push("</ul>".to_string());
                }
            }
        }

        parts.join("\n")
    }

    /// Format an error notification.
    pub fn format_error(&self, error: &str, context: Option<&str>) -> String {
        let mut vars = HashMap::new();
        vars.insert("error".to_string(), error.to_string());
        vars.insert(
            "context".to_string(),
            context.unwrap_or("unknown").to_string(),
        );
        vars.insert("timestamp".to_string(), Utc::now().to_rfc3339());

        if let Some(tpl) = self.templates.get("error") {
            let sections = tpl.render(&vars);
            self.format(&sections)
        } else {
            format!("[ERROR] {}: {}", context.unwrap_or(""), error)
        }
    }

    /// Format a task completion notification.
    pub fn format_task_complete(&self, task_name: &str, duration_secs: u64) -> String {
        let mut vars = HashMap::new();
        vars.insert("task".to_string(), task_name.to_string());
        vars.insert("duration".to_string(), format_duration(duration_secs));
        vars.insert("timestamp".to_string(), Utc::now().to_rfc3339());

        if let Some(tpl) = self.templates.get("task_complete") {
            let sections = tpl.render(&vars);
            self.format(&sections)
        } else {
            format!("[DONE] {} ({})", task_name, format_duration(duration_secs))
        }
    }

    /// Format a daily digest notification.
    pub fn format_digest(&self, items: &[DigestItem]) -> String {
        let mut vars = HashMap::new();
        vars.insert("count".to_string(), items.len().to_string());
        vars.insert(
            "date".to_string(),
            Utc::now().format("%Y-%m-%d").to_string(),
        );

        let item_lines: Vec<String> = items
            .iter()
            .map(|item| format!("{} ({})", item.summary, item.category))
            .collect();
        vars.insert("items".to_string(), item_lines.join(", "));

        if let Some(tpl) = self.templates.get("digest") {
            let mut sections = tpl.render(&vars);
            // Replace the list placeholder with actual items
            sections.push(MessageSection::List(item_lines));
            self.format(&sections)
        } else {
            let lines: Vec<String> = items
                .iter()
                .map(|i| format!("- {} [{}]", i.summary, i.category))
                .collect();
            format!("Daily Digest ({})\n{}", items.len(), lines.join("\n"))
        }
    }

    /// Format a system alert notification.
    pub fn format_alert(&self, severity: AlertSeverity, title: &str, details: &str) -> String {
        let mut vars = HashMap::new();
        vars.insert("severity".to_string(), severity.as_str().to_string());
        vars.insert("icon".to_string(), severity.icon().to_string());
        vars.insert("title".to_string(), title.to_string());
        vars.insert("details".to_string(), details.to_string());
        vars.insert("timestamp".to_string(), Utc::now().to_rfc3339());

        if let Some(tpl) = self.templates.get("alert") {
            let sections = tpl.render(&vars);
            self.format(&sections)
        } else {
            format!("[{}] {}: {}", severity.as_str(), title, details)
        }
    }

    /// Format a raw message with timestamp.
    pub fn format_timestamped(&self, message: &str, timestamp: DateTime<Utc>) -> String {
        let ts = timestamp.format("%H:%M:%S").to_string();
        match self.default_format {
            OutputFormat::Markdown => format!("**[{}]** {}", ts, message),
            OutputFormat::PlainText => format!("[{}] {}", ts, message),
            OutputFormat::Html => {
                format!("<time>{}</time> {}", ts, html_escape(message))
            }
        }
    }

    // -- Built-in templates -------------------------------------------------

    fn register_builtin_templates(&mut self) {
        // Error template
        self.templates.insert(
            "error".to_string(),
            Template::new("error")
                .header("Error: {{context}}")
                .body("{{error}}")
                .divider()
                .footer("{{timestamp}}"),
        );

        // Task completion template
        self.templates.insert(
            "task_complete".to_string(),
            Template::new("task_complete")
                .header("Task Complete")
                .fields(vec![("Task", "{{task}}"), ("Duration", "{{duration}}")])
                .footer("{{timestamp}}"),
        );

        // Digest template
        self.templates.insert(
            "digest".to_string(),
            Template::new("digest")
                .header("Daily Digest — {{date}}")
                .body("{{count}} items today"),
        );

        // Alert template
        self.templates.insert(
            "alert".to_string(),
            Template::new("alert")
                .header("{{icon}} {{severity}}: {{title}}")
                .body("{{details}}")
                .footer("{{timestamp}}"),
        );
    }
}

impl Default for NotificationFormatter {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Digest item
// ============================================================================

/// An item in a daily digest notification.
#[derive(Debug, Clone)]
pub struct DigestItem {
    pub summary: String,
    pub category: String,
    pub timestamp: DateTime<Utc>,
}

impl DigestItem {
    pub fn new(summary: &str, category: &str) -> Self {
        Self {
            summary: summary.to_string(),
            category: category.to_string(),
            timestamp: Utc::now(),
        }
    }
}

// ============================================================================
// Alert severity
// ============================================================================

/// Severity level for system alerts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

impl AlertSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            AlertSeverity::Info => "INFO",
            AlertSeverity::Warning => "WARNING",
            AlertSeverity::Critical => "CRITICAL",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            AlertSeverity::Info => "ℹ️",
            AlertSeverity::Warning => "⚠️",
            AlertSeverity::Critical => "🚨",
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Format seconds into a human-readable duration string.
fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
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

    // -- OutputFormat -------------------------------------------------------

    #[test]
    fn test_output_format_as_str() {
        assert_eq!(OutputFormat::Markdown.as_str(), "markdown");
        assert_eq!(OutputFormat::PlainText.as_str(), "plaintext");
        assert_eq!(OutputFormat::Html.as_str(), "html");
    }

    #[test]
    fn test_output_format_equality() {
        assert_eq!(OutputFormat::Markdown, OutputFormat::Markdown);
        assert_ne!(OutputFormat::Markdown, OutputFormat::PlainText);
    }

    // -- MessageSection format tests ----------------------------------------

    #[test]
    fn test_format_header_markdown() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::Header("Hello".to_string())];
        let result = f.format_as(&sections, OutputFormat::Markdown);
        assert_eq!(result, "**Hello**");
    }

    #[test]
    fn test_format_header_plaintext() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::Header("Hello".to_string())];
        let result = f.format_as(&sections, OutputFormat::PlainText);
        assert_eq!(result, "HELLO");
    }

    #[test]
    fn test_format_header_html() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::Header("Hello".to_string())];
        let result = f.format_as(&sections, OutputFormat::Html);
        assert_eq!(result, "<h3>Hello</h3>");
    }

    #[test]
    fn test_format_body() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::Body("Some text".to_string())];
        assert_eq!(f.format_as(&sections, OutputFormat::Markdown), "Some text");
        assert_eq!(f.format_as(&sections, OutputFormat::PlainText), "Some text");
        assert_eq!(
            f.format_as(&sections, OutputFormat::Html),
            "<p>Some text</p>"
        );
    }

    #[test]
    fn test_format_fields_markdown() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::Fields(vec![
            ("Status".to_string(), "OK".to_string()),
            ("Count".to_string(), "42".to_string()),
        ])];
        let result = f.format_as(&sections, OutputFormat::Markdown);
        assert!(result.contains("- **Status**: OK"));
        assert!(result.contains("- **Count**: 42"));
    }

    #[test]
    fn test_format_fields_plaintext() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::Fields(vec![(
            "Key".to_string(),
            "Value".to_string(),
        )])];
        let result = f.format_as(&sections, OutputFormat::PlainText);
        assert_eq!(result, "  Key: Value");
    }

    #[test]
    fn test_format_fields_html() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::Fields(vec![(
            "Name".to_string(),
            "Zeus".to_string(),
        )])];
        let result = f.format_as(&sections, OutputFormat::Html);
        assert!(result.contains("<dt>Name</dt>"));
        assert!(result.contains("<dd>Zeus</dd>"));
    }

    #[test]
    fn test_format_divider() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::Divider];
        assert_eq!(f.format_as(&sections, OutputFormat::Markdown), "---");
        assert!(
            f.format_as(&sections, OutputFormat::PlainText)
                .contains("────")
        );
        assert_eq!(f.format_as(&sections, OutputFormat::Html), "<hr/>");
    }

    #[test]
    fn test_format_footer_markdown() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::Footer("Zeus v1.0".to_string())];
        assert_eq!(
            f.format_as(&sections, OutputFormat::Markdown),
            "_Zeus v1.0_"
        );
    }

    #[test]
    fn test_format_footer_plaintext() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::Footer("Zeus v1.0".to_string())];
        assert_eq!(
            f.format_as(&sections, OutputFormat::PlainText),
            "-- Zeus v1.0"
        );
    }

    #[test]
    fn test_format_code_block_markdown() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::CodeBlock {
            code: "let x = 1;".to_string(),
            language: Some("rust".to_string()),
        }];
        let result = f.format_as(&sections, OutputFormat::Markdown);
        assert!(result.contains("```rust"));
        assert!(result.contains("let x = 1;"));
        assert!(result.contains("```"));
    }

    #[test]
    fn test_format_code_block_plaintext() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::CodeBlock {
            code: "echo hi".to_string(),
            language: None,
        }];
        let result = f.format_as(&sections, OutputFormat::PlainText);
        assert_eq!(result, "echo hi");
    }

    #[test]
    fn test_format_code_block_html() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::CodeBlock {
            code: "x < 1".to_string(),
            language: Some("rust".to_string()),
        }];
        let result = f.format_as(&sections, OutputFormat::Html);
        assert!(result.contains("class=\"language-rust\""));
        assert!(result.contains("x &lt; 1")); // HTML escaped
    }

    #[test]
    fn test_format_quote_markdown() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::Quote("wise words\nsecond line".to_string())];
        let result = f.format_as(&sections, OutputFormat::Markdown);
        assert!(result.contains("> wise words"));
        assert!(result.contains("> second line"));
    }

    #[test]
    fn test_format_quote_plaintext() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::Quote("text".to_string())];
        assert_eq!(f.format_as(&sections, OutputFormat::PlainText), "| text");
    }

    #[test]
    fn test_format_list_markdown() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::List(vec!["one".into(), "two".into()])];
        let result = f.format_as(&sections, OutputFormat::Markdown);
        assert!(result.contains("- one"));
        assert!(result.contains("- two"));
    }

    #[test]
    fn test_format_list_plaintext() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::List(vec!["item".into()])];
        assert_eq!(f.format_as(&sections, OutputFormat::PlainText), "* item");
    }

    #[test]
    fn test_format_list_html() {
        let f = NotificationFormatter::new();
        let sections = vec![MessageSection::List(vec!["alpha".into(), "beta".into()])];
        let result = f.format_as(&sections, OutputFormat::Html);
        assert!(result.contains("<ul>"));
        assert!(result.contains("<li>alpha</li>"));
        assert!(result.contains("<li>beta</li>"));
        assert!(result.contains("</ul>"));
    }

    // -- Multi-section composition ------------------------------------------

    #[test]
    fn test_format_multi_section() {
        let f = NotificationFormatter::new();
        let sections = vec![
            MessageSection::Header("Alert".to_string()),
            MessageSection::Body("Something happened".to_string()),
            MessageSection::Divider,
            MessageSection::Footer("Zeus".to_string()),
        ];
        let result = f.format(&sections);
        assert!(result.contains("**Alert**"));
        assert!(result.contains("Something happened"));
        assert!(result.contains("---"));
        assert!(result.contains("_Zeus_"));
    }

    // -- Template -----------------------------------------------------------

    #[test]
    fn test_template_render_substitution() {
        let tpl = Template::new("test")
            .header("Hello {{name}}")
            .body("You have {{count}} items");

        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Alice".to_string());
        vars.insert("count".to_string(), "5".to_string());

        let sections = tpl.render(&vars);
        let f = NotificationFormatter::new();
        let result = f.format(&sections);
        assert!(result.contains("Alice"));
        assert!(result.contains("5 items"));
    }

    #[test]
    fn test_template_render_missing_var() {
        let tpl = Template::new("test").header("Hello {{name}}");
        let vars = HashMap::new(); // no variables
        let sections = tpl.render(&vars);
        let f = NotificationFormatter::new();
        let result = f.format(&sections);
        assert!(result.contains("{{name}}")); // unresolved placeholder remains
    }

    #[test]
    fn test_template_fields_substitution() {
        let tpl = Template::new("test").fields(vec![("Status", "{{status}}")]);
        let mut vars = HashMap::new();
        vars.insert("status".to_string(), "running".to_string());

        let sections = tpl.render(&vars);
        let f = NotificationFormatter::new();
        let result = f.format(&sections);
        assert!(result.contains("running"));
    }

    #[test]
    fn test_template_code_block() {
        let tpl = Template::new("test").code_block("{{code}}", Some("bash"));
        let mut vars = HashMap::new();
        vars.insert("code".to_string(), "echo hello".to_string());

        let sections = tpl.render(&vars);
        let f = NotificationFormatter::new();
        let result = f.format(&sections);
        assert!(result.contains("echo hello"));
        assert!(result.contains("```bash"));
    }

    #[test]
    fn test_template_list() {
        let tpl = Template::new("test").list(vec!["{{item1}}", "{{item2}}"]);
        let mut vars = HashMap::new();
        vars.insert("item1".to_string(), "first".to_string());
        vars.insert("item2".to_string(), "second".to_string());

        let sections = tpl.render(&vars);
        let f = NotificationFormatter::new();
        let result = f.format(&sections);
        assert!(result.contains("- first"));
        assert!(result.contains("- second"));
    }

    // -- Built-in formatters ------------------------------------------------

    #[test]
    fn test_format_error() {
        let f = NotificationFormatter::new();
        let result = f.format_error("connection timeout", Some("database"));
        assert!(result.contains("database"));
        assert!(result.contains("connection timeout"));
    }

    #[test]
    fn test_format_error_no_context() {
        let f = NotificationFormatter::new();
        let result = f.format_error("crash", None);
        assert!(result.contains("unknown"));
        assert!(result.contains("crash"));
    }

    #[test]
    fn test_format_task_complete() {
        let f = NotificationFormatter::new();
        let result = f.format_task_complete("Build Zeus", 125);
        assert!(result.contains("Build Zeus"));
        assert!(result.contains("2m 5s"));
    }

    #[test]
    fn test_format_digest() {
        let f = NotificationFormatter::new();
        let items = vec![
            DigestItem::new("Deployed v2.0", "release"),
            DigestItem::new("Fixed bug #42", "bugfix"),
        ];
        let result = f.format_digest(&items);
        assert!(result.contains("2 items"));
        assert!(result.contains("Deployed v2.0"));
        assert!(result.contains("Fixed bug #42"));
    }

    #[test]
    fn test_format_digest_empty() {
        let f = NotificationFormatter::new();
        let result = f.format_digest(&[]);
        assert!(result.contains("0 items"));
    }

    #[test]
    fn test_format_alert_info() {
        let f = NotificationFormatter::new();
        let result = f.format_alert(AlertSeverity::Info, "Update", "New version available");
        assert!(result.contains("INFO"));
        assert!(result.contains("Update"));
        assert!(result.contains("New version available"));
    }

    #[test]
    fn test_format_alert_critical() {
        let f = NotificationFormatter::new();
        let result = f.format_alert(AlertSeverity::Critical, "Disk Full", "< 1% free");
        assert!(result.contains("CRITICAL"));
        assert!(result.contains("Disk Full"));
    }

    #[test]
    fn test_format_timestamped_markdown() {
        let f = NotificationFormatter::new();
        let ts = Utc::now();
        let result = f.format_timestamped("hello", ts);
        assert!(result.starts_with("**["));
        assert!(result.contains("hello"));
    }

    #[test]
    fn test_format_timestamped_plaintext() {
        let f = NotificationFormatter::with_format(OutputFormat::PlainText);
        let ts = Utc::now();
        let result = f.format_timestamped("hello", ts);
        assert!(result.starts_with("["));
        assert!(result.contains("hello"));
    }

    #[test]
    fn test_format_timestamped_html() {
        let f = NotificationFormatter::with_format(OutputFormat::Html);
        let ts = Utc::now();
        let result = f.format_timestamped("hello", ts);
        assert!(result.contains("<time>"));
    }

    // -- Custom template registration ---------------------------------------

    #[test]
    fn test_register_custom_template() {
        let mut f = NotificationFormatter::new();
        let tpl = Template::new("deploy")
            .header("Deployed {{version}}")
            .body("Env: {{env}}");
        f.register_template(tpl);
        assert!(f.get_template("deploy").is_some());
        assert!(f.template_names().contains(&"deploy"));
    }

    #[test]
    fn test_builtin_templates_exist() {
        let f = NotificationFormatter::new();
        assert!(f.get_template("error").is_some());
        assert!(f.get_template("task_complete").is_some());
        assert!(f.get_template("digest").is_some());
        assert!(f.get_template("alert").is_some());
    }

    // -- AlertSeverity ------------------------------------------------------

    #[test]
    fn test_alert_severity_as_str() {
        assert_eq!(AlertSeverity::Info.as_str(), "INFO");
        assert_eq!(AlertSeverity::Warning.as_str(), "WARNING");
        assert_eq!(AlertSeverity::Critical.as_str(), "CRITICAL");
    }

    #[test]
    fn test_alert_severity_icon() {
        assert_eq!(AlertSeverity::Info.icon(), "ℹ️");
        assert_eq!(AlertSeverity::Warning.icon(), "⚠️");
        assert_eq!(AlertSeverity::Critical.icon(), "🚨");
    }

    // -- Helper functions ---------------------------------------------------

    #[test]
    fn test_format_duration_seconds() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(30), "30s");
        assert_eq!(format_duration(59), "59s");
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_duration(60), "1m 0s");
        assert_eq!(format_duration(125), "2m 5s");
        assert_eq!(format_duration(3599), "59m 59s");
    }

    #[test]
    fn test_format_duration_hours() {
        assert_eq!(format_duration(3600), "1h 0m");
        assert_eq!(format_duration(7260), "2h 1m");
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(html_escape("safe text"), "safe text");
    }

    #[test]
    fn test_substitute_basic() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Zeus".to_string());
        assert_eq!(substitute("Hello {{name}}", &vars), "Hello Zeus");
    }

    #[test]
    fn test_substitute_multiple() {
        let mut vars = HashMap::new();
        vars.insert("a".to_string(), "1".to_string());
        vars.insert("b".to_string(), "2".to_string());
        assert_eq!(substitute("{{a}} + {{b}}", &vars), "1 + 2");
    }

    #[test]
    fn test_substitute_missing_unchanged() {
        let vars = HashMap::new();
        assert_eq!(substitute("{{missing}}", &vars), "{{missing}}");
    }

    // -- DigestItem ---------------------------------------------------------

    #[test]
    fn test_digest_item_new() {
        let item = DigestItem::new("Test", "category");
        assert_eq!(item.summary, "Test");
        assert_eq!(item.category, "category");
    }

    // -- Default format -----------------------------------------------------

    #[test]
    fn test_with_format() {
        let f = NotificationFormatter::with_format(OutputFormat::PlainText);
        let sections = vec![MessageSection::Header("Test".to_string())];
        let result = f.format(&sections);
        assert_eq!(result, "TEST"); // plaintext uppercases headers
    }
}
