//! Markdown renderer for TUI chat messages.
//!
//! Converts markdown text into ratatui `Line` spans with appropriate styling.
//! Supports: bold, italic, inline code, code blocks, headers, bullet points.

use ratatui::prelude::*;
use crate::theme;

/// Parse a markdown string into a vec of ratatui Lines, ready to render.
#[allow(dead_code)]
pub fn render_markdown(text: &str, indent: &str) -> Vec<Line<'static>> {
    render_markdown_with_width(text, indent, 0)
}

/// Render markdown with word wrapping at the given width (0 = no wrap)
pub fn render_markdown_with_width(text: &str, indent: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();

    for raw in text.lines() {
        // Code block fence
        if raw.trim_start().starts_with("```") {
            if in_code_block {
                // closing fence
                in_code_block = false;
                code_lang.clear();
            } else {
                in_code_block = true;
                code_lang = raw.trim_start().trim_start_matches('`').to_string();
            }
            continue;
        }

        if in_code_block {
            // Render code lines with code style
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}  {}", indent, raw),
                    Style::default().fg(theme::YELLOW).bg(theme::BG_PANEL),
                ),
            ]));
            continue;
        }

        // Empty line
        if raw.trim().is_empty() {
            lines.push(Line::raw(""));
            continue;
        }

        // Headers
        if raw.starts_with("### ") {
            let content = raw[4..].to_string();
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}  {}", indent, content),
                    Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD),
                ),
            ]));
            continue;
        }
        if raw.starts_with("## ") {
            let content = raw[3..].to_string();
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}  {}", indent, content),
                    Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD),
                ),
            ]));
            continue;
        }
        if raw.starts_with("# ") {
            let content = raw[2..].to_string();
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}  {}", indent, content),
                    Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
                ),
            ]));
            continue;
        }

        // Bullet points
        let (prefix, content_str) = if raw.starts_with("- ") || raw.starts_with("* ") {
            ("• ", &raw[2..])
        } else if raw.len() > 2 && raw.starts_with("  - ") {
            ("  • ", &raw[4..])
        } else {
            ("", &raw[..])
        };

        // Parse inline spans (bold, italic, inline code)
        let mut spans = Vec::new();
        if !prefix.is_empty() {
            spans.push(Span::styled(
                format!("{}  {}", indent, prefix),
                Style::default().fg(theme::GREEN),
            ));
        } else {
            spans.push(Span::raw(format!("{}  ", indent)));
        }

        spans.extend(parse_inline(content_str));

        // Word wrap if width is set
        if width > 4 {
            let indent_len = indent.len() + 2; // "  " prefix
            let wrap_at = width.saturating_sub(indent_len);
            let full_text: String = spans.iter().map(|s| s.content.as_ref()).collect();
            if full_text.len() > wrap_at {
                // Simple word wrap — split into lines that fit
                let mut current = String::new();
                let mut col = 0usize;
                let wrap_indent = format!("{}  ", indent);
                let mut first = true;
                for word in full_text.split_whitespace() {
                    if col + word.len() + 1 > wrap_at && col > 0 {
                        if first {
                            lines.push(Line::from(Span::raw(current.clone())));
                            first = false;
                        } else {
                            lines.push(Line::styled(current.clone(), theme::text()));
                        }
                        current = format!("{}{}", wrap_indent, word);
                        col = wrap_indent.len() + word.len();
                    } else if col == 0 {
                        current = if first { full_text[..0].to_string() + &format!("{}  {}", indent, word) } else { format!("{}{}", wrap_indent, word) };
                        col = wrap_indent.len() + word.len();
                    } else {
                        current.push(' ');
                        current.push_str(word);
                        col += 1 + word.len();
                    }
                }
                if !current.is_empty() {
                    lines.push(Line::styled(current, theme::text()));
                }
            } else {
                lines.push(Line::from(spans));
            }
        } else {
            lines.push(Line::from(spans));
        }
    }

    lines
}

/// Parse inline markdown (bold, italic, inline code) into spans.
pub fn parse_inline(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut chars = text.chars().peekable();
    let mut buf = String::new();

    while let Some(ch) = chars.next() {
        match ch {
            '`' => {
                // Inline code
                if !buf.is_empty() {
                    spans.push(Span::styled(buf.clone(), theme::text()));
                    buf.clear();
                }
                let mut code = String::new();
                for c in chars.by_ref() {
                    if c == '`' { break; }
                    code.push(c);
                }
                spans.push(Span::styled(
                    code,
                    Style::default().fg(theme::YELLOW).bg(theme::BG_PANEL),
                ));
            }
            '*' => {
                // Bold (**text**) or italic (*text*)
                if chars.peek() == Some(&'*') {
                    // bold
                    chars.next();
                    if !buf.is_empty() {
                        spans.push(Span::styled(buf.clone(), theme::text()));
                        buf.clear();
                    }
                    let mut bold = String::new();
                    let mut prev = ' ';
                    for c in chars.by_ref() {
                        if c == '*' && prev == '*' {
                            bold.pop(); // remove trailing *
                            break;
                        }
                        bold.push(c);
                        prev = c;
                    }
                    spans.push(Span::styled(
                        bold,
                        Style::default().fg(theme::TEXT_BRIGHT).add_modifier(Modifier::BOLD),
                    ));
                } else {
                    // italic
                    if !buf.is_empty() {
                        spans.push(Span::styled(buf.clone(), theme::text()));
                        buf.clear();
                    }
                    let mut italic = String::new();
                    for c in chars.by_ref() {
                        if c == '*' { break; }
                        italic.push(c);
                    }
                    spans.push(Span::styled(
                        italic,
                        Style::default().fg(theme::GREEN).add_modifier(Modifier::ITALIC),
                    ));
                }
            }
            _ => buf.push(ch),
        }
    }

    if !buf.is_empty() {
        spans.push(Span::styled(buf, theme::text()));
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text_renders_as_single_line() {
        let lines = render_markdown("Hello world", "");
        assert_eq!(lines.len(), 1);
        // Should contain the text somewhere in spans
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Hello world"), "got: {text}");
    }

    #[test]
    fn test_code_block_renders_with_code_style() {
        let md = "```rust\nlet x = 1;\n```";
        let lines = render_markdown(md, "");
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("let x = 1;"), "got: {text}");
        // Code line should have YELLOW foreground
        assert_eq!(lines[0].spans[0].style.fg, Some(theme::YELLOW));
    }

    #[test]
    fn test_h1_renders_bold_red() {
        let lines = render_markdown("# Title", "");
        assert_eq!(lines.len(), 1);
        let span = &lines[0].spans[0];
        assert!(span.content.contains("Title"));
        assert_eq!(span.style.fg, Some(theme::RED));
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_h2_renders_bold_purple() {
        let lines = render_markdown("## Subtitle", "");
        assert_eq!(lines.len(), 1);
        let span = &lines[0].spans[0];
        assert!(span.content.contains("Subtitle"));
        assert_eq!(span.style.fg, Some(theme::PURPLE));
    }

    #[test]
    fn test_bullet_point_renders_with_bullet() {
        let lines = render_markdown("- item one", "");
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains('•'), "got: {text}");
        assert!(text.contains("item one"), "got: {text}");
    }

    #[test]
    fn test_inline_code_parsed() {
        let spans = parse_inline("use `cargo check` to verify");
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("cargo check"), "got: {text}");
        // Find the inline code span
        let code_span = spans.iter().find(|s| s.content.contains("cargo check")).unwrap();
        assert_eq!(code_span.style.fg, Some(theme::YELLOW));
    }

    #[test]
    fn test_bold_parsed() {
        let spans = parse_inline("this is **bold** text");
        let bold_span = spans.iter().find(|s| s.content.contains("bold")).unwrap();
        assert!(bold_span.style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(bold_span.style.fg, Some(theme::TEXT_BRIGHT));
    }

    #[test]
    fn test_italic_parsed() {
        let spans = parse_inline("this is *italic* text");
        let italic_span = spans.iter().find(|s| s.content.contains("italic")).unwrap();
        assert!(italic_span.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn test_empty_line_produces_empty_line() {
        let lines = render_markdown("line one\n\nline two", "");
        assert_eq!(lines.len(), 3);
        assert!(lines[1].spans.is_empty() || lines[1].spans.iter().all(|s| s.content.trim().is_empty()));
    }

    #[test]
    fn test_multiline_code_block() {
        let md = "```\nfn main() {\n    println!(\"hi\");\n}\n```";
        let lines = render_markdown(md, "");
        assert_eq!(lines.len(), 3);
        for line in &lines {
            assert_eq!(line.spans[0].style.fg, Some(theme::YELLOW));
        }
    }

    #[test]
    fn test_mixed_markdown() {
        let md = "# Header\n\n**bold** and `code`\n\n- bullet";
        let lines = render_markdown(md, "");
        // header + empty + inline + empty + bullet = 5
        assert_eq!(lines.len(), 5);
    }
}
