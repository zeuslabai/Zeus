use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Block, Borders, Widget};
use std::collections::HashMap;

use crate::theme;

// Theme aliases
const FG: ratatui::style::Color = theme::TEXT;
const BG2: ratatui::style::Color = theme::BG_PANEL;

/// Truncate `text` to fit `max_w` display columns, appending `…` when clipped.
///
/// #271 visual-parity: every narrow-width text seam on this screen used a bare
/// `set_line` clamp, which hard-chops mid-word with NO ellipsis (the model.rs /
/// fallback.rs truncation class — e.g. `nomic-embed-text` → `nomic-e`). Route
/// any value that can exceed its budget through this so it truncates honestly
/// with a trailing `…` *inside* the budget. Char-based (the strings here are
/// ASCII provider/model ids); the 1-col `…` replaces the last char on clip.
fn clamp_ellipsis(text: &str, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    if text.chars().count() <= max_w {
        return text.to_string();
    }
    if max_w == 1 {
        return "…".to_string();
    }
    let keep: String = text.chars().take(max_w - 1).collect();
    format!("{keep}…")
}

/// Memory embedding provider — matches JSX MEMORY_PROVIDERS (line 199).
struct MemoryProvider {
    id: &'static str,
    name: &'static str,
    glyph: &'static str,
    color: ratatui::style::Color,
    sub: &'static str,
    model: &'static str,
    recommended: bool,
}

/// All 3 memory providers from the JSX prototype (verified against the actual const).
const MEMORY_PROVIDERS: &[MemoryProvider] = &[
    MemoryProvider {
        id: "ollama",
        name: "Ollama",
        glyph: "OLM",
        color: theme::CYAN,
        sub: "Local, free, private",
        model: "nomic-embed-text",
        recommended: false,
    },
    MemoryProvider {
        id: "openai",
        name: "OpenAI",
        glyph: "OAI",
        color: theme::GREEN,
        sub: "Cloud, paid, fast",
        model: "text-embedding-3-small",
        recommended: false,
    },
    MemoryProvider {
        id: "none",
        name: "FTS-only",
        glyph: "FTS",
        color: theme::AMBER,
        sub: "No embeddings, full-text search only",
        model: "—",
        recommended: true,
    },
];

/// Disk projection rows — verbatim from JSX MemoryStep (line 1633-1638).
const DISK_PROJECTION: &[(&str, &str)] = &[
    ("1K facts", "~12 MB"),
    ("10K facts", "~120 MB"),
    ("100K facts", "~1.2 GB"),
    ("1M facts", "~12 GB"),
];

/// Editable fields in the STORAGE section. The JSX MemoryStep has exactly two:
/// DB Path (always) and Embedding Model (hidden when selected == "none").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemoryField {
    DbPath,
    Model,
}

pub struct MemoryScreen {
    /// Index into MEMORY_PROVIDERS.
    pub selected: usize,
    /// Field values keyed like the JSX `values` map.
    pub values: HashMap<String, String>,
    /// Currently focused storage field.
    pub focused_field: MemoryField,
    /// #260: LIVE Ollama probe result. `None` = probe in flight (or never run,
    /// e.g. standalone preview bin with no runtime); `Some(true)` = `/api/tags`
    /// reachable; `Some(false)` = unreachable/timed-out. The badge + cyan banner
    /// render from THIS, not the static const `detected` flag, so a missing
    /// Ollama shows an honest "not detected" instead of a fabricated badge.
    pub ollama_detected: Option<bool>,
}

impl Default for MemoryScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryScreen {
    pub fn new() -> Self {
        Self {
            selected: 2, // FTS-only — default per #258 (no embeddings unless opted in)
            values: HashMap::new(),
            focused_field: MemoryField::DbPath,
            ollama_detected: None, // #260: until the live probe writes a result
        }
    }

    /// #260: record the result of the live Ollama `/api/tags` probe. Called by
    /// the probe worker (`lib.rs`) under the app lock once the bounded GET
    /// resolves. `true` → reachable (badge + banner render); `false` → honest
    /// "not detected".
    pub fn set_ollama_detected(&mut self, reachable: bool) {
        self.ollama_detected = Some(reachable);
    }

    pub fn selected_id(&self) -> &'static str {
        MEMORY_PROVIDERS[self.selected].id
    }

    pub fn select_prev(&mut self) {
        self.selected = if self.selected == 0 {
            MEMORY_PROVIDERS.len() - 1
        } else {
            self.selected - 1
        };
    }

    pub fn select_next(&mut self) {
        self.selected = (self.selected + 1) % MEMORY_PROVIDERS.len();
    }

    /// Number of Tab-focusable fields (drives the App footer Tab cursor).
    /// DB Path is always focusable; Model only when a backend != "none" is
    /// picked (matches the JSX `selected !== "none"` conditional).
    pub fn field_count(&self) -> usize {
        if self.selected_id() != "none" {
            2
        } else {
            1
        }
    }

    /// Tab cycles DB Path -> Model -> DB Path. With FTS-only the Model field
    /// is hidden (JSX `selected !== "none"` conditional) so focus stays on DB Path.
    pub fn focus_next(&mut self) {
        self.focused_field = match self.focused_field {
            MemoryField::DbPath if self.selected_id() != "none" => MemoryField::Model,
            _ => MemoryField::DbPath,
        };
    }

    fn focused_key(&self) -> &'static str {
        match self.focused_field {
            MemoryField::DbPath => "db_path",
            MemoryField::Model => "model",
        }
    }

    pub fn input_char(&mut self, c: char) {
        let key = self.focused_key().to_string();
        self.values.entry(key).or_default().push(c);
    }

    pub fn input_backspace(&mut self) {
        let key = self.focused_key().to_string();
        if let Some(v) = self.values.get_mut(&key) {
            v.pop();
        }
    }

    /// Effective DB path — JSX default `~/.zeus/mnemosyne.db`.
    fn db_path_display(&self) -> String {
        match self.values.get("db_path") {
            Some(v) if !v.is_empty() => v.clone(),
            _ => "~/.zeus/mnemosyne.db".to_string(),
        }
    }

    /// Effective embedding model — JSX fallback:
    /// ollama -> nomic-embed-text, otherwise text-embedding-3-small.
    fn model_display(&self) -> String {
        match self.values.get("model") {
            Some(v) if !v.is_empty() => v.clone(),
            _ => match self.selected_id() {
                "none" => "—".to_string(), // FTS-only: no embedding model (#258 default)
                "ollama" => "nomic-embed-text".to_string(),
                _ => "text-embedding-3-small".to_string(),
            },
        }
    }

    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        self.render_with_cursor(area, buf, false);
    }

    /// Canonical cursor-port entry (matches auth.rs): the call-site threads
    /// `app.cursor_visible()` so the focused text field paints a blink-gated `▏`.
    pub fn render_with_cursor(
        &self,
        area: Rect,
        buf: &mut ratatui::buffer::Buffer,
        cursor_on: bool,
    ) {
        Clear.render(area, buf);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Length(40)])
            .split(area);

        self.render_left(cols[0], buf, cursor_on);
        self.render_right(cols[1], buf);
    }

    fn render_left(&self, area: Rect, buf: &mut ratatui::buffer::Buffer, cursor_on: bool) {
        let inner = Rect {
            x: area.x + 2,
            y: area.y + 1,
            width: area.width.saturating_sub(4),
            height: area.height.saturating_sub(2),
        };
        let bottom = inner.y + inner.height;
        let mut y = inner.y;

        // Header — JSX 1613-1614
        if y < bottom {
            buf.set_line(
                inner.x,
                y,
                &Line::from(Span::styled(
                    "Memory backend",
                    Style::default().fg(FG).add_modifier(Modifier::BOLD),
                )),
                inner.width,
            );
            y += 1;
        }
        if y < bottom {
            buf.set_line(
                inner.x,
                y,
                &Line::from(Span::styled(
                    clamp_ellipsis(
                        "Mnemosyne — semantic search over agent history. Pick embedding provider.",
                        inner.width as usize,
                    ),
                    Style::default().fg(theme::DIM),
                )),
                inner.width,
            );
            y += 2;
        }

        // Provider cards (3 rows of bordered cards)
        for (i, p) in MEMORY_PROVIDERS.iter().enumerate() {
            let card_h = 4u16;
            if y + card_h > bottom {
                break;
            }
            let selected = i == self.selected;
            let border_color = if selected { p.color } else { theme::MUTED };
            let card = Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: card_h,
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(BG2));
            let card_inner = block.inner(card);
            block.render(card, buf);

            // Line 1: glyph block + name + badges
            let mut spans = vec![
                Span::styled(
                    format!(" {} ", p.glyph),
                    Style::default()
                        .fg(theme::BG)
                        .bg(p.color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    p.name,
                    Style::default().fg(FG).add_modifier(Modifier::BOLD),
                ),
            ];
            if p.recommended {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    "★ REC",
                    Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                ));
            }
            // #260: Ollama's badge is LIVE — only show `● DETECTED` when the
            // probe confirmed `/api/tags` reachable. While probing (`None`) show
            // an honest "PROBING…"; if unreachable, show "○ NOT DETECTED". Other
            // providers keep their static const `detected` flag (none set it).
            if p.id == "ollama" {
                match self.ollama_detected {
                    Some(true) => {
                        spans.push(Span::raw("  "));
                        spans.push(Span::styled(
                            "● DETECTED",
                            Style::default().fg(theme::CYAN).add_modifier(Modifier::BOLD),
                        ));
                    }
                    Some(false) => {
                        spans.push(Span::raw("  "));
                        spans.push(Span::styled(
                            "○ NOT DETECTED",
                            Style::default().fg(theme::DIM),
                        ));
                    }
                    None => {
                        spans.push(Span::raw("  "));
                        spans.push(Span::styled(
                            "… PROBING",
                            Style::default().fg(theme::DIM),
                        ));
                    }
                }
            }
            if selected {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    "▸ SELECTED",
                    Style::default().fg(p.color).add_modifier(Modifier::BOLD),
                ));
            }
            buf.set_line(card_inner.x + 1, card_inner.y, &Line::from(spans), card_inner.width.saturating_sub(2));

            // Line 2: sub + model — clamp so neither hard-chops mid-word.
            // #271: budget the line; the model (rightmost) clips first with `…`.
            // Keep `sub` whole when it fits, give the remainder to `model`; if even
            // `sub` overflows, clamp it (the model then drops to empty honestly).
            let line2_w = card_inner.width.saturating_sub(2) as usize;
            const SEP: &str = "  ·  ";
            let sub_str = clamp_ellipsis(p.sub, line2_w);
            let used = sub_str.chars().count() + SEP.chars().count();
            let model_w = line2_w.saturating_sub(used);
            let model_str = clamp_ellipsis(p.model, model_w);
            let mut line2_spans = vec![Span::styled(sub_str, Style::default().fg(theme::DIM))];
            if !model_str.is_empty() {
                line2_spans.push(Span::styled(SEP, Style::default().fg(theme::MUTED)));
                line2_spans
                    .push(Span::styled(model_str, Style::default().fg(theme::ACCENT_DIM)));
            }
            buf.set_line(
                card_inner.x + 1,
                card_inner.y + 1,
                &Line::from(line2_spans),
                card_inner.width.saturating_sub(2),
            );

            y += card_h + 1;
        }

        // STORAGE section — JSX 1623-1627
        if y < bottom {
            buf.set_line(
                inner.x,
                y,
                &Line::from(Span::styled(
                    "S T O R A G E",
                    Style::default()
                        .fg(theme::ACCENT_DIM)
                        .add_modifier(Modifier::BOLD),
                )),
                inner.width,
            );
            y += 1;
        }

        // DB Path field (always shown)
        y = self.render_field(
            buf,
            inner,
            y,
            "DB Path",
            &self.db_path_display(),
            self.values.get("db_path").map(|v| !v.is_empty()).unwrap_or(false),
            self.focused_field == MemoryField::DbPath,
            cursor_on,
        );

        // Embedding Model field — hidden when FTS-only (JSX conditional)
        if self.selected_id() != "none" {
            self.render_field(
                buf,
                inner,
                y,
                "Embedding Model",
                &self.model_display(),
                self.values.get("model").map(|v| !v.is_empty()).unwrap_or(false),
                self.focused_field == MemoryField::Model,
                cursor_on,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_field(
        &self,
        buf: &mut ratatui::buffer::Buffer,
        inner: Rect,
        y: u16,
        label: &str,
        value: &str,
        has_raw_value: bool,
        focused: bool,
        cursor_on: bool,
    ) -> u16 {
        let bottom = inner.y + inner.height;
        if y >= bottom {
            return y;
        }
        let label_color = if focused { theme::ACCENT } else { theme::DIM };
        let marker = if focused { "▸ " } else { "  " };
        let mut spans = vec![
            Span::styled(marker, Style::default().fg(theme::ACCENT)),
            Span::styled(format!("{label:<18}"), Style::default().fg(label_color)),
            Span::styled(value.to_string(), Style::default().fg(FG)),
        ];
        // Canonical blink-gated insertion caret `▏` (auth.rs/orchestration pattern).
        // Painted on the focused field during the blink-on phase ONLY when the
        // field holds real input (`has_raw_value`). Placeholder trap (mem 1249):
        // an empty field shows its default hint via the display `value`, which is
        // NOT an edit position — so no caret until the user types.
        if focused && cursor_on && has_raw_value {
            spans.push(Span::styled(
                "\u{258f}",
                Style::default().fg(theme::ACCENT),
            ));
        }
        buf.set_line(
            inner.x,
            y,
            &Line::from(spans),
            inner.width,
        );
        y + 1
    }

    fn render_right(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        // JSX: the 280w right column carries `borderLeft: 1px solid C.muted` —
        // a single vertical rule separating it from the LEFT card column.
        for ry in area.y..(area.y + area.height) {
            buf.set_string(area.x, ry, "│", Style::default().fg(theme::MUTED));
        }
        let inner = Rect {
            x: area.x + 2,
            y: area.y + 1,
            width: area.width.saturating_sub(3),
            height: area.height.saturating_sub(2),
        };
        let bottom = inner.y + inner.height;
        let mut y = inner.y;

        // DISK PROJECTION header — JSX 1631
        if y < bottom {
            buf.set_line(
                inner.x,
                y,
                &Line::from(Span::styled(
                    "D I S K   P R O J E C T I O N",
                    Style::default()
                        .fg(theme::ACCENT_DIM)
                        .add_modifier(Modifier::BOLD),
                )),
                inner.width,
            );
            y += 2;
        }

        // Projection table rows
        for (k, v) in DISK_PROJECTION {
            if y >= bottom {
                break;
            }
            // #271: guarantee a ≥1-col gap so the key and value never collide
            // (the `1K facts~12 MB` no-gap clobber when the column is squeezed).
            // Value (rightmost) is load-bearing; clamp the key if the row can't
            // hold both with a gap.
            let w = inner.width as usize;
            let v_w = v.chars().count();
            let key_budget = w.saturating_sub(v_w + 1); // reserve value + ≥1 gap
            let key_str = clamp_ellipsis(k, key_budget);
            let pad = w
                .saturating_sub(key_str.chars().count() + v_w)
                .max(1);
            buf.set_line(
                inner.x,
                y,
                &Line::from(vec![
                    Span::styled(key_str, Style::default().fg(theme::DIM)),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(*v, Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
                ]),
                inner.width,
            );
            y += 1;
            if y < bottom {
                buf.set_line(
                    inner.x,
                    y,
                    &Line::from(Span::styled(
                        "─".repeat(inner.width as usize),
                        Style::default().fg(theme::MUTED),
                    )),
                    inner.width,
                );
                y += 1;
            }
        }

        // Ollama detected banner — JSX 1641-1644 (cyan bordered box).
        // #260: only render when the LIVE probe confirmed Ollama reachable.
        // `None` (probing) / `Some(false)` (unreachable) → no fabricated banner.
        y += 1;
        let banner_h = 7u16;
        if self.ollama_detected == Some(true) && y + banner_h <= bottom {
            let banner = Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: banner_h,
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::CYAN))
                .style(Style::default().bg(BG2));
            let b_inner = block.inner(banner);
            block.render(banner, buf);

            buf.set_line(
                b_inner.x + 1,
                b_inner.y,
                &Line::from(Span::styled(
                    "● OLLAMA DETECTED",
                    Style::default().fg(theme::CYAN).add_modifier(Modifier::BOLD),
                )),
                b_inner.width.saturating_sub(2),
            );
            // NOTE: the right panel box is ~33 cols inner — `localhost:11434`
            // must lead its own line or `set_line` clips it (the URL is the
            // asserted token + JSX accentBright highlight). Wrap so no run clips.
            buf.set_line(
                b_inner.x + 1,
                b_inner.y + 1,
                &Line::from(Span::styled(
                    "Found local Ollama at",
                    Style::default().fg(FG),
                )),
                b_inner.width.saturating_sub(2),
            );
            buf.set_line(
                b_inner.x + 1,
                b_inner.y + 2,
                &Line::from(Span::styled(
                    "localhost:11434",
                    Style::default().fg(theme::ACCENT_BRIGHT),
                )),
                b_inner.width.saturating_sub(2),
            );
            buf.set_line(
                b_inner.x + 1,
                b_inner.y + 3,
                &Line::from(vec![
                    Span::styled("with ", Style::default().fg(FG)),
                    Span::styled(
                        "nomic-embed-text",
                        Style::default().fg(FG).add_modifier(Modifier::BOLD),
                    ),
                ]),
                b_inner.width.saturating_sub(2),
            );
            buf.set_line(
                b_inner.x + 1,
                b_inner.y + 4,
                &Line::from(Span::styled(
                    "available. Recommended",
                    Style::default().fg(FG),
                )),
                b_inner.width.saturating_sub(2),
            );
            buf.set_line(
                b_inner.x + 1,
                b_inner.y + 5,
                &Line::from(Span::styled(
                    "for free local embeddings.",
                    Style::default().fg(FG),
                )),
                b_inner.width.saturating_sub(2),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_ids_match_jsx() {
        let ids: Vec<&str> = MEMORY_PROVIDERS.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec!["ollama", "openai", "none"]);
    }

    #[test]
    fn fts_is_recommended_default() {
        // #258: FTS-only is the default selection (no embeddings unless opted in).
        let s = MemoryScreen::new();
        assert_eq!(s.selected_id(), "none");
        let p = &MEMORY_PROVIDERS[s.selected];
        assert!(p.recommended, "FTS-only must carry the ★ REC badge as default");
        assert_ne!(p.id, "ollama", "FTS-only is not a detection target (no probe)");
        // Ollama is no longer the default-recommended pick (#258).
        let ollama = &MEMORY_PROVIDERS[0];
        assert!(!ollama.recommended, "Ollama no longer default-recommended");
    }

    #[test]
    fn selection_wraps_both_directions() {
        // Default is now FTS-only (index 2) per #258.
        let mut s = MemoryScreen::new();
        assert_eq!(s.selected_id(), "none");
        s.select_prev(); // 2 -> 1
        assert_eq!(s.selected_id(), "openai");
        s.select_next(); // 1 -> 2
        assert_eq!(s.selected_id(), "none");
        s.select_next(); // 2 -> 0 (wrap)
        assert_eq!(s.selected_id(), "ollama");
    }

    #[test]
    fn model_fallback_matches_jsx() {
        // Default is FTS-only (index 2): model is the em-dash placeholder.
        let mut s = MemoryScreen::new();
        assert_eq!(s.model_display(), "—"); // FTS-only — no embedding model
        s.select_next(); // 2 -> 0 ollama
        assert_eq!(s.model_display(), "nomic-embed-text");
        s.select_next(); // 0 -> 1 openai
        assert_eq!(s.model_display(), "text-embedding-3-small");
        assert_eq!(s.db_path_display(), "~/.zeus/mnemosyne.db");
    }

    #[test]
    fn fts_only_hides_model_field_focus() {
        let mut s = MemoryScreen::new();
        // Default is FTS-only (#258) → Model field hidden, focus stays on DbPath.
        s.focus_next();
        assert_eq!(s.focused_field, MemoryField::DbPath);
        // Pick a backend WITH embeddings (ollama) → Model becomes focusable.
        while s.selected_id() != "ollama" {
            s.select_next();
        }
        s.focused_field = MemoryField::DbPath;
        s.focus_next();
        assert_eq!(s.focused_field, MemoryField::Model);
        // Switch back to FTS-only → focus cycling must collapse to DbPath.
        while s.selected_id() != "none" {
            s.select_next();
        }
        s.focused_field = MemoryField::DbPath;
        s.focus_next();
        assert_eq!(s.focused_field, MemoryField::DbPath);
    }

    #[test]
    fn disk_projection_rows_match_jsx() {
        assert_eq!(DISK_PROJECTION.len(), 4);
        assert_eq!(DISK_PROJECTION[0], ("1K facts", "~12 MB"));
        assert_eq!(DISK_PROJECTION[3], ("1M facts", "~12 GB"));
    }

    /// Render the screen to a flat string with an explicit blink phase —
    /// mirrors the auth.rs/orchestration canonical caret-test harness.
    fn render_to_string(screen: &MemoryScreen, cursor_on: bool) -> String {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        screen.render_with_cursor(area, &mut buf, cursor_on);
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// #271: render at an arbitrary width so the narrow-column truncation seams
    /// are exercised (the default `render_to_string` is fixed at 80 and never
    /// triggers the clip path).
    fn render_at_width(screen: &MemoryScreen, w: u16) -> String {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        let area = Rect::new(0, 0, w, 30);
        let mut buf = Buffer::empty(area);
        screen.render_with_cursor(area, &mut buf, false);
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn clamp_ellipsis_truncates_with_marker() {
        // Unit: the helper appends `…` only on clip, never widens.
        assert_eq!(clamp_ellipsis("nomic-embed-text", 16), "nomic-embed-text");
        assert_eq!(clamp_ellipsis("nomic-embed-text", 8), "nomic-e…");
        assert_eq!(clamp_ellipsis("nomic-embed-text", 1), "…");
        assert_eq!(clamp_ellipsis("nomic-embed-text", 0), "");
    }

    #[test]
    fn narrow_width_clips_with_ellipsis_not_midword() {
        // #271 LOAD-BEARING: at a squeezed width the model id and subtitle must
        // truncate with a trailing `…` — NOT hard-chop mid-word (the model.rs /
        // fallback.rs class). Revert any clamp → this fails (mid-word chop, no `…`).
        let mut s = MemoryScreen::new();
        s.set_ollama_detected(false);
        let r = render_at_width(&s, 56);
        // The full model id must NOT survive whole at this width...
        assert!(
            !r.contains("nomic-embed-text"),
            "model id should be clipped at narrow width; got:\n{r}"
        );
        // ...and where it clips, it must carry an ellipsis (honest truncation).
        assert!(
            r.contains("nomic-…") || r.contains("nomic-e…"),
            "clipped model must end with `…`, not a bare mid-word chop; got:\n{r}"
        );
        // Subtitle must also ellipsis, not vanish mid-word.
        assert!(
            r.contains('…') && r.contains("Mnemosyne"),
            "subtitle must clamp with `…`; got:\n{r}"
        );
    }

    #[test]
    fn disk_projection_never_collides_keeps_gap() {
        // #271 LOAD-BEARING: the DISK PROJECTION key/value must keep a ≥1-col gap
        // even when squeezed — never the `1K facts~12 MB` no-gap clobber. Revert
        // the `.max(1)` pad guard → key and value collide → this fails.
        let mut s = MemoryScreen::new();
        s.set_ollama_detected(false);
        let r = render_at_width(&s, 56);
        // The squeezed form clamps the key but the value stays whole with a gap.
        assert!(
            !r.contains("facts~") && !r.contains("facts~12"),
            "key and value must not collide (need a gap); got:\n{r}"
        );
        assert!(r.contains("~12 MB"), "value must render whole; got:\n{r}");
    }

    #[test]
    fn normal_width_renders_model_in_full() {
        // #271: at a comfortable width nothing clamps prematurely — the full
        // model id renders. Pins against an over-eager clamp.
        let mut s = MemoryScreen::new();
        s.set_ollama_detected(false);
        let r = render_at_width(&s, 100);
        assert!(
            r.contains("nomic-embed-text"),
            "full model id must render at width 100; got:\n{r}"
        );
        assert!(
            r.contains("text-embedding-3-small"),
            "full OpenAI model id must render at width 100; got:\n{r}"
        );
    }

    #[test]
    fn ollama_probing_shows_no_fabricated_badge_or_banner() {
        // #260: before the live probe resolves (`None`), the Ollama card shows
        // an honest "… PROBING" and the cyan banner must NOT render — no
        // fabricated `● DETECTED` / `● OLLAMA DETECTED` (#251 honest-fallback).
        let s = MemoryScreen::new();
        assert_eq!(s.ollama_detected, None, "default is unprobed");
        let r = render_to_string(&s, false);
        assert!(r.contains("PROBING"), "expected honest PROBING state; got:\n{r}");
        assert!(
            !r.contains("● DETECTED"),
            "must NOT fabricate ● DETECTED while probing; got:\n{r}"
        );
        assert!(
            !r.contains("● OLLAMA DETECTED"),
            "must NOT render the cyan banner while probing; got:\n{r}"
        );
    }

    #[test]
    fn ollama_unreachable_shows_not_detected_and_no_banner() {
        // #260: probe resolved unreachable → honest "○ NOT DETECTED", no banner.
        let mut s = MemoryScreen::new();
        s.set_ollama_detected(false);
        let r = render_to_string(&s, false);
        assert!(
            r.contains("NOT DETECTED"),
            "expected honest NOT DETECTED; got:\n{r}"
        );
        assert!(
            !r.contains("● OLLAMA DETECTED"),
            "no cyan banner when Ollama unreachable; got:\n{r}"
        );
    }

    #[test]
    fn ollama_reachable_shows_detected_and_banner() {
        // #260: probe confirmed reachable → `● DETECTED` badge + cyan banner.
        let mut s = MemoryScreen::new();
        s.set_ollama_detected(true);
        let r = render_to_string(&s, false);
        assert!(r.contains("● DETECTED"), "expected ● DETECTED badge; got:\n{r}");
        assert!(
            r.contains("● OLLAMA DETECTED"),
            "expected the cyan banner when reachable; got:\n{r}"
        );
    }

    #[test]
    fn caret_painted_on_blink_phase_when_field_has_input() {
        // DbPath is focused by default; type into it so it holds raw input.
        let mut s = MemoryScreen::new();
        s.input_char('/');
        s.input_char('x');
        let rendered = render_to_string(&s, true);
        assert!(
            rendered.contains('\u{258f}'),
            "expected canonical caret `▏` when cursor_on + focused field has input; got:\n{rendered}"
        );
    }

    #[test]
    fn caret_hidden_on_blink_off_phase() {
        let mut s = MemoryScreen::new();
        s.input_char('/');
        let rendered = render_to_string(&s, false);
        assert!(
            !rendered.contains('\u{258f}'),
            "expected NO caret during blink-off phase; got:\n{rendered}"
        );
    }

    #[test]
    fn caret_absent_on_unfilled_field() {
        // Placeholder trap (mem 1249): an empty field shows its default hint,
        // which is NOT an edit position — no caret until the user types.
        let s = MemoryScreen::new();
        let rendered = render_to_string(&s, true);
        assert!(
            !rendered.contains('\u{258f}'),
            "expected NO caret on an unfilled field (placeholder is a hint); got:\n{rendered}"
        );
    }

    #[test]
    fn static_block_caret_is_absent() {
        // Regression: never paint the old static `▌` block caret (Option A).
        let mut s = MemoryScreen::new();
        s.input_char('/');
        let rendered = render_to_string(&s, true);
        assert!(
            !rendered.contains('\u{258c}'),
            "the static `▌` block caret must be gone (Option A); got:\n{rendered}"
        );
    }


}
