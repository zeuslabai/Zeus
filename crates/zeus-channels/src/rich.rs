//! Gateway-mediated rich multimedia responses.
//!
//! Sprint #85 / Issue #88 — implements the channel-agnostic intermediate
//! format defined in `docs/design/gateway-rich-multimedia-responses-2026-05-22.md`.
//!
//! # Architecture
//!
//! Per banked discipline `universal-features-route-through-gateway-not-per-channel-adapter`:
//! the gateway constructs a [`RichResponse`] from a sequence of [`ContentBlock`]
//! fragments, and each channel adapter renders it via [`ChannelAdapter::send_rich`]
//! (provided as a default impl that degrades to plain text).
//!
//! Channel adapters with native rich-content support (Discord embeds, Slack
//! Block Kit) override `send_rich` to produce native output. Adapters without
//! support inherit the default text-degradation path automatically.
//!
//! # Capability negotiation
//!
//! Adapters advertise their capabilities via [`ChannelCapabilities`], which the
//! gateway uses to decide whether to attempt rich rendering or pre-flatten.
//!
//! # Graceful degradation
//!
//! Every [`ContentBlock`] knows how to render itself as plain text via
//! [`ContentBlock::to_text`]. This is the canonical fallback used by:
//! - The default [`ChannelAdapter::send_rich`] impl.
//! - Native renderers that encounter a fragment they cannot represent (e.g.
//!   CLI hitting an inline image — text caption is emitted instead).

use serde::{Deserialize, Serialize};

// ── Capabilities ────────────────────────────────────────────────────────

/// What a channel adapter can natively render. Used by the gateway to decide
/// whether to attempt rich-content dispatch or pre-degrade to plain text.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ChannelCapabilities {
    /// Can render block-structured rich content (Discord embeds, Slack blocks,
    /// HTML, etc.). When `false`, gateway should pre-flatten to text.
    pub rich_content: bool,
    /// Supports inline image rendering by URL (Discord embeds, Slack image
    /// blocks, HTML img tags).
    pub inline_images: bool,
    /// Supports file attachments (uploaded bytes with filename + caption).
    pub file_attachments: bool,
    /// Supports markdown formatting in text content.
    pub markdown: bool,
    /// Supports tables (either natively as columns, or as a rendered grid).
    pub tables: bool,
    /// Supports thread-aware reply ordering.
    pub threading: bool,
}

impl ChannelCapabilities {
    /// Lowest-common-denominator capability set: plain text only.
    pub const PLAIN_TEXT: Self = Self {
        rich_content: false,
        inline_images: false,
        file_attachments: false,
        markdown: false,
        tables: false,
        threading: false,
    };

    /// Tier-1 capability set for full-featured chat platforms (Discord, Slack).
    pub const TIER_1: Self = Self {
        rich_content: true,
        inline_images: true,
        file_attachments: true,
        markdown: true,
        tables: true,
        threading: true,
    };

    /// Markdown-aware text-only channel (Matrix-text, Telegram-text-only).
    pub const MARKDOWN_TEXT: Self = Self {
        rich_content: false,
        inline_images: false,
        file_attachments: false,
        markdown: true,
        tables: true,
        threading: false,
    };
}

// ── Content blocks ──────────────────────────────────────────────────────

/// A single fragment of a rich response. The gateway composes a `RichResponse`
/// from an ordered sequence of these.
///
/// Each variant is renderable by every channel adapter — adapters with native
/// support for a primitive use it, others degrade via [`ContentBlock::to_text`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text paragraph.
    Text { text: String },
    /// Heading. `level` is 1-6 (h1..h6).
    Heading { level: u8, text: String },
    /// Inline code block, optionally with a language hint for syntax highlighting.
    Code {
        text: String,
        language: Option<String>,
    },
    /// Visual divider (horizontal rule).
    Divider,
    /// Image referenced by URL. `alt` is mandatory for accessibility +
    /// text-degradation captioning.
    Image {
        url: String,
        alt: String,
        caption: Option<String>,
    },
    /// File attachment uploaded as raw bytes. The adapter is responsible for
    /// uploading via the channel's native file-upload API.
    File {
        filename: String,
        bytes: Vec<u8>,
        caption: Option<String>,
        mime_type: Option<String>,
    },
    /// Tabular data. `headers.len()` should equal each row's len.
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// Bullet or numbered list.
    List { ordered: bool, items: Vec<String> },
    /// Quoted block (e.g. for citations or LLM-quoted sources).
    Quote { text: String, source: Option<String> },
}

impl ContentBlock {
    /// Convenience constructor.
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text { text: s.into() }
    }

    /// Convenience constructor.
    pub fn heading(level: u8, s: impl Into<String>) -> Self {
        Self::Heading {
            level: level.clamp(1, 6),
            text: s.into(),
        }
    }

    /// Convenience constructor.
    pub fn code(text: impl Into<String>, lang: Option<impl Into<String>>) -> Self {
        Self::Code {
            text: text.into(),
            language: lang.map(Into::into),
        }
    }

    /// Convenience constructor.
    pub fn image(
        url: impl Into<String>,
        alt: impl Into<String>,
        caption: Option<impl Into<String>>,
    ) -> Self {
        Self::Image {
            url: url.into(),
            alt: alt.into(),
            caption: caption.map(Into::into),
        }
    }

    /// Plain-text rendering — the canonical graceful-degradation path.
    ///
    /// Every adapter ultimately falls back to this when it cannot render a
    /// fragment natively.
    pub fn to_text(&self) -> String {
        match self {
            Self::Text { text } => text.clone(),
            Self::Heading { level, text } => {
                let prefix = "#".repeat(*level as usize);
                format!("{prefix} {text}")
            }
            Self::Code { text, language } => match language {
                Some(lang) => format!("```{lang}\n{text}\n```"),
                None => format!("```\n{text}\n```"),
            },
            Self::Divider => "───".to_string(),
            Self::Image { url, alt, caption } => match caption {
                Some(c) => format!("🖼️ [{alt}]({url}) — {c}"),
                None => format!("🖼️ [{alt}]({url})"),
            },
            Self::File {
                filename, caption, ..
            } => match caption {
                Some(c) => format!("📎 {filename} — {c}"),
                None => format!("📎 {filename}"),
            },
            Self::Table { headers, rows } => render_table_text(headers, rows),
            Self::List { ordered, items } => {
                let mut out = String::new();
                for (i, item) in items.iter().enumerate() {
                    let bullet = if *ordered {
                        format!("{}.", i + 1)
                    } else {
                        "•".to_string()
                    };
                    if i > 0 {
                        out.push('\n');
                    }
                    out.push_str(&format!("{bullet} {item}"));
                }
                out
            }
            Self::Quote { text, source } => match source {
                Some(s) => format!("> {text}\n— {s}"),
                None => format!("> {text}"),
            },
        }
    }
}

/// Render a table as a fixed-width ASCII grid suitable for plain-text channels.
fn render_table_text(headers: &[String], rows: &[Vec<String>]) -> String {
    if headers.is_empty() {
        return String::new();
    }
    // Compute column widths.
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() && cell.len() > widths[i] {
                widths[i] = cell.len();
            }
        }
    }
    let render_row = |cells: &[String]| -> String {
        let parts: Vec<String> = cells
            .iter()
            .zip(widths.iter())
            .map(|(c, w)| format!("{:width$}", c, width = w))
            .collect();
        format!("| {} |", parts.join(" | "))
    };
    let sep: String = format!(
        "|{}|",
        widths
            .iter()
            .map(|w| "-".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("|")
    );
    let mut out = String::new();
    out.push_str(&render_row(headers));
    out.push('\n');
    out.push_str(&sep);
    for row in rows {
        out.push('\n');
        out.push_str(&render_row(row));
    }
    out
}

// ── RichResponse ────────────────────────────────────────────────────────

/// A complete rich response composed of ordered content blocks.
///
/// The gateway constructs this; channel adapters render it. The same response
/// renders correctly on every channel — natively where supported, degraded to
/// plain text otherwise.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RichResponse {
    /// Optional response title (Discord embed title / Slack header block).
    pub title: Option<String>,
    /// Optional color hint as 0xRRGGBB (used by Discord embeds, Slack
    /// attachments). Adapters without color support ignore.
    pub color: Option<u32>,
    /// Optional footer (Discord embed footer / Slack context block).
    pub footer: Option<String>,
    /// Ordered content fragments.
    pub blocks: Vec<ContentBlock>,
}

impl RichResponse {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, t: impl Into<String>) -> Self {
        self.title = Some(t.into());
        self
    }

    pub fn color(mut self, c: u32) -> Self {
        self.color = Some(c);
        self
    }

    pub fn footer(mut self, f: impl Into<String>) -> Self {
        self.footer = Some(f.into());
        self
    }

    pub fn block(mut self, b: ContentBlock) -> Self {
        self.blocks.push(b);
        self
    }

    pub fn text(self, s: impl Into<String>) -> Self {
        self.block(ContentBlock::text(s))
    }

    pub fn heading(self, level: u8, s: impl Into<String>) -> Self {
        self.block(ContentBlock::heading(level, s))
    }

    pub fn divider(self) -> Self {
        self.block(ContentBlock::Divider)
    }

    pub fn image(
        self,
        url: impl Into<String>,
        alt: impl Into<String>,
        caption: Option<String>,
    ) -> Self {
        self.block(ContentBlock::Image {
            url: url.into(),
            alt: alt.into(),
            caption,
        })
    }

    /// Canonical full-degradation: render the whole response as plain text.
    /// Used by adapters with `ChannelCapabilities::PLAIN_TEXT` or as the
    /// default-impl fallback for `ChannelAdapter::send_rich`.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        if let Some(ref t) = self.title {
            out.push_str(&format!("**{t}**\n\n"));
        }
        for (i, b) in self.blocks.iter().enumerate() {
            if i > 0 {
                out.push_str("\n\n");
            }
            out.push_str(&b.to_text());
        }
        if let Some(ref f) = self.footer {
            out.push_str(&format!("\n\n_{f}_"));
        }
        out
    }

    /// Partial degradation: filter out fragments the adapter cannot handle,
    /// keeping the rest. Returns text-form for filtered blocks. Used by
    /// renderers that natively support *some* but not all primitives.
    ///
    /// `keep` returns true for blocks the adapter renders natively; the rest
    /// are flattened to text and merged into adjacent text blocks.
    pub fn partition<F>(&self, keep: F) -> (Vec<&ContentBlock>, String)
    where
        F: Fn(&ContentBlock) -> bool,
    {
        let mut native = Vec::new();
        let mut degraded = String::new();
        for b in &self.blocks {
            if keep(b) {
                native.push(b);
            } else {
                if !degraded.is_empty() {
                    degraded.push_str("\n\n");
                }
                degraded.push_str(&b.to_text());
            }
        }
        (native, degraded)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Per-fragment text degradation ─────────────────────────────

    #[test]
    fn text_block_to_text() {
        assert_eq!(ContentBlock::text("hello").to_text(), "hello");
    }

    #[test]
    fn heading_block_to_text() {
        assert_eq!(
            ContentBlock::heading(2, "Title").to_text(),
            "## Title"
        );
    }

    #[test]
    fn heading_level_clamped() {
        // level > 6 clamps to 6
        assert_eq!(
            ContentBlock::heading(9, "T").to_text(),
            "###### T"
        );
        // level 0 clamps to 1
        assert_eq!(
            ContentBlock::heading(0, "T").to_text(),
            "# T"
        );
    }

    #[test]
    fn code_block_with_lang() {
        let out = ContentBlock::code("fn x() {}", Some("rust")).to_text();
        assert!(out.starts_with("```rust"));
        assert!(out.contains("fn x() {}"));
    }

    #[test]
    fn code_block_no_lang() {
        let out = ContentBlock::code("plain", None::<String>).to_text();
        assert!(out.starts_with("```\n"));
    }

    #[test]
    fn divider_block_to_text() {
        assert_eq!(ContentBlock::Divider.to_text(), "───");
    }

    #[test]
    fn image_block_to_text_with_caption() {
        let img = ContentBlock::image(
            "https://example.com/a.png",
            "alt",
            Some("caption"),
        );
        let s = img.to_text();
        assert!(s.contains("alt"));
        assert!(s.contains("https://example.com/a.png"));
        assert!(s.contains("caption"));
    }

    #[test]
    fn image_block_to_text_no_caption() {
        let img =
            ContentBlock::image("https://x.com/y.png", "alt", None::<String>);
        let s = img.to_text();
        assert!(s.contains("alt"));
        assert!(!s.contains("—"));
    }

    #[test]
    fn file_block_to_text() {
        let f = ContentBlock::File {
            filename: "report.pdf".to_string(),
            bytes: vec![0, 1, 2],
            caption: Some("Q4".to_string()),
            mime_type: Some("application/pdf".to_string()),
        };
        assert!(f.to_text().contains("report.pdf"));
        assert!(f.to_text().contains("Q4"));
    }

    #[test]
    fn list_unordered_to_text() {
        let l = ContentBlock::List {
            ordered: false,
            items: vec!["a".into(), "b".into()],
        };
        let s = l.to_text();
        assert!(s.contains("• a"));
        assert!(s.contains("• b"));
    }

    #[test]
    fn list_ordered_to_text() {
        let l = ContentBlock::List {
            ordered: true,
            items: vec!["a".into(), "b".into()],
        };
        let s = l.to_text();
        assert!(s.contains("1. a"));
        assert!(s.contains("2. b"));
    }

    #[test]
    fn quote_to_text() {
        let q = ContentBlock::Quote {
            text: "x".into(),
            source: Some("y".into()),
        };
        let s = q.to_text();
        assert!(s.starts_with("> x"));
        assert!(s.contains("— y"));
    }

    #[test]
    fn table_to_text_alignment() {
        let t = ContentBlock::Table {
            headers: vec!["Name".into(), "Status".into()],
            rows: vec![
                vec!["zeus100".into(), "ok".into()],
                vec!["zeus112".into(), "active".into()],
            ],
        };
        let s = t.to_text();
        // Header row + separator + 2 data rows = 4 lines
        assert_eq!(s.lines().count(), 4);
        assert!(s.contains("Name"));
        assert!(s.contains("zeus112"));
    }

    // ── RichResponse aggregation ──────────────────────────────────

    #[test]
    fn rich_response_to_text_composition() {
        let r = RichResponse::new()
            .title("Test")
            .heading(1, "H")
            .text("body")
            .footer("foot");
        let s = r.to_text();
        assert!(s.starts_with("**Test**"));
        assert!(s.contains("# H"));
        assert!(s.contains("body"));
        assert!(s.ends_with("_foot_"));
    }

    #[test]
    fn rich_response_empty_renders_empty() {
        let r = RichResponse::new();
        assert_eq!(r.to_text(), "");
    }

    // ── Partition / partial degradation ───────────────────────────

    #[test]
    fn partition_keeps_natives_degrades_rest() {
        let r = RichResponse::new()
            .text("hello")
            .image("u", "alt", None)
            .text("world");
        // Adapter that supports only text blocks
        let (native, degraded) = r.partition(|b| matches!(b, ContentBlock::Text { .. }));
        assert_eq!(native.len(), 2);
        assert!(degraded.contains("alt"));
        assert!(degraded.contains("u"));
    }

    #[test]
    fn partition_no_keepers_returns_empty_native() {
        let r = RichResponse::new().text("x");
        let (native, degraded) = r.partition(|_| false);
        assert!(native.is_empty());
        assert_eq!(degraded, "x");
    }

    #[test]
    fn partition_all_keepers_returns_empty_degraded() {
        let r = RichResponse::new().text("x").text("y");
        let (native, degraded) = r.partition(|_| true);
        assert_eq!(native.len(), 2);
        assert!(degraded.is_empty());
    }

    // ── Capabilities presets ──────────────────────────────────────

    #[test]
    fn capability_presets_consistent() {
        assert!(!ChannelCapabilities::PLAIN_TEXT.rich_content);
        assert!(!ChannelCapabilities::PLAIN_TEXT.inline_images);
        assert!(ChannelCapabilities::TIER_1.rich_content);
        assert!(ChannelCapabilities::TIER_1.inline_images);
        assert!(ChannelCapabilities::MARKDOWN_TEXT.markdown);
        assert!(!ChannelCapabilities::MARKDOWN_TEXT.inline_images);
    }

    // ── Serde round-trip ──────────────────────────────────────────

    #[test]
    fn rich_response_json_round_trip() {
        let r = RichResponse::new()
            .title("T")
            .color(0xff0000)
            .heading(1, "H")
            .text("body")
            .image("u", "alt", Some("cap".into()));
        let json = serde_json::to_string(&r).expect("serialize");
        let back: RichResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(r, back);
    }

    // ── Per-channel renderer integration (helpers) ────────────────

    #[test]
    fn discord_capability_check() {
        // Discord adapter should advertise TIER_1 caps
        let caps = ChannelCapabilities::TIER_1;
        assert!(caps.rich_content && caps.inline_images && caps.file_attachments);
    }

    #[test]
    fn cli_degradation_path() {
        // CLI (PLAIN_TEXT) should fall back fully to text
        let r = RichResponse::new()
            .heading(1, "H")
            .image("u", "alt", None);
        // Simulated CLI render: just to_text()
        let s = r.to_text();
        assert!(s.contains("# H"));
        assert!(s.contains("alt"));
    }
}
