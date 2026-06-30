use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Block, Borders, Widget};

use crate::theme;

// Theme aliases — match the actual const names in theme.rs
const FG: ratatui::style::Color = theme::TEXT;
const BG2: ratatui::style::Color = theme::BG_PANEL;

// Fallback candidates now derive from the canonical shared provider registry
// (`crate::screens::providers`) — no separate list to drift. A `FallbackProvider`
// is just a `ProviderInfo` (fallback reads the id/name/glyph/color/flagship subset).
use crate::screens::providers::{self, ProviderInfo as FallbackProvider};

/// Truncate `text` to at most `max_w` display chars, appending `…` when clipped.
/// Mirrors the proven #271 idiom (chanconfig/channels) so long names/flagships
/// truncate honestly *inside* their budget instead of hard-clipping mid-word.
fn clamp_ellipsis(text: &str, max_w: usize) -> String {
    let len = text.chars().count();
    if len <= max_w {
        return text.to_string();
    }
    if max_w == 0 {
        return String::new();
    }
    if max_w == 1 {
        return "…".to_string();
    }
    let mut out: String = text.chars().take(max_w - 1).collect();
    out.push('…');
    out
}


/// Backup LLM chain screen — step 6 (id: "fallback", code: "FLBK").
///
/// Matches JSX FallbackStep (line 870):
/// - Left: "Backup LLM chain" header + AVAILABLE list (checkbox cards)
/// - Right: YOUR FALLBACK CHAIN (ordered list with ↑↓✕ controls)
/// - Bottom-right: SUGGESTED hint box
pub struct FallbackScreen {
    /// Primary provider id (excluded from candidates).
    pub primary: String,
    /// Selected fallback chain — ordered list of provider ids.
    pub chain: Vec<String>,
    /// Cursor position in the AVAILABLE list (index into filtered candidates).
    pub cursor: usize,
}

impl FallbackScreen {
    pub fn new(primary: String) -> Self {
        Self {
            primary,
            chain: Vec::new(),
            cursor: 0,
        }
    }

    /// Re-point the screen at a newly-picked primary provider. Called when the
    /// Fallback step is (re)entered so the AVAILABLE list excludes the right
    /// primary instead of staying frozen at the construction-time provider
    /// ("anthropic"). If the new primary was already in the chain (user picked
    /// it as a fallback, then went back and promoted it to primary), drop it
    /// from the chain so a provider can't be its own fallback. Clamps the
    /// cursor into the (possibly shorter) filtered candidate list.
    pub fn set_primary(&mut self, primary: &str) {
        let lc = primary.to_lowercase();
        if self.primary != lc {
            self.primary = lc;
            // A provider can't be its own fallback — drop it from the chain.
            let p = self.primary.clone();
            self.chain.retain(|x| x != &p);
        }
        // Clamp the cursor in case the new primary shortened the list (or the
        // chain edit above changed the effective candidate count).
        let len = self.candidates().len();
        if len == 0 {
            self.cursor = 0;
        } else if self.cursor >= len {
            self.cursor = len - 1;
        }
    }

    /// Candidates filtered to exclude the primary provider.
    fn candidates(&self) -> Vec<&'static FallbackProvider> {
        providers::PROVIDERS
            .iter()
            .filter(|p| p.id != self.primary)
            .collect()
    }

    pub fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn move_down(&mut self) {
        let len = self.candidates().len();
        if self.cursor < len.saturating_sub(1) {
            self.cursor += 1;
        }
    }

    /// Toggle the currently highlighted provider in/out of the chain.
    /// Max 3 fallbacks (matches JSX: "Pick 0-3 backups").
    pub fn toggle(&mut self) {
        let candidates = self.candidates();
        if let Some(provider) = candidates.get(self.cursor) {
            let id = provider.id.to_string();
            if let Some(pos) = self.chain.iter().position(|x| x == &id) {
                self.chain.remove(pos);
            } else if self.chain.len() < 3 {
                self.chain.push(id);
            }
        }
    }

    /// Move a chain item up (↑ control in the right panel).
    pub fn chain_move_up(&mut self) {
        // For simplicity, move the last-added item up
        if self.chain.len() > 1 {
            let len = self.chain.len();
            self.chain.swap(len - 1, len - 2);
        }
    }

    /// Move a chain item down (↓ control in the right panel).
    pub fn chain_move_down(&mut self) {
        if self.chain.len() > 1 {
            let len = self.chain.len();
            self.chain.swap(len - 1, len - 2);
        }
    }

    /// Remove a provider from the chain by id.
    pub fn chain_remove(&mut self, id: &str) {
        self.chain.retain(|x| x != id);
    }

    /// Render the fallback screen into the given area.
    /// Matches JSX FallbackStep layout:
    /// - Left column: header + AVAILABLE list (checkbox cards)
    /// - Right column: YOUR FALLBACK CHAIN (ordered) + SUGGESTED box
    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(40),    // left: available list
                Constraint::Length(36), // right: chain + suggested
            ])
            .split(area);

        self.render_available(chunks[0], buf);
        self.render_chain_panel(chunks[1], buf);
    }

    fn render_available(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let inner = Block::default()
            .borders(Borders::NONE)
            .padding(ratatui::widgets::Padding::new(2, 2, 1, 0));

        let inner_area = inner.inner(area);
        inner.render(area, buf);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // header
                Constraint::Length(1), // AVAILABLE label
                Constraint::Min(0),   // provider list
            ])
            .split(inner_area);

        // Header: "Backup LLM chain" + sub
        let header = Line::from(vec![
            Span::styled("Backup LLM chain", Style::default().fg(FG).add_modifier(Modifier::BOLD)),
        ]);
        buf.set_line(rows[0].x, rows[0].y, &header, rows[0].width);

        let sub = Line::from(vec![
            Span::styled(
                "If your primary provider fails, the agent loop tries each fallback in order. Pick 0-3 backups.",
                Style::default().fg(theme::DIM),
            ),
        ]);
        buf.set_line(rows[0].x, rows[0].y + 1, &sub, rows[0].width);

        // AVAILABLE label
        let label = Line::from(vec![
            Span::styled("AVAILABLE", Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD)),
        ]);
        buf.set_line(rows[1].x, rows[1].y, &label, rows[1].width);

        // Provider cards
        let candidates = self.candidates();
        let card_height: u16 = 1;
        let gap: u16 = 1;

        for (i, provider) in candidates.iter().enumerate() {
            let in_chain = self.chain.contains(&provider.id.to_string());
            let y = rows[2].y + (i as u16) * (card_height + gap);
            if y + card_height > rows[2].y + rows[2].height {
                break;
            }

            let card_area = Rect::new(rows[2].x, y, rows[2].width, card_height);

            // Background
            let bg_color = if in_chain { theme::ACCENT_FAINT } else { BG2 };
            let bg_block = Block::default().style(Style::default().bg(bg_color));
            bg_block.render(card_area, buf);

            // Border: 1px muted, left 2px accent for all (matches JSX)
            let border_style = Style::default().fg(theme::MUTED);
            let border_block = Block::default()
                .borders(Borders::ALL)
                .border_style(border_style);
            border_block.render(card_area, buf);

            // Left accent stripe (2px)
            let accent_style = Style::default().fg(provider.color);
            buf.set_string(card_area.x, card_area.y, "▐", accent_style);
            buf.set_string(card_area.x + 1, card_area.y, "▐", accent_style);

            // Checkbox
            let check = if in_chain {
                Span::styled(" ✓ ", Style::default().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("   ", Style::default().fg(theme::MUTED))
            };

            // Glyph
            let glyph = Span::styled(
                format!("{:>3}", provider.glyph),
                Style::default().fg(provider.color).add_modifier(Modifier::BOLD),
            );

            // Name
            let name = Span::styled(
                format!(" {} ", provider.name),
                Style::default().fg(FG),
            );

            // Flagship — clamp to the remaining budget after the fixed-width
            // prefix so a long flagship truncates honestly with `…` *inside*
            // the card border instead of hard-clipping mid-word at the buffer
            // edge (the set_line hard-clip seam).
            //
            // Fixed left zone widths (chars): "  "(2) + check(3) + " "(1)
            //   + glyph(3) + " "(1) + name(" {name} " = name_chars + 2)
            //   + "  "(2) = 12 + name_chars.
            let total_budget = (card_area.width.saturating_sub(4)) as usize;
            let name_chars = provider.name.chars().count() + 2;
            let prefix = 12 + name_chars;
            let flagship_budget = total_budget.saturating_sub(prefix);
            let flagship = Span::styled(
                clamp_ellipsis(provider.flagship, flagship_budget),
                Style::default().fg(theme::DIM),
            );

            let line = Line::from(vec![
                Span::raw("  "),
                check,
                Span::raw(" "),
                glyph,
                Span::raw(" "),
                name,
                Span::raw("  "),
                flagship,
            ]);
            buf.set_line(card_area.x + 2, card_area.y, &line, card_area.width - 4);
        }
    }

    fn render_chain_panel(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let inner = Block::default()
            .borders(Borders::NONE)
            .padding(ratatui::widgets::Padding::new(2, 2, 1, 0));

        let inner_area = inner.inner(area);
        inner.render(area, buf);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // header
                Constraint::Min(0),   // chain items
                Constraint::Length(8), // suggested box (wrapped copy needs height)
            ])
            .split(inner_area);

        // Header: "FALLBACK CHAIN (N)" + reorder hint (matches JSX).
        let header = Line::from(vec![
            Span::styled(
                format!("FALLBACK CHAIN ({})", self.chain.len()),
                Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(rows[0].x, rows[0].y, &header, rows[0].width);

        // Reorder hint — brackets accent, matches JSX "Reorder with [ / ]".
        let hint = Line::from(vec![
            Span::styled("Reorder with ", Style::default().fg(theme::DIM)),
            Span::styled("[", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(" / ", Style::default().fg(theme::DIM)),
            Span::styled("]", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
        ]);
        buf.set_line(rows[0].x, rows[0].y + 1, &hint, rows[0].width);

        // Chain items
        if self.chain.is_empty() {
            let empty_lines = [
                "No fallbacks selected.",
                "Primary failures will fail the agent loop.",
            ];
            for (i, copy) in empty_lines.iter().enumerate() {
                let y = rows[1].y + i as u16;
                if y >= rows[1].y + rows[1].height {
                    break;
                }
                let line = Line::from(vec![Span::styled(
                    *copy,
                    Style::default().fg(theme::DIM),
                )]);
                buf.set_line(rows[1].x, y, &line, rows[1].width);
            }
        } else {
            for (i, id) in self.chain.iter().enumerate() {
                let provider = providers::by_id(id);
                let y = rows[1].y + i as u16;
                if y >= rows[1].y + rows[1].height {
                    break;
                }

                let (name, glyph, color) = if let Some(p) = provider {
                    (p.name, p.glyph, p.color)
                } else {
                    (id.as_str(), "???", theme::DIM)
                };

                // Order number — numbered badge (accent bg + bg-color text),
                // matches JSX numbered-badge shape (1/2/3 chips).
                let num = Span::styled(
                    format!(" {} ", i + 1),
                    Style::default()
                        .fg(theme::BG)
                        .bg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                );

                // Glyph
                let glyph_span = Span::styled(
                    format!("{:>3}", glyph),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                );

                // Name — clamp so the right-side ↑↓✕ controls always survive.
                // set_line clips the *right* of the line, so an over-long name
                // would push the controls off the panel (losing the ✕ remove
                // affordance entirely — worse than truncation). Reserve the
                // control zone (6 chars) + fixed prefix, clamp the name to the
                // remainder so it truncates with `…` and the controls stay put.
                //
                // Fixed widths (chars): " "(1) + num(" {n} " = 3) + " "(1)
                //   + glyph({:>3} = 3) + " "(1) = 9; name renders as " {name} "
                //   (name_chars + 2); controls " ↑"+" ↓"+" ✕" = 6.
                const CONTROL_W: usize = 6;
                let row_budget = rows[1].width as usize;
                let name_fixed = 9 + 2; // prefix + the two padding spaces in " {name} "
                let name_budget = row_budget
                    .saturating_sub(CONTROL_W)
                    .saturating_sub(name_fixed);
                let name_clamped = clamp_ellipsis(name, name_budget);
                let name_span = Span::styled(
                    format!(" {} ", name_clamped),
                    Style::default().fg(FG),
                );

                // Controls: ↑ ↓ ✕
                let up = Span::styled(" ↑", Style::default().fg(theme::DIM));
                let down = Span::styled(" ↓", Style::default().fg(theme::DIM));
                let remove = Span::styled(" ✕", Style::default().fg(theme::DIM));

                let line = Line::from(vec![
                    Span::raw(" "),
                    num,
                    Span::raw(" "),
                    glyph_span,
                    Span::raw(" "),
                    name_span,
                    up,
                    down,
                    remove,
                ]);
                buf.set_line(rows[1].x, y, &line, rows[1].width);
            }
        }

        // Suggested box (only shown when chain has items, matches JSX)
        if !self.chain.is_empty() {
            let box_area = rows[2];
            let suggested_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::MUTED))
                .style(Style::default().bg(BG2));
            let inner = suggested_block.inner(box_area);
            suggested_block.render(box_area, buf);
            let label = Line::from(vec![
                Span::styled("SUGGESTED", Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD)),
            ]);
            buf.set_line(inner.x + 1, inner.y, &label, inner.width - 2);

            // Wrap the suggestion copy across the narrow box (JSX wraps it in
            // a flex box) so the full "OpenAI + Ollama" text is visible rather
            // than truncated by a single set_line.
            let hint = ratatui::text::Text::from(vec![Line::from(vec![
                Span::raw("Based on "),
                Span::styled(
                    format!("{} primary", self.primary),
                    Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw(", consider adding "),
                Span::styled(
                    "OpenAI + Ollama",
                    Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" for cheap-fast fallback."),
            ])]);
            let hint_area = Rect {
                x: inner.x + 1,
                y: inner.y + 1,
                width: inner.width.saturating_sub(2),
                height: inner.height.saturating_sub(1),
            };
            ratatui::widgets::Paragraph::new(hint)
                .wrap(ratatui::widgets::Wrap { trim: true })
                .render(hint_area, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    fn row_text(buf: &Buffer, y: u16, x0: u16, x1: u16) -> String {
        let mut s = String::new();
        for x in x0..x1.min(buf.area.width) {
            s.push_str(buf[(x, y)].symbol());
        }
        s
    }

    fn full_dump(buf: &Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area.height {
            out.push_str(&row_text(buf, y, 0, buf.area.width));
            out.push('\n');
        }
        out
    }

    /// clamp_ellipsis budget semantics (the shared #271 idiom).
    #[test]
    fn clamp_ellipsis_budget_semantics() {
        assert_eq!(clamp_ellipsis("short", 10), "short");
        assert_eq!(clamp_ellipsis("exactfit12", 10), "exactfit12");
        assert_eq!(clamp_ellipsis("toolongname", 6), "toolo…");
        assert_eq!(clamp_ellipsis("anything", 1), "…");
        assert_eq!(clamp_ellipsis("anything", 0), "");
    }

    /// LOAD-BEARING: at a narrow width the AVAILABLE-card flagship must clamp
    /// with `…` *inside* the card border — never hard-clip mid-word past the
    /// right edge. Reverting the flagship clamp makes this FAIL.
    #[test]
    fn narrow_width_available_flagship_clamps_inside_card() {
        let screen = FallbackScreen::new("anthropic".to_string());
        // Narrow: left list = Min(40) gets ~ width-36. Force a tight buffer.
        let w = 64u16;
        let mut buf = Buffer::empty(Rect::new(0, 0, w, 30));
        screen.render(Rect::new(0, 0, w, 30), &mut buf);

        let dump = full_dump(&buf);
        // The clamp must be ACTIVE: a narrow card forces at least one flagship
        // to truncate, so the ellipsis must appear in the rendered buffer.
        // Reverting the flagship clamp removes the `…` (hard-clip mid-word
        // instead) — this fails.
        assert!(
            dump.contains('…'),
            "no ellipsis present — flagship clamp is not active: {dump}"
        );
        // OVERFLOW GUARD: for every card row (identified by the right border
        // glyph `┐`/`│`), assert no text glyph sits in the cell immediately
        // past the border — i.e. the flagship is clamped INSIDE the card, not
        // bleeding past its right edge. Walk each row, find the card's right
        // border, and check the next cell is blank.
        for y in 0..30u16 {
            // Find the rightmost card-border glyph on this row.
            let mut border_x: Option<u16> = None;
            for x in 0..w {
                let g = buf[(x, y)].symbol();
                if g == "┐" || g == "┘" || g == "│" {
                    border_x = Some(x);
                }
            }
            if let Some(bx) = border_x {
                if bx + 1 < w {
                    let past = buf[(bx + 1, y)].symbol().to_string();
                    assert!(
                        past == " " || past.is_empty(),
                        "row {y}: glyph {past:?} bled past the card border at x={}: {dump}",
                        bx + 1
                    );
                }
            }
        }
    }

    /// LOAD-BEARING: in the CHAIN panel the ↑↓✕ controls must always render —
    /// an over-long name must clamp with `…` rather than push the controls off
    /// the panel. Reverting the name clamp makes this FAIL (controls vanish).
    #[test]
    fn narrow_width_chain_controls_always_render() {
        let mut screen = FallbackScreen::new("anthropic".to_string());
        // Populate the chain with real provider ids so rows render.
        screen.chain = vec!["openai".to_string(), "google".to_string()];

        let w = 64u16;
        let mut buf = Buffer::empty(Rect::new(0, 0, w, 30));
        screen.render(Rect::new(0, 0, w, 30), &mut buf);

        let dump = full_dump(&buf);
        // The ✕ remove control must appear at least once in the rendered buffer
        // (one per chain row). If the name pushed it off, it's gone.
        assert!(
            dump.contains('✕'),
            "chain ✕ remove control missing — name clamp regressed: {dump}"
        );
        assert!(
            dump.contains('↑') && dump.contains('↓'),
            "chain ↑↓ controls missing — name clamp regressed: {dump}"
        );
    }

    /// At a normal width the screen renders without panic and the controls are
    /// present (no premature clamping breaks the layout).
    #[test]
    fn normal_width_renders_chain_with_controls() {
        let mut screen = FallbackScreen::new("anthropic".to_string());
        screen.chain = vec!["openai".to_string()];
        let w = 120u16;
        let mut buf = Buffer::empty(Rect::new(0, 0, w, 30));
        screen.render(Rect::new(0, 0, w, 30), &mut buf);
        let dump = full_dump(&buf);
        assert!(dump.contains('✕'), "normal-width chain ✕ missing: {dump}");
    }
}
