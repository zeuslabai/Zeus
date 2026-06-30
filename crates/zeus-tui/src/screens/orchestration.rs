use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Block, Borders, Widget};

use crate::theme;

/// Truncate `text` to `max_w` display columns, appending `…` when clipped.
/// Mirrors the canonical idiom in voice/memory/fallback/chanconfig/channels.
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
    let kept: String = text.chars().take(max_w - 1).collect();
    format!("{}…", kept)
}

// Theme aliases
const FG: ratatui::style::Color = theme::TEXT;
const BG2: ratatui::style::Color = theme::BG_PANEL;

/// Orchestration mode entry — matches JSX ORCH_MODES (line 193).
struct OrchMode {
    id: &'static str,
    name: &'static str,
    glyph: &'static str,
    color: ratatui::style::Color,
    sub: &'static str,
    desc: &'static str,
    recommended: bool,
}

/// All 3 orchestration modes from the JSX prototype.
const ORCH_MODES: &[OrchMode] = &[
    OrchMode {
        id: "all-on",
        name: "All-on",
        glyph: "ALL",
        color: theme::ACCENT,
        sub: "Heartbeat + cron + watchdog",
        desc: "Full autonomous operation. Recommended for fleet agents.",
        recommended: true,
    },
    OrchMode {
        id: "heartbeat-only",
        name: "Heartbeat-only",
        glyph: "HB",
        color: theme::AMBER,
        sub: "Wake events only, no scheduled tasks",
        desc: "Reactive only. Agent wakes on inputs, doesn't run scheduled work.",
        recommended: false,
    },
    OrchMode {
        id: "disabled",
        name: "Disabled",
        glyph: "OFF",
        color: theme::DIM,
        sub: "Manual invocation only",
        desc: "No background activity. Agent runs only when explicitly invoked.",
        recommended: false,
    },
];

/// Config field definition for the conditional fields panel.
struct ConfigField {
    key: &'static str,
    label: &'static str,
    placeholder: &'static str,
    hint: &'static str,
}

/// Config fields shown when all-on or heartbeat-only is selected.
const CONFIG_FIELDS: &[ConfigField] = &[
    ConfigField {
        key: "interval",
        label: "Interval",
        placeholder: "300",
        hint: "Seconds between heartbeat ticks (default 300 = 5 min)",
    },
    ConfigField {
        key: "quiet_start",
        label: "Quiet Start",
        placeholder: "23",
        hint: "Hour (24h) when heartbeat goes quiet",
    },
    ConfigField {
        key: "quiet_end",
        label: "Quiet End",
        placeholder: "8",
        hint: "Hour (24h) when heartbeat resumes",
    },
];

/// Orchestration screen state — step 15, id: "orchestration", code: ORCH.
pub struct OrchestrationScreen {
    /// Index of the currently selected orchestration mode.
    selected: usize,
    /// Config field values (key -> value).
    values: std::collections::HashMap<String, String>,
    /// Currently focused config field index.
    focused_field: usize,
}

impl Default for OrchestrationScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl OrchestrationScreen {
    pub fn new() -> Self {
        Self {
            selected: 0, // "all-on" is default (recommended)
            values: std::collections::HashMap::new(),
            focused_field: 0,
        }
    }

    /// Handle Up key — kept for parity (3-col grid is single-row, no-op).
    pub fn handle_up(&mut self) {}

    /// Handle Down key — kept for parity (3-col grid is single-row, no-op).
    pub fn handle_down(&mut self) {}

    /// Handle Left key — move selection left in the horizontal 3-col grid.
    pub fn move_left(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
        self.clamp_focus();
    }

    /// Handle Right key — move selection right in the horizontal 3-col grid.
    pub fn move_right(&mut self) {
        if self.selected < ORCH_MODES.len() - 1 {
            self.selected += 1;
        }
        self.clamp_focus();
    }

    /// Number of config fields visible for the current mode (0 when disabled).
    fn visible_field_count(&self) -> usize {
        let mode = &ORCH_MODES[self.selected];
        if mode.id == "all-on" || mode.id == "heartbeat-only" {
            CONFIG_FIELDS.len()
        } else {
            0
        }
    }

    /// Re-clamp the focused field index into the active mode's field set.
    /// When switching to Disabled (0 fields) the focus must not dangle.
    pub fn clamp_focus(&mut self) {
        let count = self.visible_field_count();
        if count == 0 {
            self.focused_field = 0;
        } else if self.focused_field >= count {
            self.focused_field = count - 1;
        }
    }

    /// Type a character into the focused config field (when fields are visible).
    pub fn input_char(&mut self, c: char) {
        if self.visible_field_count() == 0 {
            return;
        }
        let key = CONFIG_FIELDS[self.focused_field].key.to_string();
        self.values.entry(key).or_default().push(c);
    }

    /// Backspace from the focused config field (when fields are visible).
    pub fn input_backspace(&mut self) {
        if self.visible_field_count() == 0 {
            return;
        }
        let key = CONFIG_FIELDS[self.focused_field].key.to_string();
        if let Some(v) = self.values.get_mut(&key) {
            v.pop();
        }
    }

    /// Handle Tab — cycle focus through config fields (when visible).
    pub fn handle_tab(&mut self) {
        let mode = &ORCH_MODES[self.selected];
        if mode.id == "all-on" || mode.id == "heartbeat-only" {
            self.focused_field = (self.focused_field + 1) % CONFIG_FIELDS.len();
        }
    }

    /// Get the currently selected mode id.
    pub fn selected_mode(&self) -> &str {
        ORCH_MODES[self.selected].id
    }

    /// Render the orchestration screen into the given area.
    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        // Default render with the cursor off (the `Widget`-style entry).
        self.render_with_cursor(area, buf, false);
    }

    /// Render with an explicit blink phase (`cursor_on` from `App.cursor_visible()`).
    /// The focused config field paints the canonical `▏` caret only when `cursor_on`
    /// (auth.rs canonical pattern, ported via 106's `render_with_cursor` recipe).
    pub fn render_with_cursor(&self, area: Rect, buf: &mut ratatui::buffer::Buffer, cursor_on: bool) {
        Clear.render(area, buf);
        // Outer block with opaque bg
        Block::default()
            .style(Style::default().bg(theme::BG))
            .render(area, buf);

        // Main layout: header | mode grid | config fields (conditional)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // header
                Constraint::Length(10), // mode grid (3 cards)
                Constraint::Min(0),    // config fields (conditional)
            ])
            .split(area);

        self.render_header(chunks[0], buf);
        self.render_mode_grid(chunks[1], buf);
        self.render_config_fields(chunks[2], buf, cursor_on);
    }

    /// Render header: "Orchestration mode" + sub.
    fn render_header(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let lines = [Line::from(vec![
                Span::styled("Orchestration mode", Style::default().fg(FG).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("How Zeus runs background work — heartbeat, cron, watchdog.", Style::default().fg(theme::DIM)),
            ])];
        let block = Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme::BG));
        let inner = block.inner(area);
        block.render(area, buf);
        for (i, line) in lines.iter().enumerate() {
            if i as u16 >= inner.height {
                break;
            }
            buf.set_line(inner.x, inner.y + i as u16, line, inner.width);
        }
    }

    /// Render 3-column mode grid with bordered cards.
    fn render_mode_grid(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let constraints: Vec<Constraint> = ORCH_MODES.iter().map(|_| Constraint::Ratio(1, ORCH_MODES.len() as u32)).collect();
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(area);

        for (i, mode) in ORCH_MODES.iter().enumerate() {
            self.render_mode_card(chunks[i], buf, mode, i == self.selected);
        }
    }

    /// Render a single mode card with bordered card, left accent, glyph + name + sub + desc.
    fn render_mode_card(&self, area: Rect, buf: &mut ratatui::buffer::Buffer, mode: &OrchMode, is_selected: bool) {
        let border_color = if is_selected {
            theme::ACCENT
        } else if mode.recommended {
            theme::GREEN
        } else {
            theme::DIM
        };

        let bg = if is_selected {
            theme::ACCENT_FAINT
        } else {
            BG2
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(bg));

        let inner = block.inner(area);
        block.render(area, buf);

        // Left accent stripe (2px)
        for y in inner.y..inner.y + inner.height {
            buf.set_style(Rect::new(inner.x, y, 1, 1), Style::default().fg(mode.color));
            buf.set_style(Rect::new(inner.x + 1, y, 1, 1), Style::default().fg(mode.color));
        }

        // Content area (after left accent)
        let content_x = inner.x + 3;
        let content_w = inner.width.saturating_sub(3);
        let mut y = inner.y;

        // Glyph badge — JSX 36×22 filled box (bg = mode color, fg = C.bg).
        // Drawn FIRST so the status badge can be collision-guarded against it.
        let glyph_badge = format!(" {} ", mode.glyph);
        let glyph_end = content_x + glyph_badge.chars().count() as u16;
        if y < inner.y + inner.height {
            let glyph_style = Style::default()
                .fg(theme::BG)
                .bg(mode.color)
                .add_modifier(Modifier::BOLD);
            buf.set_string(content_x, y, &glyph_badge, glyph_style);
        }

        // Badge precedence (JSX 1580-1585): ▸ SELECTED when selected; else
        // ★ REC on the recommended card when it is NOT the current selection.
        // The badge right-anchors on the glyph row; at narrow card widths it
        // would otherwise OVERWRITE the glyph badge (`ALL` → `ALL LECTED`).
        // Guard: only draw when there is a 1-col gap clear of the glyph, and
        // clamp the badge to that gap so it can never overrun the glyph zone.
        let status_badge = if is_selected {
            Some(("▸ SELECTED", theme::ACCENT))
        } else if mode.recommended {
            Some(("★ REC", theme::GREEN))
        } else {
            None
        };
        if let Some((badge, color)) = status_badge {
            if inner.height > 0 {
                let badge_w = badge.chars().count() as u16;
                let badge_x = inner.x + inner.width.saturating_sub(badge_w + 1);
                // Only render if the badge starts at least 1 col past the glyph.
                if badge_x > glyph_end {
                    buf.set_string(
                        badge_x,
                        inner.y,
                        badge,
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    );
                }
            }
        }

        y += 1;

        // Name — JSX C.white, bold. Clamp to the card interior so a long
        // name (e.g. "Heartbeat-only") truncates with `…` inside the border
        // rather than overflowing into the card chrome at narrow widths.
        if y < inner.y + inner.height {
            let name = clamp_ellipsis(mode.name, content_w as usize);
            buf.set_string(content_x, y, &name, Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD));
            y += 1;
        }

        // Sub — JSX italic, dim. Same clamp: subs like "Wake events only, no
        // scheduled tasks" are long and would bleed past the card border.
        if y < inner.y + inner.height {
            let sub = clamp_ellipsis(mode.sub, content_w as usize);
            buf.set_string(content_x, y, &sub, Style::default().fg(theme::DIM).add_modifier(Modifier::ITALIC));
            y += 1;
        }

        // Desc (simple word-wrap)
        if y < inner.y + inner.height {
            let words: Vec<&str> = mode.desc.split_whitespace().collect();
            let mut line = String::new();
            for word in words {
                if line.len() + word.len() + 1 > content_w as usize && !line.is_empty() {
                    if y >= inner.y + inner.height {
                        break;
                    }
                    buf.set_string(content_x, y, &line, Style::default().fg(theme::DIM));
                    y += 1;
                    line.clear();
                }
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(word);
            }
            if !line.is_empty() && y < inner.y + inner.height {
                buf.set_string(content_x, y, &line, Style::default().fg(theme::DIM));
            }
        }
    }

    /// Render config fields (conditional on selected mode).
    fn render_config_fields(&self, area: Rect, buf: &mut ratatui::buffer::Buffer, cursor_on: bool) {
        let mode = &ORCH_MODES[self.selected];

        // Only show fields for all-on or heartbeat-only
        if mode.id != "all-on" && mode.id != "heartbeat-only" {
            // Disabled mode — show info text
            let block = Block::default()
                .borders(Borders::NONE)
                .style(Style::default().bg(theme::BG));
            let inner = block.inner(area);
            block.render(area, buf);
            buf.set_string(inner.x, inner.y, "No configuration needed for disabled mode.", Style::default().fg(theme::DIM));
            return;
        }

        let block = Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme::BG));
        let inner = block.inner(area);
        block.render(area, buf);

        let mut y = inner.y;

        // Section label
        if y < inner.y + inner.height {
            buf.set_string(inner.x, y, "HEARTBEAT TIMING", Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD));
            y += 1;
        }

        // Config fields
        for (i, field) in CONFIG_FIELDS.iter().enumerate() {
            if y >= inner.y + inner.height {
                break;
            }

            let is_focused = i == self.focused_field;
            // Raw stored entry (empty if unfilled) — distinct from the display
            // fallback below. The caret gates on the RAW value being non-empty,
            // so it never paints at the end of placeholder hint text.
            let raw_value = self.values.get(field.key).map(|s| s.as_str()).unwrap_or("");
            let value = if raw_value.is_empty() { field.placeholder } else { raw_value };

            // Label
            let label_style = if is_focused {
                Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::DIM)
            };
            buf.set_string(inner.x, y, field.label, label_style);
            y += 1;

            // Value / input field
            if y >= inner.y + inner.height {
                break;
            }
            let value_style = if is_focused {
                Style::default().fg(FG).bg(theme::BG_PANEL)
            } else {
                Style::default().fg(FG)
            };
            // `value` already resolves to placeholder-or-raw (see binding above).
            buf.set_string(inner.x + 2, y, value, value_style);
            // Insertion-point cursor: painted after real input on the focused
            // field, gated on the shared blink phase (`cursor_on` from
            // `App.cursor_visible()`). The empty-state placeholder is a hint,
            // not an edit position, so no caret there (auth.rs canonical `▏`).
            if is_focused && cursor_on && !raw_value.is_empty() {
                let cursor_col = inner.x + 2 + raw_value.chars().count() as u16;
                if cursor_col < inner.x + inner.width {
                    buf.set_string(cursor_col, y, "\u{258f}", Style::default().fg(theme::AMBER));
                }
            }
            y += 1;

            // Hint
            if y >= inner.y + inner.height {
                break;
            }
            buf.set_string(inner.x + 2, y, field.hint, Style::default().fg(theme::DIM));
            y += 1;

            // Spacer
            y += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    /// Render the screen to a flat string (all cells concatenated) at a fixed
    /// size, with an explicit blink phase — mirrors the auth.rs/chanconfig
    /// canonical caret-test harness.
    fn render_to_string(screen: &OrchestrationScreen, cursor_on: bool) -> String {
        let area = Rect::new(0, 0, 60, 24);
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

    /// Render at an explicit width (height fixed tall enough for the grid),
    /// returning the per-row buffer for column-precise overflow assertions.
    fn render_at_width(screen: &OrchestrationScreen, w: u16) -> Buffer {
        let area = Rect::new(0, 0, w, 24);
        let mut buf = Buffer::empty(area);
        screen.render_with_cursor(area, &mut buf, false);
        buf
    }

    fn buf_to_string(buf: &Buffer) -> String {
        let area = *buf.area();
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// For each card row, find the right border `│`/`┐` glyph column and
    /// assert no text bleeds into the inter-card gap cell just past it.
    /// (Returns the number of card-border columns checked, so the test can
    /// assert the harness actually exercised the cards.)
    fn assert_no_bleed_past_card_border(buf: &Buffer) -> usize {
        let area = *buf.area();
        let mut checked = 0;
        for y in 0..area.height {
            for x in 0..area.width.saturating_sub(1) {
                let s = buf[(x, y)].symbol();
                if s == "│" || s == "┐" {
                    let next = buf[(x + 1, y)].symbol();
                    // The cell immediately past a card border must be either a
                    // gap (space) or the next card's left border — never text.
                    assert!(
                        next == " " || next == "│" || next == "┌" || next.is_empty(),
                        "text bled past card border at ({x},{y}): border `{s}` followed by `{next}`\n{}",
                        buf_to_string(buf)
                    );
                    checked += 1;
                }
            }
        }
        checked
    }

    #[test]
    fn narrow_width_card_name_and_sub_clamp_with_ellipsis() {
        // At a narrow total width the 3-up card grid squeezes each card to
        // ~17 cols. Long names ("Heartbeat-only") and subs ("Wake events
        // only, no scheduled tasks") MUST clamp with `…` inside the border —
        // never overflow into the card chrome.
        let screen = OrchestrationScreen::new();
        let buf = render_at_width(&screen, 56);
        let dump = buf_to_string(&buf);
        // Ellipsis present → something legitimately clamped.
        assert!(
            dump.contains('…'),
            "expected at least one ellipsis-clamped card field at W56; got:\n{dump}"
        );
        // No card content bleeds past its right border into the gap.
        let checked = assert_no_bleed_past_card_border(&buf);
        assert!(checked > 0, "harness did not find any card borders to check");
    }

    #[test]
    fn normal_width_renders_name_and_sub_in_full() {
        // At a comfortable width the full name + sub must render — no
        // premature clamp.
        let screen = OrchestrationScreen::new();
        let dump = buf_to_string(&render_at_width(&screen, 100));
        assert!(
            dump.contains("Heartbeat-only"),
            "expected full mode name at W100; got:\n{dump}"
        );
        assert!(
            dump.contains("Heartbeat + cron + watchdog"),
            "expected full sub at W100; got:\n{dump}"
        );
    }

    #[test]
    fn narrow_width_badge_never_clobbers_glyph() {
        // The right-anchored ▸ SELECTED badge shares the glyph row. At narrow
        // widths it must drop rather than overwrite the glyph badge — the
        // `ALL` glyph must stay intact (regression: `ALL LECTED`).
        let screen = OrchestrationScreen::new(); // all-on selected by default
        let buf = render_at_width(&screen, 56);
        let dump = buf_to_string(&buf);
        assert!(
            dump.contains("ALL"),
            "the ALL glyph badge must survive at narrow width; got:\n{dump}"
        );
        assert!(
            !dump.contains("LECTED"),
            "the SELECTED badge must not clobber the glyph (`ALL LECTED`); got:\n{dump}"
        );
    }

    #[test]
    fn normal_width_shows_selected_badge() {
        // At a comfortable width the SELECTED badge renders in full alongside
        // the glyph (no premature drop).
        let screen = OrchestrationScreen::new();
        let dump = buf_to_string(&render_at_width(&screen, 100));
        assert!(
            dump.contains("▸ SELECTED"),
            "expected the full SELECTED badge at W100; got:\n{dump}"
        );
    }

    #[test]
    fn caret_painted_on_blink_phase_when_field_has_input() {
        // all-on is default (selected=0) → config fields visible.
        let mut screen = OrchestrationScreen::new();
        screen.input_char('3');
        screen.input_char('0');
        let rendered = render_to_string(&screen, true);
        assert!(
            rendered.contains('\u{258f}'),
            "expected canonical caret `▏` when cursor_on && focused field has raw input; got:\n{rendered}"
        );
    }

    #[test]
    fn caret_hidden_on_blink_off() {
        let mut screen = OrchestrationScreen::new();
        screen.input_char('3');
        screen.input_char('0');
        let rendered = render_to_string(&screen, false);
        assert!(
            !rendered.contains('\u{258f}'),
            "expected NO caret on the blink-off half-cycle (cursor_on=false); got:\n{rendered}"
        );
    }

    #[test]
    fn no_caret_on_empty_field_even_when_blinking() {
        // Fresh screen: focused field is empty (only placeholder shows). The
        // caret must NOT paint at the end of placeholder hint text.
        let screen = OrchestrationScreen::new();
        let rendered = render_to_string(&screen, true);
        assert!(
            !rendered.contains('\u{258f}'),
            "expected NO caret on an unfilled field (placeholder is a hint, not an edit position); got:\n{rendered}"
        );
    }

    #[test]
    fn static_block_caret_is_absent() {
        // Regression: the screen must never paint the old static `▌` block caret.
        let mut screen = OrchestrationScreen::new();
        screen.input_char('3');
        let rendered = render_to_string(&screen, true);
        assert!(
            !rendered.contains('\u{258c}'),
            "the static `▌` block caret must be gone (Option A unifies on blink-gated `▏`); got:\n{rendered}"
        );
    }
}
