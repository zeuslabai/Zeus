// ═══════════════════════════════════════════════════════════
// ZEUS — Diff Viewer Component — Unified diff preview for approvals
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;

/// Escape HTML entities to prevent XSS — mirrors markdown.rs sanitizer logic.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// A single diff line with classification.
#[derive(Clone, Debug, PartialEq)]
pub enum DiffLine {
    /// Unchanged context line
    Context(String),
    /// Removed line (old)
    Removed(String),
    /// Added line (new)
    Added(String),
    /// Hunk header (e.g. @@ -1,3 +1,4 @@)
    HunkHeader(String),
}

/// Parse a unified diff string into classified lines.
pub fn parse_unified_diff(patch: &str) -> Vec<DiffLine> {
    patch
        .lines()
        .map(|line| {
            if line.starts_with("@@") {
                DiffLine::HunkHeader(line.to_string())
            } else if line.starts_with('-') && !line.starts_with("---") {
                DiffLine::Removed(line[1..].to_string())
            } else if line.starts_with('+') && !line.starts_with("+++") {
                DiffLine::Added(line[1..].to_string())
            } else if line.starts_with(' ') {
                DiffLine::Context(line[1..].to_string())
            } else {
                // Lines without prefix (e.g. "---", "+++", or bare context)
                DiffLine::Context(line.to_string())
            }
        })
        .collect()
}

/// Render a unified diff as styled HTML.
///
/// # Arguments
/// * `patch` — unified diff string (e.g. from `diff -u` or `git diff`)
fn render_diff_html(patch: &str) -> String {
    let lines = parse_unified_diff(patch);
    let mut html = String::with_capacity(patch.len() * 3);

    html.push_str("<div style=\"font-family: 'JetBrains Mono', 'Fira Code', monospace; font-size: 12px; line-height: 1.5; background: rgba(0,0,0,0.3); border-radius: 6px; overflow-x: auto; padding: 8px 0;\">");

    for (i, line) in lines.iter().enumerate() {
        let line_num = i + 1;
        match line {
            DiffLine::HunkHeader(text) => {
                let escaped = escape_html(text);
                html.push_str(&format!(
                    "<div style=\"color: rgba(139,92,246,0.8); padding: 2px 12px; background: rgba(139,92,246,0.08); font-weight: 600;\">{}</div>",
                    escaped
                ));
            }
            DiffLine::Context(text) => {
                let escaped = escape_html(text);
                html.push_str(&format!(
                    "<div style=\"color: rgba(255,255,255,0.6); padding: 1px 12px;\"><span style=\"color: rgba(255,255,255,0.25); user-select: none; display: inline-block; width: 40px; text-align: right; margin-right: 12px;\">{}</span> {}</div>",
                    line_num, escaped
                ));
            }
            DiffLine::Removed(text) => {
                let escaped = escape_html(text);
                html.push_str(&format!(
                    "<div style=\"color: rgba(239,68,68,0.9); padding: 1px 12px; background: rgba(239,68,68,0.08);\"><span style=\"color: rgba(239,68,68,0.5); user-select: none; display: inline-block; width: 40px; text-align: right; margin-right: 12px;\">−{}</span> {}</div>",
                    line_num, escaped
                ));
            }
            DiffLine::Added(text) => {
                let escaped = escape_html(text);
                html.push_str(&format!(
                    "<div style=\"color: rgba(34,197,94,0.9); padding: 1px 12px; background: rgba(34,197,94,0.08);\"><span style=\"color: rgba(34,197,94,0.5); user-select: none; display: inline-block; width: 40px; text-align: right; margin-right: 12px;\">+{}</span> {}</div>",
                    line_num, escaped
                ));
            }
        }
    }

    html.push_str("</div>");
    html
}

/// Leptos DiffViewer component.
///
/// Renders a unified diff preview with:
/// - Red (−) for removed lines, green (+) for added lines
/// - Line numbers in gutter
/// - Monospace font matching design.rs aesthetic
/// - XSS-safe HTML escaping
///
/// # Props
/// * `patch` — unified diff string
#[component]
pub fn DiffViewer(patch: ReadSignal<String>) -> impl IntoView {
    view! {
        <div
            style="border: 1px solid rgba(255,255,255,0.08); border-radius: 8px; overflow: hidden;"
            inner_html={move || render_diff_html(&patch.get())}
        ></div>
    }
}

/// Detect if a PendingApproval args payload contains diff-able content.
/// Returns Some(patch_string) if detected, None otherwise.
///
/// Supports:
/// - `{patch: "..."}` — raw unified diff
/// - `{path, old_str, new_str}` — construct diff from old/new
/// - `{path, content_old, content_new}` — alternate naming
pub fn detect_diff_args(args: &serde_json::Value) -> Option<String> {
    // Case 1: raw patch field
    if let Some(patch) = args.get("patch").and_then(|v| v.as_str()) {
        if !patch.is_empty() {
            return Some(patch.to_string());
        }
    }

    // Case 2: old_str + new_str (edit_file tool shape)
    let old = args.get("old_str").and_then(|v| v.as_str())
        .or_else(|| args.get("content_old").and_then(|v| v.as_str()));
    let new = args.get("new_str").and_then(|v| v.as_str())
        .or_else(|| args.get("content_new").and_then(|v| v.as_str()));

    if let (Some(old), Some(new)) = (old, new) {
        // Construct a unified diff from old/new
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("file");
        return Some(build_unified_diff(path, old, new));
    }

    None
}

/// Build a simple unified diff from old and new content.
fn build_unified_diff(path: &str, old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut diff = format!("--- a/{}\n+++ b/{}\n", path, path);

    // Simple line-by-line diff: mark all old as removed, all new as added
    // (A real LCS diff would be better, but this covers the common case
    // where edit_file replaces a block)
    diff.push_str(&format!("@@ -1,{} +1,{} @@\n", old_lines.len(), new_lines.len()));
    for line in &old_lines {
        diff.push_str(&format!("-{}\n", line));
    }
    for line in &new_lines {
        diff.push_str(&format!("+{}\n", line));
    }

    diff
}

// ═══════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_basic_render_unchanged_lines() {
        let patch = "--- a/test.rs\n+++ b/test.rs\n@@ -1,3 +1,3 @@\n fn main() {\n-    println!(\"old\");\n+    println!(\"new\");\n }\n";
        let lines = parse_unified_diff(patch);
        // First 2 lines are "---" and "+++" which parse as Context (no prefix)
        assert!(matches!(&lines[0], DiffLine::Context(s) if s.contains("test.rs")));
        assert!(matches!(&lines[1], DiffLine::Context(s) if s.contains("test.rs")));
        // Hunk header
        assert!(matches!(&lines[2], DiffLine::HunkHeader(s) if s.contains("@@")));
        // Context line (space prefix stripped)
        assert!(matches!(&lines[3], DiffLine::Context(s) if s == "fn main() {"));
        // Removed
        assert!(matches!(&lines[4], DiffLine::Removed(s) if s.contains("println!(\"old\")")));
        // Added
        assert!(matches!(&lines[5], DiffLine::Added(s) if s.contains("println!(\"new\")")));
        // Closing context
        assert!(matches!(&lines[6], DiffLine::Context(s) if s == "}"));
    }

    #[test]
    fn test_diff_minus_plus_classification() {
        let patch = "-removed\n+added\n context\n@@ -1 +1 @@\n-old\n+new\n";
        let lines = parse_unified_diff(patch);
        assert!(matches!(&lines[0], DiffLine::Removed(s) if s == "removed"));
        assert!(matches!(&lines[1], DiffLine::Added(s) if s == "added"));
        assert!(matches!(&lines[2], DiffLine::Context(s) if s == "context"));
        assert!(matches!(&lines[3], DiffLine::HunkHeader(s) if s.contains("@@")));
        assert!(matches!(&lines[4], DiffLine::Removed(s) if s == "old"));
        assert!(matches!(&lines[5], DiffLine::Added(s) if s == "new"));
    }

    #[test]
    fn test_diff_html_escapes_lt_gt_amp() {
        let patch = "-<script>alert('xss')</script>\n+<b>safe & sound</b>\n";
        let html = render_diff_html(patch);
        // Must NOT contain raw <script> or unescaped &
        assert!(!html.contains("<script>"), "XSS: raw <script> found in output");
        assert!(!html.contains("alert('xss')"), "XSS: unescaped script content");
        // Must contain escaped versions
        assert!(html.contains("&lt;script&gt;"), "Expected escaped <script>");
        assert!(html.contains("&amp;"), "Expected escaped &");
        assert!(html.contains("&lt;b&gt;"), "Expected escaped <b>");
    }

    #[test]
    fn test_detect_diff_args_patch_field() {
        let args = serde_json::json!({"patch": "--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-old\n+new\n"});
        let result = detect_diff_args(&args);
        assert!(result.is_some());
        assert!(result.unwrap().contains("-old"));
    }

    #[test]
    fn test_detect_diff_args_old_new_str() {
        let args = serde_json::json!({"path": "src/main.rs", "old_str": "fn old() {}", "new_str": "fn new() {}"});
        let result = detect_diff_args(&args);
        assert!(result.is_some());
        let diff = result.unwrap();
        assert!(diff.contains("--- a/src/main.rs"));
        assert!(diff.contains("-fn old() {}"));
        assert!(diff.contains("+fn new() {}"));
    }

    #[test]
    fn test_detect_diff_args_none_for_json() {
        let args = serde_json::json!({"command": "ls", "path": "/tmp"});
        assert!(detect_diff_args(&args).is_none());
    }
}
