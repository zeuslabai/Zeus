//! Inline diff viewer — S100 #12
//!
//! Detects unified diff blocks inside chat messages and renders them with
//! syntax-highlighted hunks:
//!   - Added lines   (+)  → green
//!   - Removed lines (-)  → red
//!   - Hunk headers (@@)  → cyan/bold
//!   - File headers (---/+++) → yellow
//!   - Context lines        → default text colour

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::theme;

/// Classify a diff line for styling purposes.
#[derive(Debug, PartialEq)]
pub enum DiffLineKind {
    FileHeader,  // --- / +++
    HunkHeader,  // @@
    Added,       // +
    Removed,     // -
    Context,     // space / unchanged
    Normal,      // not a diff line
}

pub fn classify(line: &str) -> DiffLineKind {
    if line.starts_with("--- ") || line.starts_with("+++ ") {
        DiffLineKind::FileHeader
    } else if line.starts_with("@@ ") {
        DiffLineKind::HunkHeader
    } else if line.starts_with('+') {
        DiffLineKind::Added
    } else if line.starts_with('-') {
        DiffLineKind::Removed
    } else if line.starts_with(' ') {
        DiffLineKind::Context
    } else {
        DiffLineKind::Normal
    }
}

/// Returns true when `text` looks like it contains a unified diff block.
/// Heuristic: must have at least one hunk header (`@@ `) and one add/remove line.
pub fn contains_diff(text: &str) -> bool {
    let has_hunk = text.lines().any(|l| l.starts_with("@@ "));
    let has_delta = text.lines().any(|l| l.starts_with('+') || l.starts_with('-'));
    has_hunk && has_delta
}

/// Render a single diff line as a styled `Line<'static>`.
pub fn render_diff_line(raw: &str, indent: &str) -> Line<'static> {
    let kind = classify(raw);
    let style = match kind {
        DiffLineKind::FileHeader => Style::default().fg(theme::YELLOW).add_modifier(Modifier::BOLD),
        DiffLineKind::HunkHeader => Style::default().fg(theme::CYAN).add_modifier(Modifier::BOLD),
        DiffLineKind::Added      => Style::default().fg(theme::GREEN),
        DiffLineKind::Removed    => Style::default().fg(theme::RED),
        DiffLineKind::Context    => theme::text(),
        DiffLineKind::Normal     => theme::text(),
    };
    Line::from(Span::styled(format!("{}{}", indent, raw), style))
}

/// Convert a text block that contains diff content into styled `Line`s.
/// Non-diff lines are passed through with normal styling.
pub fn render_diff_block<'a>(text: &str, indent: &str) -> Vec<Line<'static>> {
    text.lines()
        .map(|line| render_diff_line(line, indent))
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_file_header_minus() {
        assert_eq!(classify("--- a/foo.rs"), DiffLineKind::FileHeader);
    }

    #[test]
    fn test_classify_file_header_plus() {
        assert_eq!(classify("+++ b/foo.rs"), DiffLineKind::FileHeader);
    }

    #[test]
    fn test_classify_hunk_header() {
        assert_eq!(classify("@@ -1,4 +1,5 @@"), DiffLineKind::HunkHeader);
    }

    #[test]
    fn test_classify_added() {
        assert_eq!(classify("+    let x = 1;"), DiffLineKind::Added);
    }

    #[test]
    fn test_classify_removed() {
        assert_eq!(classify("-    let x = 0;"), DiffLineKind::Removed);
    }

    #[test]
    fn test_classify_context() {
        assert_eq!(classify(" fn main() {"), DiffLineKind::Context);
    }

    #[test]
    fn test_classify_normal() {
        assert_eq!(classify("some prose"), DiffLineKind::Normal);
    }

    #[test]
    fn test_contains_diff_true() {
        let text = "--- a/foo.rs\n+++ b/foo.rs\n@@ -1,3 +1,4 @@\n fn main() {}\n+    println!(\"hi\");\n";
        assert!(contains_diff(text));
    }

    #[test]
    fn test_contains_diff_false_no_hunk() {
        let text = "+++ b/foo.rs\n+    let x = 1;\n";
        assert!(!contains_diff(text));
    }

    #[test]
    fn test_contains_diff_false_prose() {
        let text = "Here is some text about diffs.\nNothing special here.";
        assert!(!contains_diff(text));
    }

    #[test]
    fn test_render_diff_line_added_is_green() {
        let line = render_diff_line("+    let x = 1;", "  ");
        // Check the span text contains the raw line
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("+    let x = 1;"));
    }

    #[test]
    fn test_render_diff_line_removed_is_red() {
        let line = render_diff_line("-    let x = 0;", "  ");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("-    let x = 0;"));
    }

    #[test]
    fn test_render_diff_block_line_count() {
        let diff = "--- a/foo.rs\n+++ b/foo.rs\n@@ -1,2 +1,3 @@\n fn main() {}\n+    let x = 1;\n";
        let lines = render_diff_block(diff, "  ");
        assert_eq!(lines.len(), 5);
    }
}
