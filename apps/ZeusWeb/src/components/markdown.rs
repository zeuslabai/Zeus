#![allow(dead_code)]
use pulldown_cmark::{Parser, Options, html};

/// Convert markdown text to sanitized HTML.
/// Strips dangerous tags (script, iframe, etc.) from the output.
/// Replaces ```mermaid and ```chart code blocks with renderable containers.
pub fn render_markdown(text: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(text, opts);
    let mut html_output = String::with_capacity(text.len() * 2);
    html::push_html(&mut html_output, parser);

    // Basic sanitization: remove script/iframe/object/embed tags
    let mut result = sanitize_html(&html_output);

    // Post-process: replace mermaid code blocks with renderable containers
    result = replace_visual_blocks(&result, "mermaid");
    result = replace_visual_blocks(&result, "chart");

    result
}

/// Replace `<pre><code class="language-{lang}">...</code></pre>` blocks
/// with special container divs for client-side rendering.
fn replace_visual_blocks(html: &str, lang: &str) -> String {
    let open_tag = format!("<code class=\"language-{}\">", lang);
    let mut result = String::with_capacity(html.len());
    let mut search_from = 0;

    while let Some(code_pos) = html[search_from..].find(&open_tag) {
        let abs_code = search_from + code_pos;
        // Find the <pre> that precedes this <code>
        let pre_start = html[..abs_code].rfind("<pre>").unwrap_or(abs_code);
        // Push everything before the <pre>
        result.push_str(&html[search_from..pre_start]);

        let content_start = abs_code + open_tag.len();
        if let Some(end) = html[content_start..].find("</code></pre>") {
            let raw_content = &html[content_start..content_start + end];
            // Decode HTML entities back to raw text for mermaid/chart
            let decoded = decode_html_entities(raw_content);
            // Escape for HTML attribute
            let attr_safe = decoded
                .replace('&', "&amp;")
                .replace('"', "&quot;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");

            if lang == "mermaid" {
                result.push_str(&format!(
                    "<div class=\"zeus-mermaid\" data-pending=\"1\" data-code=\"{}\"></div>",
                    attr_safe
                ));
            } else {
                result.push_str(&format!(
                    "<div class=\"zeus-chart\" data-pending=\"1\" data-config=\"{}\"></div>",
                    attr_safe
                ));
            }
            search_from = content_start + end + "</code></pre>".len();
        } else {
            // Malformed — keep as-is
            result.push_str(&html[pre_start..content_start]);
            search_from = content_start;
        }
    }
    result.push_str(&html[search_from..]);
    result
}

/// Decode common HTML entities back to plain text.
fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
     .replace("&lt;", "<")
     .replace("&gt;", ">")
     .replace("&quot;", "\"")
     .replace("&#39;", "'")
     .replace("&#x27;", "'")
}

fn sanitize_html(html: &str) -> String {
    let mut result = html.to_string();
    // `style` — inline CSS can exfiltrate info via `background:url()` and
    // visually deface the app with `position:fixed`.
    // `svg`   — embedded SVG can carry `<script>` children and `javascript:`
    // hrefs that escape the tag-level blocklist above.
    // `link`  — external stylesheets at render time.
    // `meta`  — attacker-controlled meta-refresh redirects.
    // `base`  — rebasing relative URLs on the page.
    // Leptos escapes interpolated text by default, so the markdown input
    // has to have already produced raw HTML for these to matter — but
    // pulldown-cmark does emit raw HTML passthrough blocks, so this
    // sanitizer is the last line of defence.
    for tag in &[
        "script", "iframe", "object", "embed", "form",
        "style", "svg", "link", "meta", "base",
    ] {
        // Remove opening tags (with attributes)
        let open_pattern = format!("<{}", tag);
        while let Some(start) = result.to_lowercase().find(&open_pattern) {
            if let Some(end) = result[start..].find('>') {
                result.replace_range(start..start + end + 1, "");
            } else {
                break;
            }
        }
        // Remove closing tags
        let close_pattern = format!("</{}>", tag);
        result = result.replace(&close_pattern, "");
    }
    // Remove on* event attributes
    let mut i = 0;
    while i < result.len() {
        if let Some(pos) = result[i..].find(" on") {
            let abs_pos = i + pos;
            // Check if it looks like an event handler (on + alpha + =)
            let rest = &result[abs_pos + 3..];
            if rest.starts_with(|c: char| c.is_ascii_alphabetic())
                && let Some(eq) = rest.find('=') {
                    let between = &rest[..eq];
                    if between.chars().all(|c| c.is_ascii_alphabetic()) {
                        // Find end of attribute value
                        let after_eq = &rest[eq + 1..];
                        let end = if let Some(stripped) = after_eq.strip_prefix('"') {
                            stripped.find('"').map(|p| p + 2)
                        } else if let Some(stripped) = after_eq.strip_prefix('\'') {
                            stripped.find('\'').map(|p| p + 2)
                        } else {
                            after_eq.find(|c: char| c.is_whitespace() || c == '>').or(Some(after_eq.len()))
                        };
                        if let Some(end_offset) = end {
                            let remove_end = abs_pos + 3 + eq + 1 + end_offset;
                            result.replace_range(abs_pos..remove_end, "");
                            continue;
                        }
                    }
                }
            i = abs_pos + 3;
        } else {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizer_strips_svg_and_nested_script() {
        let html = r#"<p>ok</p><svg><script>alert(1)</script></svg><p>end</p>"#;
        let out = sanitize_html(html);
        assert!(!out.to_lowercase().contains("<svg"), "svg tag should be stripped");
        assert!(!out.to_lowercase().contains("<script"), "nested script should be stripped");
        assert!(out.contains("<p>ok</p>"));
        assert!(out.contains("<p>end</p>"));
    }

    #[test]
    fn sanitizer_strips_style_tag() {
        let html = "<style>body{background:red}</style><p>hi</p>";
        let out = sanitize_html(html);
        assert!(!out.to_lowercase().contains("<style"));
        assert!(out.contains("<p>hi</p>"));
    }

    #[test]
    fn sanitizer_strips_link_meta_base() {
        let html = r#"<link rel="stylesheet" href="x"><meta http-equiv="refresh" content="0;url=x"><base href="x">"#;
        let out = sanitize_html(html);
        assert!(!out.to_lowercase().contains("<link"));
        assert!(!out.to_lowercase().contains("<meta"));
        assert!(!out.to_lowercase().contains("<base"));
    }

    #[test]
    fn sanitizer_preserves_safe_markdown_html() {
        // All the structural tags pulldown-cmark emits — none should be stripped.
        let html = "<h1>t</h1><p><strong>b</strong><em>i</em><code>c</code></p>\
                    <ul><li>x</li></ul><ol><li>y</li></ol>\
                    <table><thead><tr><th>a</th></tr></thead><tbody><tr><td>b</td></tr></tbody></table>\
                    <a href=\"https://example.com\">link</a>";
        let out = sanitize_html(html);
        assert!(out.contains("<h1>t</h1>"));
        assert!(out.contains("<strong>b</strong>"));
        assert!(out.contains("<em>i</em>"));
        assert!(out.contains("<code>c</code>"));
        assert!(out.contains("<ul>") && out.contains("<ol>"));
        assert!(out.contains("<table>"));
        assert!(out.contains("href=\"https://example.com\""));
    }

    #[test]
    fn sanitizer_strips_on_event_attributes() {
        let html = r#"<a href="x" onclick="alert(1)">click</a>"#;
        let out = sanitize_html(html);
        assert!(!out.contains("onclick"));
        assert!(out.contains("href=\"x\""));
    }

    #[test]
    fn render_markdown_end_to_end_safe() {
        // Raw HTML passthrough containing a malicious svg — the render pipeline
        // must strip it before returning.
        let md = "Start\n\n<svg><script>alert(1)</script></svg>\n\nEnd";
        let out = render_markdown(md);
        assert!(!out.to_lowercase().contains("<svg"));
        assert!(!out.to_lowercase().contains("<script"));
    }
}
