//! Markdown → ratatui renderer for the TUI chat pane (#280).
//!
//! There is no shared TUI markdown renderer — ZeusWeb's `markdown.rs` emits an
//! HTML *string* (`pulldown_cmark::html`), which is unusable in a terminal where
//! ratatui needs `Vec<Line<'static>>` of styled `Span`s. This module is the
//! from-scratch TUI equivalent: it drives `pulldown_cmark::Parser` (with the
//! same options as ZeusWeb — TABLES + STRIKETHROUGH + TASKLISTS, for
//! cross-surface consistency) and maps the event stream to width-aware,
//! span-style-preserving lines.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme;

/// A styled word, used by the span-preserving wrapper.
struct StyledWord {
    text: String,
    style: Style,
}

/// Count the leading-pipe table cells in a line, returning `None` if the line
/// isn't a pipe-table row (no `|` outside of code).
fn pipe_cell_count(line: &str) -> Option<usize> {
    let t = line.trim();
    if !t.contains('|') {
        return None;
    }
    // A row needs at least one `|`. Strip optional leading/trailing pipes, then
    // count the segments. `a | b | c`, `| a | b |`, and `a | b` all qualify.
    let inner = t.trim_start_matches('|').trim_end_matches('|');
    Some(inner.split('|').count())
}

/// Is `line` already a CommonMark table delimiter row? e.g. `|---|:--:|---:|`.
/// Every cell must consist solely of `-`, `:`, and whitespace, with ≥1 dash.
fn is_delimiter_row(line: &str) -> bool {
    let t = line.trim();
    if !t.contains('-') || !t.contains('|') {
        return false;
    }
    let inner = t.trim_start_matches('|').trim_end_matches('|');
    inner.split('|').all(|cell| {
        let c = cell.trim();
        !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':')
    }) && inner.contains('-')
}

/// Pre-process markdown so delimiter-less pipe tables still render as grids.
///
/// pulldown-cmark (per the CommonMark table extension) only recognises a table
/// when the header row is immediately followed by a `|---|---|` delimiter row.
/// Many LLMs emit the header + data rows but omit that separator — pulldown then
/// degrades the whole block to paragraphs and each `|`-laden line gets
/// word-split into pipe-soup (see #280 repro). This walks the source and, when
/// it finds ≥2 consecutive pipe-rows whose second line is *not* a delimiter,
/// synthesises a delimiter row (matching the header's cell count) right after
/// the header so the parser emits a real `Tag::Table`.
///
/// Lines inside fenced code blocks are left untouched.
fn inject_table_delimiters(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 4);
    let mut in_fence = false;
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        // Track fenced code blocks — never touch their contents.
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push(line.to_string());
            i += 1;
            continue;
        }
        if in_fence {
            out.push(line.to_string());
            i += 1;
            continue;
        }

        // Candidate header: this line is a pipe-row AND the next line is also a
        // pipe-row that is NOT already a delimiter. That's the delimiter-less
        // table shape we repair. Skip if THIS line is itself a delimiter row —
        // otherwise a valid `header / |---| / data` table would get a second
        // delimiter injected after its real one.
        if let Some(header_cells) = pipe_cell_count(line).filter(|_| !is_delimiter_row(line)) {
            let next_is_pipe_nondelim = lines
                .get(i + 1)
                .map(|n| pipe_cell_count(n).is_some() && !is_delimiter_row(n))
                .unwrap_or(false);
            if header_cells >= 2 && next_is_pipe_nondelim {
                out.push(line.to_string());
                // Synthesise a delimiter row matching the header's cell count.
                let delim = std::iter::repeat("---")
                    .take(header_cells)
                    .collect::<Vec<_>>()
                    .join(" | ");
                out.push(format!("| {delim} |"));
                i += 1;
                continue;
            }
        }

        out.push(line.to_string());
        i += 1;
    }

    out.join("\n")
}

/// Render markdown `text` into terminal lines bounded to `width` columns.
///
/// `base` is the default foreground colour (the caller's bubble colour) applied
/// to plain prose. Headings/code/etc. layer their own styling on top.
pub fn render_markdown(text: &str, width: u16, base: Color) -> Vec<Line<'static>> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);

    let normalized = inject_table_delimiters(text);
    let parser = Parser::new_ext(&normalized, opts);
    let mut r = Renderer::new(width, base);
    for event in parser {
        r.handle(event);
    }
    r.finish()
}

/// Inline style stack + line accumulator driving the event walk.
struct Renderer {
    width: u16,
    base: Color,
    lines: Vec<Line<'static>>,
    /// Words accumulated for the current (not-yet-wrapped) block.
    pending: Vec<StyledWord>,
    /// Active inline modifiers (bold/italic/strike), layered.
    modifier: Modifier,
    /// Foreground override (headings, links), `None` = base.
    fg_override: Option<Color>,
    /// Inline-code styling active.
    in_inline_code: bool,
    /// List nesting: each entry is the next ordered index, or `None` for bullet.
    list_stack: Vec<Option<u64>>,
    /// Block-quote nesting depth.
    quote_depth: usize,
    /// Fenced/indented code-block buffer (raw text); `Some` while inside one.
    code_block: Option<String>,
    /// Table rows being assembled (each row = cells = plain text).
    table: Option<TableState>,
}

struct TableState {
    rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    cell_buf: String,
}

impl Renderer {
    fn new(width: u16, base: Color) -> Self {
        Self {
            width: width.max(8),
            base,
            lines: Vec::new(),
            pending: Vec::new(),
            modifier: Modifier::empty(),
            fg_override: None,
            in_inline_code: false,
            list_stack: Vec::new(),
            quote_depth: 0,
            code_block: None,
            table: None,
        }
    }

    fn cur_style(&self) -> Style {
        let fg = if self.in_inline_code {
            theme::DIM
        } else {
            self.fg_override.unwrap_or(self.base)
        };
        Style::default().fg(fg).add_modifier(self.modifier)
    }

    fn push_word(&mut self, text: String) {
        let style = self.cur_style();
        self.pending.push(StyledWord { text, style });
    }

    /// Continuation-indent prefix string for the current block context.
    fn indent_prefix(&self) -> String {
        let mut p = String::new();
        for _ in 0..self.quote_depth {
            p.push_str("▏ ");
        }
        // List items get hanging indent matching their marker width.
        for _ in 0..self.list_stack.len() {
            p.push_str("  ");
        }
        p
    }

    /// Flush the pending words as one or more wrapped lines, with an optional
    /// first-line `marker` (list bullet / number) and hanging indent for wraps.
    fn flush_block(&mut self, marker: Option<String>) {
        if self.pending.is_empty() && marker.is_none() {
            return;
        }
        let base_indent = self.indent_prefix();
        let quote_style = Style::default().fg(theme::DIM);
        let marker_style = Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD);
        let words = std::mem::take(&mut self.pending);
        let avail = self.width as usize;
        let marker_w = marker.as_ref().map(|m| m.chars().count()).unwrap_or(0);

        // Seed a fresh visual line with indent + marker (first line) or hanging
        // pad (wrapped lines). Returns (spans, prefix_len, has_word=false).
        let open_line = |first: bool| -> (Vec<Span<'static>>, usize) {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut len = 0usize;
            if !base_indent.is_empty() {
                spans.push(Span::styled(base_indent.clone(), quote_style));
                len += base_indent.chars().count();
            }
            if first {
                if let Some(m) = &marker {
                    spans.push(Span::styled(m.clone(), marker_style));
                    len += marker_w;
                }
            } else if marker_w > 0 {
                spans.push(Span::raw(" ".repeat(marker_w)));
                len += marker_w;
            }
            (spans, len)
        };

        let (mut cur_spans, mut cur_len) = open_line(true);
        let mut prefix_len = cur_len;
        let mut has_word = false;

        for w in words {
            let wlen = w.text.chars().count();
            let sep = if has_word { 1 } else { 0 };
            if cur_len + sep + wlen > avail && has_word {
                self.lines.push(Line::from(std::mem::take(&mut cur_spans)));
                let (s, l) = open_line(false);
                cur_spans = s;
                cur_len = l;
                prefix_len = l;
                has_word = false;
            }
            if has_word {
                cur_spans.push(Span::raw(" "));
                cur_len += 1;
            }
            cur_spans.push(Span::styled(w.text, w.style));
            cur_len += wlen;
            has_word = true;
        }
        let _ = prefix_len;
        if has_word || marker.is_some() {
            self.lines.push(Line::from(cur_spans));
        }
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(t.into_string()),
            Event::Code(t) => {
                // Inline code span.
                let was = self.in_inline_code;
                self.in_inline_code = true;
                self.push_word(t.into_string());
                self.in_inline_code = was;
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(cb) = self.code_block.as_mut() {
                    cb.push('\n');
                }
            }
            Event::Rule => {
                self.flush_block(None);
                let dash: String = "─".repeat(self.width as usize);
                self.lines
                    .push(Line::from(Span::styled(dash, Style::default().fg(theme::DIM))));
            }
            Event::TaskListMarker(done) => {
                let mark = if done { "[x] " } else { "[ ] " };
                self.push_word(mark.to_string());
            }
            _ => {}
        }
    }

    fn text(&mut self, t: String) {
        if let Some(cb) = self.code_block.as_mut() {
            cb.push_str(&t);
            return;
        }
        if self.table.is_some() {
            if let Some(tb) = self.table.as_mut() {
                tb.cell_buf.push_str(&t);
            }
            return;
        }
        for word in t.split_whitespace() {
            self.push_word(word.to_string());
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush_block(None);
                self.fg_override = Some(heading_color(level));
                self.modifier |= Modifier::BOLD;
            }
            Tag::BlockQuote(_) => {
                self.flush_block(None);
                self.quote_depth += 1;
            }
            Tag::CodeBlock(_) => {
                self.flush_block(None);
                self.code_block = Some(String::new());
            }
            Tag::List(start) => {
                self.flush_block(None);
                self.list_stack.push(start);
            }
            Tag::Item => {}
            Tag::Emphasis => self.modifier |= Modifier::ITALIC,
            Tag::Strong => self.modifier |= Modifier::BOLD,
            Tag::Strikethrough => self.modifier |= Modifier::CROSSED_OUT,
            Tag::Link { .. } => self.fg_override = Some(theme::CYAN),
            Tag::Table(_) => {
                self.flush_block(None);
                self.table = Some(TableState {
                    rows: Vec::new(),
                    current_row: Vec::new(),
                    cell_buf: String::new(),
                });
            }
            Tag::TableHead | Tag::TableRow => {}
            Tag::TableCell => {}
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush_block(None),
            TagEnd::Heading(_) => {
                self.flush_block(None);
                self.fg_override = None;
                self.modifier.remove(Modifier::BOLD);
            }
            TagEnd::BlockQuote(_) => {
                self.flush_block(None);
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::CodeBlock => self.flush_code_block(),
            TagEnd::List(_) => {
                self.list_stack.pop();
            }
            TagEnd::Item => {
                // Compute the marker for this item, then flush its content.
                let marker = match self.list_stack.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{}. ", *n);
                        *n += 1;
                        m
                    }
                    Some(None) => "• ".to_string(),
                    None => String::new(),
                };
                self.flush_block(Some(marker));
            }
            TagEnd::Emphasis => self.modifier.remove(Modifier::ITALIC),
            TagEnd::Strong => self.modifier.remove(Modifier::BOLD),
            TagEnd::Strikethrough => self.modifier.remove(Modifier::CROSSED_OUT),
            TagEnd::Link => self.fg_override = None,
            TagEnd::Table => self.flush_table(),
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(tb) = self.table.as_mut() {
                    let row = std::mem::take(&mut tb.current_row);
                    if !row.is_empty() {
                        tb.rows.push(row);
                    }
                }
            }
            TagEnd::TableCell => {
                if let Some(tb) = self.table.as_mut() {
                    let cell = std::mem::take(&mut tb.cell_buf);
                    tb.current_row.push(cell.trim().to_string());
                }
            }
            _ => {}
        }
    }

    /// Emit the fenced code block: subtle bg fill + dim fg, per-line so it reads
    /// as a contiguous block (bg painted across the full width).
    fn flush_code_block(&mut self) {
        let Some(code) = self.code_block.take() else {
            return;
        };
        let code = code.strip_suffix('\n').unwrap_or(&code);
        let bg = theme::BG_HIGHLIGHT;
        let style = Style::default().fg(theme::DIM).bg(bg);
        let width = self.width as usize;
        for raw in code.split('\n') {
            // Pad each line to full width so the bg fill reads as a block.
            let mut s = String::from(" ");
            s.push_str(raw);
            let len = s.chars().count();
            if len < width {
                s.push_str(&" ".repeat(width - len));
            }
            self.lines.push(Line::from(Span::styled(s, style)));
        }
    }

    /// Emit a column-aligned table bounded to width.
    fn flush_table(&mut self) {
        let Some(tb) = self.table.take() else {
            return;
        };
        if tb.rows.is_empty() {
            return;
        }
        let cols = tb.rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut widths = vec![0usize; cols];
        for row in &tb.rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
        // Clamp total width to terminal.
        let style = Style::default().fg(self.base);
        let head_style = Style::default()
            .fg(theme::CYAN)
            .add_modifier(Modifier::BOLD);
        for (ri, row) in tb.rows.iter().enumerate() {
            let mut s = String::new();
            for (i, w) in widths.iter().enumerate() {
                let cell = row.get(i).map(|c| c.as_str()).unwrap_or("");
                let pad = w.saturating_sub(cell.chars().count());
                s.push_str(cell);
                s.push_str(&" ".repeat(pad));
                if i + 1 < cols {
                    s.push_str("  ");
                }
            }
            let truncated: String = s.chars().take(self.width as usize).collect();
            let st = if ri == 0 { head_style } else { style };
            self.lines.push(Line::from(Span::styled(truncated, st)));
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_block(None);
        // Trim trailing blank line.
        while self
            .lines
            .last()
            .map(|l| l.spans.is_empty())
            .unwrap_or(false)
        {
            self.lines.pop();
        }
        self.lines
    }
}

fn heading_color(level: HeadingLevel) -> Color {
    match level {
        HeadingLevel::H1 | HeadingLevel::H2 => theme::CYAN,
        _ => theme::ACCENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    fn flatten(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn find_span<'a>(lines: &'a [Line], needle: &str) -> Option<&'a Span<'a>> {
        for l in lines {
            for s in &l.spans {
                if s.content.contains(needle) {
                    return Some(s);
                }
            }
        }
        None
    }

    #[test]
    fn heading_is_bold_and_colored() {
        let lines = render_markdown("# Title", 40, theme::TEXT);
        let span = find_span(&lines, "Title").expect("heading text present");
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(span.style.fg, Some(theme::CYAN));
    }

    #[test]
    fn bold_emits_bold_span() {
        let lines = render_markdown("hello **world**", 40, theme::TEXT);
        let span = find_span(&lines, "world").expect("bold text present");
        assert!(span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn italic_emits_italic_span() {
        let lines = render_markdown("an *emphatic* word", 40, theme::TEXT);
        let span = find_span(&lines, "emphatic").expect("italic text present");
        assert!(span.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn strikethrough_emits_crossed_span() {
        let lines = render_markdown("~~gone~~", 40, theme::TEXT);
        let span = find_span(&lines, "gone").expect("strike text present");
        assert!(span.style.add_modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    fn inline_code_is_dim() {
        let lines = render_markdown("call `foo()` now", 40, theme::TEXT);
        let span = find_span(&lines, "foo()").expect("inline code present");
        assert_eq!(span.style.fg, Some(theme::DIM));
    }

    #[test]
    fn fenced_code_block_has_bg_and_dim_fg() {
        let md = "```\nlet x = 1;\n```";
        let lines = render_markdown(md, 40, theme::TEXT);
        let span = find_span(&lines, "let x = 1;").expect("code line present");
        assert_eq!(span.style.fg, Some(theme::DIM));
        assert_eq!(span.style.bg, Some(theme::BG_HIGHLIGHT));
    }

    #[test]
    fn bullet_list_gets_marker() {
        let md = "- one\n- two";
        let lines = render_markdown(md, 40, theme::TEXT);
        let text = flatten(&lines);
        assert!(text.contains("• one"), "got: {text:?}");
        assert!(text.contains("• two"), "got: {text:?}");
    }

    #[test]
    fn ordered_list_numbers() {
        let md = "1. first\n2. second";
        let lines = render_markdown(md, 40, theme::TEXT);
        let text = flatten(&lines);
        assert!(text.contains("1. first"), "got: {text:?}");
        assert!(text.contains("2. second"), "got: {text:?}");
    }

    #[test]
    fn table_aligns_and_headers_styled() {
        let md = "| A | B |\n|---|---|\n| 1 | 22 |";
        let lines = render_markdown(md, 40, theme::TEXT);
        let header = find_span(&lines, "A").expect("table header present");
        assert!(header.style.add_modifier.contains(Modifier::BOLD));
        let text = flatten(&lines);
        assert!(text.contains("22"), "got: {text:?}");
    }

    #[test]
    fn width_aware_wrap_preserves_span_style() {
        // A long run of bold words must wrap AND keep BOLD on every span.
        let md = "**aaaa bbbb cccc dddd eeee ffff gggg hhhh**";
        let lines = render_markdown(md, 16, theme::TEXT);
        assert!(lines.len() > 1, "expected wrapping across multiple lines");
        let mut saw_word = false;
        for l in &lines {
            for s in &l.spans {
                if s.content.trim().is_empty() {
                    continue;
                }
                saw_word = true;
                assert!(
                    s.style.add_modifier.contains(Modifier::BOLD),
                    "wrapped span lost BOLD: {:?}",
                    s.content
                );
            }
        }
        assert!(saw_word, "expected at least one styled word");
    }

    #[test]
    fn no_line_exceeds_width() {
        let md = "the quick brown fox jumps over the lazy dog repeatedly today";
        let width = 20u16;
        let lines = render_markdown(md, width, theme::TEXT);
        for l in &lines {
            let len: usize = l.spans.iter().map(|s| s.content.chars().count()).sum();
            assert!(len <= width as usize, "line too long ({len}): {l:?}");
        }
    }

    #[test]
    fn plain_text_uses_base_color() {
        let lines = render_markdown("just words", 40, theme::TEXT);
        let span = find_span(&lines, "just").expect("plain text present");
        assert_eq!(span.style.fg, Some(theme::TEXT));
    }

    // ---- #280: delimiter-less table repair --------------------------------

    /// A well-formed table (WITH the `|---|` delimiter) renders as a clean grid:
    /// header row in cyan+bold, one styled line per row, no pipe-soup.
    #[test]
    fn table_with_delimiter_renders_grid() {
        let md = "| Field | Value |\n| --- | --- |\n| Binary | zeus |\n| Version | 0.1.2 |";
        let lines = render_markdown(md, 80, theme::TEXT);
        // Header cell present and styled as a header (cyan + bold).
        let header = find_span(&lines, "Field").expect("header cell present");
        assert_eq!(header.style.fg, Some(theme::CYAN));
        assert!(header.style.add_modifier.contains(Modifier::BOLD));
        // Data cells present.
        assert!(find_span(&lines, "Binary").is_some(), "data cell Binary");
        assert!(find_span(&lines, "0.1.2").is_some(), "data cell 0.1.2");
    }

    /// The #280 repro: a table that OMITS the delimiter row must now render as
    /// the SAME clean grid (synthetic delimiter injected pre-parse), not the
    /// pipe-soup that pulldown produces for delimiter-less pipe blocks.
    #[test]
    fn table_without_delimiter_is_repaired() {
        let md = "| Field | Value |\n| Binary | zeus |\n| Version | 0.1.2 |";
        let lines = render_markdown(md, 80, theme::TEXT);
        // Header is recognised as a table head: cyan + bold.
        let header = find_span(&lines, "Field").expect("header cell present");
        assert_eq!(
            header.style.fg,
            Some(theme::CYAN),
            "delimiter-less table should still be parsed as a table"
        );
        assert!(header.style.add_modifier.contains(Modifier::BOLD));
        // Data survives intact, not split into per-pipe garbage.
        assert!(find_span(&lines, "Binary").is_some(), "data cell Binary");
        assert!(find_span(&lines, "0.1.2").is_some(), "data cell 0.1.2");
        // No span should be a bare stray pipe (the pipe-soup signature).
        for l in &lines {
            for s in &l.spans {
                assert_ne!(s.content.trim(), "|", "stray pipe leaked: {l:?}");
            }
        }
    }

    /// The injector must not corrupt a table that already has its delimiter
    /// (no double-delimiter, grid still clean).
    #[test]
    fn injector_idempotent_on_valid_table() {
        let md = "| A | B |\n| --- | --- |\n| 1 | 2 |";
        let injected = inject_table_delimiters(md);
        // Exactly one delimiter row.
        let delim_rows = injected.lines().filter(|l| is_delimiter_row(l)).count();
        assert_eq!(delim_rows, 1, "should not add a second delimiter");
    }

    /// Pipe characters inside fenced code blocks must NOT trigger injection.
    #[test]
    fn injector_skips_code_fences() {
        let md = "```\n| not | a | table |\n| still | code | here |\n```";
        let injected = inject_table_delimiters(md);
        assert!(
            !injected.lines().any(is_delimiter_row),
            "must not inject a delimiter inside a code fence"
        );
    }
}
