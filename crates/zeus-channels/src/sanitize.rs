//! Channel-outbound text sanitization helpers.
//!
//! Some channel transports render naked Markdown literally to end users
//! (Telegram with no `parse_mode`, IRC PRIVMSG, etc.). Titans naturally
//! emit Markdown (`**bold**`, `` `code` ``, `# heading`, `- bullet`).
//! This module strips that syntax while preserving the readable content.
//!
//! Adapter-adjacent: lives in `zeus-channels` alongside the transports
//! that need it. Pure functions only; no I/O, no async, no allocation
//! beyond the returned `String`.

/// Strip common Markdown syntax from text before sending to a
/// markdown-naive transport (Telegram, IRC, etc.).
///
/// Three-pass strategy:
///   1. Drop code-fence delimiter lines (```...```), keep inner lines.
///   2. Per-line leading-marker strip (headings, blockquote, bullets).
///   3. Paired-delimiter inline-syntax strip (longest delimiter first).
///
/// Pure function, idempotent on already-sanitized text.
pub fn strip_markdown(input: &str) -> String {
    // First pass: drop code-fence delimiter lines (```...```), keep inner lines.
    let mut out_lines: Vec<String> = Vec::with_capacity(input.lines().count());
    for line in input.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            continue;
        }
        out_lines.push(line.to_string());
    }

    // Second pass: per-line leading-marker strip (headings, blockquote, bullets).
    for line in out_lines.iter_mut() {
        let leading_ws_len = line.len() - line.trim_start().len();
        let (ws, rest) = line.split_at(leading_ws_len);
        let stripped = if let Some(r) = rest
            .strip_prefix("###### ")
            .or_else(|| rest.strip_prefix("##### "))
            .or_else(|| rest.strip_prefix("#### "))
            .or_else(|| rest.strip_prefix("### "))
            .or_else(|| rest.strip_prefix("## "))
            .or_else(|| rest.strip_prefix("# "))
            .or_else(|| rest.strip_prefix("> "))
            .or_else(|| rest.strip_prefix("- "))
            .or_else(|| rest.strip_prefix("* "))
            .or_else(|| rest.strip_prefix("+ "))
        {
            r.to_string()
        } else {
            rest.to_string()
        };
        *line = format!("{}{}", ws, stripped);
    }

    let mut text = out_lines.join("\n");

    // Third pass: paired-delimiter inline-syntax strip. Longer delimiters first.
    text = strip_paired(&text, "**");
    text = strip_paired(&text, "__");
    text = strip_paired(&text, "~~");
    text = strip_paired(&text, "`");
    text = strip_paired(&text, "*");
    text = strip_paired(&text, "_");

    text
}

/// Strip all occurrences of `delim` from `s` when at least one pair exists.
/// Pure helper for `strip_markdown`.
fn strip_paired(s: &str, delim: &str) -> String {
    let parts: Vec<&str> = s.split(delim).collect();
    if parts.len() < 3 {
        return s.to_string();
    }
    parts.join("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_markdown_handles_inline_syntax() {
        assert_eq!(strip_markdown("**bold** text"), "bold text");
        assert_eq!(strip_markdown("*italic* text"), "italic text");
        assert_eq!(strip_markdown("`code` here"), "code here");
        assert_eq!(strip_markdown("~~strike~~ done"), "strike done");
        assert_eq!(strip_markdown("__under__ text"), "under text");
    }

    #[test]
    fn strip_markdown_handles_block_prefixes() {
        assert_eq!(strip_markdown("# Heading"), "Heading");
        assert_eq!(strip_markdown("### H3"), "H3");
        assert_eq!(strip_markdown("> quoted"), "quoted");
        assert_eq!(strip_markdown("- bullet"), "bullet");
        assert_eq!(strip_markdown("* bullet"), "bullet");
        assert_eq!(strip_markdown("+ bullet"), "bullet");
    }

    #[test]
    fn strip_markdown_handles_code_fences_and_multiline() {
        let input = "before\n```rust\nlet x = 1;\n```\nafter";
        let out = strip_markdown(input);
        assert!(out.contains("let x = 1;"));
        assert!(!out.contains("```"));
        assert!(out.contains("before"));
        assert!(out.contains("after"));
    }

    #[test]
    fn strip_markdown_is_idempotent_and_handles_edges() {
        let plain = "plain text with no markup";
        assert_eq!(strip_markdown(plain), plain);
        let once = strip_markdown("**bold**");
        let twice = strip_markdown(&once);
        assert_eq!(once, twice);
        // Unpaired delimiter: don't crash; output stays sensible.
        let _ = strip_markdown("a * lonely star");
        // Empty.
        assert_eq!(strip_markdown(""), "");
    }
}
