//! Markdown → Slack mrkdwn sanitizer.
//!
//! Slack's `mrkdwn` format is a restricted subset of Markdown. When
//! `text_type: "mrkdwn"` is set on a Slack text object the content is
//! interpreted by Slack's own renderer — but standard Markdown syntax does
//! NOT map 1-to-1:
//!
//! | Standard Markdown | Slack mrkdwn |
//! |-------------------|--------------|
//! | `**bold**`        | `*bold*`     |
//! | `__bold__`        | `*bold*`     |
//! | `~~strike~~`      | `~strike~`   |
//! | `# Heading`       | `*Heading*`  |
//! | `[txt](url)`      | `<url\|txt>` |
//! | `- item`          | `• item`     |
//! | `* item` (list)   | `• item`     |
//! | `` `inline` ``    | unchanged    |
//! | ` ``` `fenced` ``` ` | unchanged |
//! | `_italic_`        | unchanged    |
//!
//! Use [`to_slack_mrkdwn`] before setting any mrkdwn text block content.

use std::borrow::Cow;

/// Convert a standard Markdown string to Slack mrkdwn syntax.
///
/// Code spans and fenced code blocks are passed through verbatim so that
/// backtick formatting is preserved. All other conversions are applied
/// line-by-line.
pub fn to_slack_mrkdwn(md: &str) -> String {
    // We process the string in two passes:
    //   1. Extract code spans / fenced blocks (protect from regex mangling).
    //   2. Apply mrkdwn conversions to the non-code segments.
    //   3. Re-stitch protected segments back in.

    let mut output = String::with_capacity(md.len());
    let mut chars = md.char_indices().peekable();

    // State: are we inside a fenced code block?
    let mut in_fence = false;
    // Buffer for the current logical line (between newlines).
    let mut line_buf = String::new();

    // Flush `line_buf` through the inline converter and append to `output`.
    macro_rules! flush_line {
        () => {{
            let converted = if in_fence {
                Cow::Borrowed(line_buf.as_str())
            } else {
                Cow::Owned(convert_line(&line_buf))
            };
            output.push_str(&converted);
            line_buf.clear();
        }};
    }

    let bytes = md.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;

    while i < len {
        // Detect fenced code block delimiter (``` at start of line / after newline).
        if bytes[i] == b'`'
            && i + 2 < len
            && bytes[i + 1] == b'`'
            && bytes[i + 2] == b'`'
            && (i == 0 || bytes[i - 1] == b'\n')
        {
            // Consume the triple-backtick run (may have language hint).
            flush_line!();
            // Copy the fence line verbatim.
            while i < len && bytes[i] != b'\n' {
                output.push(bytes[i] as char);
                i += 1;
            }
            // Include the newline.
            if i < len {
                output.push('\n');
                i += 1;
            }
            in_fence = !in_fence;
            continue;
        }

        let ch = bytes[i] as char;
        if ch == '\n' {
            flush_line!();
            output.push('\n');
            i += 1;
        } else {
            line_buf.push(ch);
            i += 1;
        }
    }
    // Flush any trailing content without a terminal newline.
    flush_line!();

    output
}

/// Apply inline mrkdwn conversions to a single non-fenced line.
fn convert_line(line: &str) -> String {
    // Order matters: apply heading first (line-level), then inline spans.
    let line = convert_headings(line);
    let line = convert_bold(&line);
    let line = convert_strike(&line);
    let line = convert_links(&line);
    let line = convert_lists(&line);
    line
}

/// `# Heading` → `*Heading*` (all ATX heading levels collapse to bold).
fn convert_headings(line: &str) -> String {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return line.to_string();
    }
    // Count leading '#' chars.
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if hashes > 6 {
        return line.to_string();
    }
    let rest = trimmed[hashes..].trim_start();
    if rest.is_empty() {
        return line.to_string();
    }
    format!("*{rest}*")
}

/// `**bold**` and `__bold__` → `*bold*`.
///
/// We process the string character-by-character to avoid mangling code spans.
fn convert_bold(s: &str) -> String {
    replace_delimited_spans(s, &[("**", "**"), ("__", "__")], "*", "*")
}

/// `~~strike~~` → `~strike~`.
fn convert_strike(s: &str) -> String {
    replace_delimited_spans(s, &[("~~", "~~")], "~", "~")
}

/// `[text](url)` → `<url|text>`.
fn convert_links(s: &str) -> String {
    // Simple state-machine: scan for '[', capture label up to '](',
    // then capture URL up to ')'.
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;

    while i < len {
        if bytes[i] == b'[' {
            // Try to match [label](url)
            if let Some((label, url, end)) = parse_md_link(bytes, i) {
                out.push('<');
                out.push_str(&url);
                out.push('|');
                out.push_str(&label);
                out.push('>');
                i = end;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Parse `[label](url)` starting at `start` (the `[` byte).
/// Returns `(label, url, end_index)` where `end_index` is the byte after `)`.
fn parse_md_link(bytes: &[u8], start: usize) -> Option<(String, String, usize)> {
    let len = bytes.len();
    // Find closing ']'
    let mut i = start + 1;
    while i < len && bytes[i] != b']' {
        i += 1;
    }
    if i >= len {
        return None;
    }
    let label = String::from_utf8_lossy(&bytes[start + 1..i]).into_owned();
    i += 1; // skip ']'
    if i >= len || bytes[i] != b'(' {
        return None;
    }
    i += 1; // skip '('
    let url_start = i;
    while i < len && bytes[i] != b')' {
        i += 1;
    }
    if i >= len {
        return None;
    }
    let url = String::from_utf8_lossy(&bytes[url_start..i]).into_owned();
    Some((label, url, i + 1))
}

/// `- item` and `* item` (when not bold) → `• item`.
///
/// Only converts list items at the start of the line (after optional whitespace).
fn convert_lists(line: &str) -> String {
    let trimmed = line.trim_start();
    // Must start with `- ` or `* ` (not `**` which would be bold).
    if (trimmed.starts_with("- ") || trimmed.starts_with("* "))
        && !trimmed.starts_with("**")
    {
        let indent_len = line.len() - trimmed.len();
        let indent = &line[..indent_len];
        let content = &trimmed[2..];
        return format!("{indent}• {content}");
    }
    line.to_string()
}

/// Generic span-replacement helper.
///
/// Scans `s` for any opening delimiter in `pairs`, captures content up to
/// the matching closing delimiter, and wraps with `open_rep` / `close_rep`.
/// Code spans (backtick-delimited) are passed through verbatim.
fn replace_delimited_spans(
    s: &str,
    pairs: &[(&str, &str)],
    open_rep: &str,
    close_rep: &str,
) -> String {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;

    while i < len {
        // Pass code spans verbatim.
        if bytes[i] == b'`' {
            // Find matching closing backtick.
            let mut j = i + 1;
            while j < len && bytes[j] != b'`' {
                j += 1;
            }
            out.push_str(&s[i..=j.min(len - 1)]);
            i = j + 1;
            continue;
        }

        // Try each delimiter pair.
        let mut matched = false;
        for (open, close) in pairs {
            let ob = open.as_bytes();
            let cb = close.as_bytes();
            if bytes[i..].starts_with(ob) {
                // Look for closing delimiter after the opening.
                let content_start = i + ob.len();
                if let Some(rel) = find_substr(&bytes[content_start..], cb) {
                    let content_end = content_start + rel;
                    let inner = &s[content_start..content_end];
                    out.push_str(open_rep);
                    out.push_str(inner);
                    out.push_str(close_rep);
                    i = content_end + cb.len();
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Find the byte offset of `needle` within `haystack`, or `None`.
fn find_substr(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bold_double_star() {
        assert_eq!(to_slack_mrkdwn("Hello **world**!"), "Hello *world*!");
    }

    #[test]
    fn test_bold_double_underscore() {
        assert_eq!(to_slack_mrkdwn("Hello __world__!"), "Hello *world*!");
    }

    #[test]
    fn test_italic_unchanged() {
        // _italic_ must NOT be converted — Slack renders it natively.
        assert_eq!(to_slack_mrkdwn("_italic text_"), "_italic text_");
    }

    #[test]
    fn test_strike() {
        assert_eq!(to_slack_mrkdwn("~~deleted~~"), "~deleted~");
    }

    #[test]
    fn test_heading_all_levels() {
        assert_eq!(to_slack_mrkdwn("# Title"), "*Title*");
        assert_eq!(to_slack_mrkdwn("## Section"), "*Section*");
        assert_eq!(to_slack_mrkdwn("### Sub"), "*Sub*");
    }

    #[test]
    fn test_link() {
        assert_eq!(
            to_slack_mrkdwn("[Zeus](https://zeuslab.ai)"),
            "<https://zeuslab.ai|Zeus>"
        );
    }

    #[test]
    fn test_dash_list() {
        let input = "- first\n- second";
        let expected = "• first\n• second";
        assert_eq!(to_slack_mrkdwn(input), expected);
    }

    #[test]
    fn test_star_list() {
        let input = "* alpha\n* beta";
        let expected = "• alpha\n• beta";
        assert_eq!(to_slack_mrkdwn(input), expected);
    }

    #[test]
    fn test_fenced_code_block_preserved() {
        let input = "Before\n```rust\nlet x = **bold**;\n```\nAfter **bold**";
        let result = to_slack_mrkdwn(input);
        // The fenced block interior must not be transformed.
        assert!(result.contains("let x = **bold**;"), "fenced interior mutated: {result}");
        // Text outside fenced block should be converted.
        assert!(result.contains("After *bold*"), "bold outside fence not converted: {result}");
    }

    #[test]
    fn test_inline_code_preserved() {
        // Inline code spans must pass through verbatim.
        assert_eq!(
            to_slack_mrkdwn("Use `**raw**` here"),
            "Use `**raw**` here"
        );
    }
}
