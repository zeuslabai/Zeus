//! Per-channel pure renderers for [`crate::rich::RichResponse`].
//!
//! Sprint #85 / Issue #88. Each function takes a `&RichResponse` and produces
//! the channel-native representation. All renderers are pure (no I/O), making
//! them unit-testable in isolation.
//!
//! Adapter trait overrides (`ChannelAdapter::send_rich`) for tier-1 channels
//! call into these and forward the result to the channel's transport-layer
//! send method.
//!
//! # Graceful degradation
//!
//! Renderers use [`RichResponse::partition`] to keep natively-renderable
//! fragments and flatten the rest to text. The text-flattened portion is
//! attached as either a leading description or trailing context, depending
//! on the channel's idiom.

use crate::rich::{ChannelCapabilities, ContentBlock, RichResponse};

// ── Discord renderer ────────────────────────────────────────────────────

/// Output of [`render_discord`].
///
/// Discord's wire protocol supports `content` (top-level text), one or more
/// embeds, and file attachments. We map a `RichResponse` to a single embed
/// (title/footer/color + description-aggregated text) plus an `image_url`
/// when there's exactly one image, plus optional file-attachment list.
#[derive(Debug, Clone, Default)]
pub struct DiscordRender {
    /// Optional top-level message content (text outside the embed).
    pub content: Option<String>,
    /// The primary embed (None if response is empty).
    pub embed: Option<crate::discord::DiscordEmbed>,
    /// File attachments: (filename, bytes, optional caption).
    pub files: Vec<(String, Vec<u8>, Option<String>)>,
}

/// Render a [`RichResponse`] for Discord — single embed + file attachments.
///
/// Native primitives: title→embed.title, footer→embed.footer, color→embed.color,
/// first Image→embed.image_url, File blocks→files. Everything else (headings,
/// code, lists, tables, text, quotes, dividers, extra images) is flattened
/// to text and joined as the embed description.
pub fn render_discord(r: &RichResponse) -> DiscordRender {
    use crate::discord::DiscordEmbed;

    let mut embed = DiscordEmbed::new();
    if let Some(ref t) = r.title {
        embed = embed.title(t);
    }
    if let Some(ref f) = r.footer {
        embed = embed.footer(f);
    }
    if let Some(c) = r.color {
        embed = embed.color(c);
    }

    let mut first_image: Option<String> = None;
    let mut files: Vec<(String, Vec<u8>, Option<String>)> = Vec::new();
    let mut desc_parts: Vec<String> = Vec::new();

    for block in &r.blocks {
        match block {
            ContentBlock::Image { url, alt, caption } => {
                if first_image.is_none() {
                    first_image = Some(url.clone());
                    if let Some(c) = caption {
                        desc_parts.push(format!("_{alt}_ — {c}"));
                    } else {
                        desc_parts.push(format!("_{alt}_"));
                    }
                } else {
                    // Subsequent images degrade to text link.
                    desc_parts.push(block.to_text());
                }
            }
            ContentBlock::File {
                filename,
                bytes,
                caption,
                ..
            } => {
                files.push((filename.clone(), bytes.clone(), caption.clone()));
            }
            _ => {
                desc_parts.push(block.to_text());
            }
        }
    }

    if let Some(url) = first_image {
        embed = embed.image(url);
    }
    let desc = desc_parts.join("\n\n");
    if !desc.is_empty() {
        embed = embed.description(desc);
    }

    let has_embed_content = r.title.is_some()
        || r.footer.is_some()
        || r.color.is_some()
        || !r.blocks.is_empty();

    DiscordRender {
        content: None,
        embed: if has_embed_content { Some(embed) } else { None },
        files,
    }
}

// ── Slack renderer ──────────────────────────────────────────────────────

/// Output of [`render_slack`].
///
/// Slack message = ordered Block Kit blocks + optional file uploads (uploaded
/// separately via `files.uploadV2`). We map most `ContentBlock` variants
/// directly onto Block Kit primitives.
#[derive(Debug, Clone, Default)]
pub struct SlackRender {
    /// Block Kit blocks, in order.
    pub blocks: Vec<crate::slack::Block>,
    /// Plain-text fallback (Slack requires this on every message).
    pub fallback_text: String,
    /// File uploads: (filename, bytes, optional caption).
    pub files: Vec<(String, Vec<u8>, Option<String>)>,
}

/// Render a [`RichResponse`] for Slack — Block Kit blocks + file uploads.
pub fn render_slack(r: &RichResponse) -> SlackRender {
    use crate::slack::{Block, TextObject};
    use crate::slack_mrkdwn::to_slack_mrkdwn;

    let mut blocks: Vec<Block> = Vec::new();
    let mut files: Vec<(String, Vec<u8>, Option<String>)> = Vec::new();

    if let Some(ref t) = r.title {
        blocks.push(Block::Header {
            text: TextObject::plain(t),
        });
    }

    for block in &r.blocks {
        match block {
            ContentBlock::Text { text } => {
                blocks.push(Block::Section {
                    text: TextObject::markdown(to_slack_mrkdwn(text)),
                    accessory: None,
                    fields: None,
                });
            }
            ContentBlock::Heading { level: _, text } => {
                blocks.push(Block::Header {
                    text: TextObject::plain(text),
                });
            }
            ContentBlock::Code { text, language } => {
                let body = match language {
                    Some(lang) => format!("```{lang}\n{text}\n```"),
                    None => format!("```\n{text}\n```"),
                };
                blocks.push(Block::Section {
                    text: TextObject::markdown(body),
                    accessory: None,
                    fields: None,
                });
            }
            ContentBlock::Divider => blocks.push(Block::Divider),
            ContentBlock::Image { url, alt, caption: _ } => {
                // Slack image block — use `image_url` via Section accessory
                // fallback to markdown link if Block enum lacks ImageBlock.
                // Conservative: emit markdown link in a section — works
                // across all Slack-block-kit versions in tree.
                blocks.push(Block::Section {
                    text: TextObject::markdown(block.to_text()),
                    accessory: None,
                    fields: None,
                });
                let _ = (url, alt);
            }
            ContentBlock::File {
                filename,
                bytes,
                caption,
                ..
            } => {
                files.push((filename.clone(), bytes.clone(), caption.clone()));
            }
            ContentBlock::Table { .. }
            | ContentBlock::List { .. }
            | ContentBlock::Quote { .. } => {
                // Degrade to a markdown section.
                blocks.push(Block::Section {
                    text: TextObject::markdown(to_slack_mrkdwn(&block.to_text())),
                    accessory: None,
                    fields: None,
                });
            }
        }
    }

    if let Some(ref f) = r.footer {
        blocks.push(Block::Context {
            elements: vec![TextObject::markdown(to_slack_mrkdwn(f))],
        });
    }

    SlackRender {
        blocks,
        fallback_text: r.to_text(),
        files,
    }
}

// ── Telegram renderer (markdown-text channel) ───────────────────────────

/// Output of [`render_telegram`]. Telegram outbound is currently text-only
/// in-tree (see design doc §1.3 TBD); we render as Telegram-flavored
/// markdown plus separate image-URL list that an MTProto-aware adapter
/// could use to `sendPhoto`.
#[derive(Debug, Clone, Default)]
pub struct TelegramRender {
    /// Markdown text body.
    pub text: String,
    /// Image URLs to attach (one `sendPhoto` per).
    pub image_urls: Vec<String>,
    /// File attachments.
    pub files: Vec<(String, Vec<u8>, Option<String>)>,
}

/// Render a [`RichResponse`] for Telegram.
pub fn render_telegram(r: &RichResponse) -> TelegramRender {
    let (_natives, text_part) = r.partition(|b| {
        matches!(
            b,
            ContentBlock::Image { .. } | ContentBlock::File { .. }
        )
    });
    let mut image_urls = Vec::new();
    let mut files = Vec::new();
    for b in &r.blocks {
        match b {
            ContentBlock::Image { url, .. } => image_urls.push(url.clone()),
            ContentBlock::File {
                filename,
                bytes,
                caption,
                ..
            } => files.push((filename.clone(), bytes.clone(), caption.clone())),
            _ => {}
        }
    }
    // Prepend title if set.
    let mut text = String::new();
    if let Some(ref t) = r.title {
        text.push_str(&format!("*{t}*\n\n"));
    }
    text.push_str(&text_part);
    if let Some(ref f) = r.footer {
        text.push_str(&format!("\n\n_{f}_"));
    }
    TelegramRender {
        text,
        image_urls,
        files,
    }
}

// ── WebUI renderer (HTML) ───────────────────────────────────────────────

/// Render a [`RichResponse`] as HTML for the WebUI channel.
pub fn render_webui_html(r: &RichResponse) -> String {
    let mut out = String::new();
    out.push_str("<div class=\"rich-response\">");
    if let Some(ref t) = r.title {
        out.push_str(&format!("<h2>{}</h2>", html_escape(t)));
    }
    for b in &r.blocks {
        out.push_str(&render_block_html(b));
    }
    if let Some(ref f) = r.footer {
        out.push_str(&format!(
            "<footer class=\"rich-footer\">{}</footer>",
            html_escape(f)
        ));
    }
    out.push_str("</div>");
    out
}

fn render_block_html(b: &ContentBlock) -> String {
    match b {
        ContentBlock::Text { text } => format!("<p>{}</p>", html_escape(text)),
        ContentBlock::Heading { level, text } => {
            let lvl = (*level).clamp(1, 6);
            format!("<h{lvl}>{}</h{lvl}>", html_escape(text))
        }
        ContentBlock::Code { text, language } => {
            let cls = language
                .as_ref()
                .map(|l| format!(" class=\"language-{l}\""))
                .unwrap_or_default();
            format!(
                "<pre><code{cls}>{}</code></pre>",
                html_escape(text)
            )
        }
        ContentBlock::Divider => "<hr/>".to_string(),
        ContentBlock::Image { url, alt, caption } => {
            let cap = caption
                .as_ref()
                .map(|c| format!("<figcaption>{}</figcaption>", html_escape(c)))
                .unwrap_or_default();
            format!(
                "<figure><img src=\"{}\" alt=\"{}\"/>{}</figure>",
                html_escape(url),
                html_escape(alt),
                cap
            )
        }
        ContentBlock::File {
            filename, caption, ..
        } => {
            let cap = caption.as_deref().unwrap_or("");
            format!(
                "<div class=\"file-attachment\">📎 {} {}</div>",
                html_escape(filename),
                html_escape(cap)
            )
        }
        ContentBlock::Table { headers, rows } => {
            let mut t = String::from("<table><thead><tr>");
            for h in headers {
                t.push_str(&format!("<th>{}</th>", html_escape(h)));
            }
            t.push_str("</tr></thead><tbody>");
            for row in rows {
                t.push_str("<tr>");
                for c in row {
                    t.push_str(&format!("<td>{}</td>", html_escape(c)));
                }
                t.push_str("</tr>");
            }
            t.push_str("</tbody></table>");
            t
        }
        ContentBlock::List { ordered, items } => {
            let tag = if *ordered { "ol" } else { "ul" };
            let mut s = format!("<{tag}>");
            for i in items {
                s.push_str(&format!("<li>{}</li>", html_escape(i)));
            }
            s.push_str(&format!("</{tag}>"));
            s
        }
        ContentBlock::Quote { text, source } => match source {
            Some(src) => format!(
                "<blockquote><p>{}</p><cite>{}</cite></blockquote>",
                html_escape(text),
                html_escape(src)
            ),
            None => format!("<blockquote><p>{}</p></blockquote>", html_escape(text)),
        },
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ── CLI renderer (full text degradation) ────────────────────────────────

/// Render for CLI/plain-text channels — full degradation to text.
pub fn render_cli(r: &RichResponse) -> String {
    r.to_text()
}

// ── Capability-driven dispatch helper ───────────────────────────────────

/// Render kind selector for capability-aware dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderKind {
    Native,
    MarkdownText,
    PlainText,
}

/// Decide which renderer to use given the adapter's capabilities.
pub fn select_render_kind(caps: ChannelCapabilities) -> RenderKind {
    if caps.rich_content {
        RenderKind::Native
    } else if caps.markdown {
        RenderKind::MarkdownText
    } else {
        RenderKind::PlainText
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rich::ContentBlock;

    fn sample_response() -> RichResponse {
        RichResponse::new()
            .title("Status")
            .color(0x00ff00)
            .heading(1, "Header")
            .text("body text")
            .image("https://x.com/a.png", "alt", Some("cap".into()))
            .footer("foot")
    }

    // ── Discord ───────────────────────────────────────────────────

    #[test]
    fn discord_render_emits_embed() {
        let r = sample_response();
        let out = render_discord(&r);
        let embed = out.embed.expect("embed present");
        assert_eq!(embed.title.as_deref(), Some("Status"));
        assert_eq!(embed.footer.as_deref(), Some("foot"));
        assert_eq!(embed.color, Some(0x00ff00));
        assert_eq!(embed.image_url.as_deref(), Some("https://x.com/a.png"));
        let desc = embed.description.unwrap_or_default();
        assert!(desc.contains("Header"));
        assert!(desc.contains("body text"));
    }

    #[test]
    fn discord_render_empty_response_no_embed() {
        let r = RichResponse::new();
        let out = render_discord(&r);
        assert!(out.embed.is_none());
        assert!(out.files.is_empty());
    }

    #[test]
    fn discord_second_image_degrades_to_text() {
        let r = RichResponse::new()
            .image("https://a.com/1.png", "first", None)
            .image("https://b.com/2.png", "second", None);
        let out = render_discord(&r);
        let embed = out.embed.unwrap();
        assert_eq!(
            embed.image_url.as_deref(),
            Some("https://a.com/1.png"),
            "first image is native"
        );
        assert!(
            embed.description.unwrap_or_default().contains("second"),
            "second image degrades to description text"
        );
    }

    #[test]
    fn discord_file_block_emits_file_attachment() {
        let r = RichResponse::new().block(ContentBlock::File {
            filename: "x.pdf".into(),
            bytes: vec![1, 2, 3],
            caption: Some("cap".into()),
            mime_type: None,
        });
        let out = render_discord(&r);
        assert_eq!(out.files.len(), 1);
        assert_eq!(out.files[0].0, "x.pdf");
        assert_eq!(out.files[0].1, vec![1, 2, 3]);
    }

    // ── Slack ─────────────────────────────────────────────────────

    #[test]
    fn slack_render_uses_header_for_title() {
        use crate::slack::Block;
        let r = sample_response();
        let out = render_slack(&r);
        assert!(matches!(out.blocks.first(), Some(Block::Header { .. })));
        assert!(out.fallback_text.contains("Header"));
    }

    #[test]
    fn slack_render_emits_divider() {
        use crate::slack::Block;
        let r = RichResponse::new().divider();
        let out = render_slack(&r);
        assert!(out.blocks.iter().any(|b| matches!(b, Block::Divider)));
    }

    #[test]
    fn slack_render_footer_as_context() {
        use crate::slack::Block;
        let r = RichResponse::new().text("body").footer("foot");
        let out = render_slack(&r);
        // Last block should be the context with footer
        let last = out.blocks.last().unwrap();
        assert!(matches!(last, Block::Context { .. }));
    }

    #[test]
    fn slack_render_table_degrades_to_section() {
        let r = RichResponse::new().block(ContentBlock::Table {
            headers: vec!["A".into()],
            rows: vec![vec!["1".into()]],
        });
        let out = render_slack(&r);
        assert!(!out.blocks.is_empty());
        assert!(out.fallback_text.contains("A"));
    }

    // ── Telegram ──────────────────────────────────────────────────

    #[test]
    fn telegram_extracts_images_separately() {
        let r = RichResponse::new()
            .text("hello")
            .image("https://x.com/a.png", "alt", None);
        let out = render_telegram(&r);
        assert_eq!(out.image_urls.len(), 1);
        assert!(out.text.contains("hello"));
        // Text part should NOT include the image (it's extracted)
        assert!(!out.text.contains("https://x.com/a.png"));
    }

    #[test]
    fn telegram_title_renders_as_bold_markdown() {
        let r = RichResponse::new().title("T").text("body");
        let out = render_telegram(&r);
        assert!(out.text.starts_with("*T*"));
    }

    // ── WebUI ─────────────────────────────────────────────────────

    #[test]
    fn webui_renders_html_structure() {
        let r = sample_response();
        let html = render_webui_html(&r);
        assert!(html.contains("<h2>Status</h2>"));
        assert!(html.contains("<h1>Header</h1>"));
        assert!(html.contains("<figure>"));
        assert!(html.contains("<img src=\"https://x.com/a.png\""));
        assert!(html.contains("alt=\"alt\""));
        assert!(html.contains("<footer"));
    }

    #[test]
    fn webui_escapes_html_in_text() {
        let r = RichResponse::new().text("<script>alert(1)</script>");
        let html = render_webui_html(&r);
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn webui_table_renders_thead_tbody() {
        let r = RichResponse::new().block(ContentBlock::Table {
            headers: vec!["A".into(), "B".into()],
            rows: vec![vec!["1".into(), "2".into()]],
        });
        let html = render_webui_html(&r);
        assert!(html.contains("<thead>"));
        assert!(html.contains("<tbody>"));
        assert!(html.contains("<th>A</th>"));
        assert!(html.contains("<td>2</td>"));
    }

    #[test]
    fn webui_list_ordered_vs_unordered() {
        let r1 = RichResponse::new().block(ContentBlock::List {
            ordered: true,
            items: vec!["a".into()],
        });
        let r2 = RichResponse::new().block(ContentBlock::List {
            ordered: false,
            items: vec!["a".into()],
        });
        assert!(render_webui_html(&r1).contains("<ol>"));
        assert!(render_webui_html(&r2).contains("<ul>"));
    }

    // ── CLI ───────────────────────────────────────────────────────

    #[test]
    fn cli_full_degradation() {
        let r = sample_response();
        let s = render_cli(&r);
        assert!(s.contains("Status"));
        assert!(s.contains("Header"));
        assert!(s.contains("body text"));
    }

    // ── Capability dispatch ───────────────────────────────────────

    #[test]
    fn select_render_kind_tier_1_native() {
        assert_eq!(
            select_render_kind(ChannelCapabilities::TIER_1),
            RenderKind::Native
        );
    }

    #[test]
    fn select_render_kind_markdown_text() {
        assert_eq!(
            select_render_kind(ChannelCapabilities::MARKDOWN_TEXT),
            RenderKind::MarkdownText
        );
    }

    #[test]
    fn select_render_kind_plain_text() {
        assert_eq!(
            select_render_kind(ChannelCapabilities::PLAIN_TEXT),
            RenderKind::PlainText
        );
    }
}
