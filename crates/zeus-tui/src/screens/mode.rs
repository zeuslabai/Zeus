use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Block, Borders, Widget};

use crate::theme;

/// Truncate `text` to `max_w` columns, appending `…` when clipped. Char-safe
/// (counts Unicode scalars, not bytes) — mirrors the canonical #271 helper.
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

/// Setup mode option — matches JSX ModeStep modes array (line ~510).
struct ModeOption {
    #[allow(dead_code)]
    id: &'static str,
    name: &'static str,
    glyph: &'static str,
    color: ratatui::style::Color,
    sub: &'static str,
    time: &'static str,
    steps: &'static str,
}

const MODES: &[ModeOption] = &[
    ModeOption {
        id: "quickstart",
        name: "QuickStart",
        glyph: "QS",
        color: theme::GREEN,
        sub: "1 LLM, 1 channel, sane defaults",
        time: "~3 min",
        steps: "1 step left",
    },
    ModeOption {
        id: "full",
        name: "Full Setup",
        glyph: "FU",
        color: theme::FIRE_ORANGE,
        sub: "Walk every section in detail",
        time: "~25 min",
        steps: "17 steps left",
    },
    ModeOption {
        id: "custom",
        name: "Custom",
        glyph: "CU",
        color: theme::CYAN,
        sub: "Pick which steps you want",
        time: "varies",
        steps: "you choose",
    },
];

/// Setup Mode screen — JSX `ModeStep` (509–576).
/// Three cards laid out in a horizontal 3-column GRID: QuickStart / Full Setup
/// / Custom. ←/→ move `selected` between the cards (wired in `app.rs`, NOT
/// step-nav). The selected card gets an accent border + faint accent fill, a
/// filled glyph badge, a `▸ SELECTED` badge top-right, and a white name.
pub struct ModeScreen {
    /// Currently selected mode index (0=quickstart, 1=full, 2=custom).
    pub selected: usize,
}

impl Widget for ModeScreen {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        let inner = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // title + subtitle
                Constraint::Length(11), // cards (tall — JSX minHeight 200)
                Constraint::Length(1),  // gap
                Constraint::Length(3),  // NOTE box
                Constraint::Min(0),     // filler
            ])
            .split(area);

        // ---- Header: "Choose your setup mode" + sub -----------------------
        buf.set_string(
            inner[0].x + 2,
            inner[0].y,
            "Choose your setup mode",
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        );
        buf.set_string(
            inner[0].x + 2,
            inner[0].y + 1,
            "Select the path that matches how much you want to configure now.",
            Style::default().fg(theme::DIM),
        );

        // ---- 3-column GRID of mode cards ---------------------------------
        let card_area = inner[1];
        let card_width = card_area.width.saturating_sub(6) / 3; // 2-col gap ×2
        let card_constraints = [
            Constraint::Length(card_width),
            Constraint::Length(2), // gap
            Constraint::Length(card_width),
            Constraint::Length(2), // gap
            Constraint::Length(card_width),
        ];
        let cards = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(card_constraints)
            .split(card_area);

        for (i, mode) in MODES.iter().enumerate() {
            let is_selected = i == self.selected;
            let card_rect = cards[i * 2]; // skip the gap slots

            // Border: accent when selected, muted otherwise. (Left-border =
            // mode color is approximated by the glyph-badge color in TUI.)
            let border_color = if is_selected {
                theme::ACCENT
            } else {
                theme::MUTED
            };
            let bg = if is_selected {
                theme::ACCENT_FAINT
            } else {
                theme::BG_PANEL
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(bg));
            block.render(card_rect, buf);

            // Left accent stripe — JSX `borderLeft: 2px solid m.color`. Each
            // card carries its mode-color identity on the left edge, drawn over
            // the block's own left border so it reads as a colored rail.
            if card_rect.height > 2 {
                for sy in (card_rect.y + 1)..(card_rect.y + card_rect.height - 1) {
                    buf.set_string(
                        card_rect.x,
                        sy,
                        "\u{2502}",
                        Style::default().fg(mode.color).bg(bg),
                    );
                }
            }

            if card_rect.width < 4 {
                continue; // too narrow to draw contents safely
            }
            let cx = card_rect.x + 2;
            let inner_right = card_rect.x + card_rect.width.saturating_sub(2);
            let mut cy = card_rect.y + 1;

            // ▸ SELECTED badge top-right when selected.
            if is_selected {
                let badge = "\u{25b8} SELECTED";
                let bw = badge.chars().count() as u16;
                if inner_right > card_rect.x + bw {
                    buf.set_string(
                        inner_right.saturating_sub(bw),
                        card_rect.y,
                        badge,
                        Style::default()
                            .fg(theme::ACCENT)
                            .add_modifier(Modifier::BOLD),
                    );
                }
            }

            // Glyph badge — JSX 48×48 box with `1px solid m.color`. In the TUI
            // we draw a bordered badge box (mode-color border): filled fill
            // (bg=mode color, fg=bg) when selected, else outlined glyph on bg.
            // Badge box is `glyph_w` wide ×3 tall.
            let glyph_inner = format!(" {} ", mode.glyph);
            let glyph_w = glyph_inner.chars().count() as u16 + 2; // + 2 borders
            let badge_rect = Rect {
                x: cx,
                y: cy,
                width: glyph_w.min(card_rect.width.saturating_sub(3)),
                height: 3,
            };
            if badge_rect.width >= 3 && badge_rect.y + 3 <= card_rect.y + card_rect.height {
                let badge_fill = if is_selected { mode.color } else { bg };
                let badge_block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(mode.color))
                    .style(Style::default().bg(badge_fill));
                badge_block.render(badge_rect, buf);
                let glyph_style = if is_selected {
                    Style::default()
                        .fg(theme::BG)
                        .bg(mode.color)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(mode.color)
                        .add_modifier(Modifier::BOLD)
                };
                buf.set_string(
                    badge_rect.x + 1,
                    badge_rect.y + 1,
                    &glyph_inner,
                    glyph_style,
                );
            }
            cy += 4;

            // Name — white when selected, fg otherwise.
            let name_color = if is_selected {
                theme::WHITE
            } else {
                theme::TEXT
            };
            // Card interior budget: cx (left pad) → inner_right (right pad).
            // Clamp both name and sub so a long string truncates with `…`
            // INSIDE the card border instead of bleeding past it / into the
            // next card. (TIME/STEPS rows are already right-clamped by set_row.)
            let content_w = inner_right.saturating_sub(cx) as usize;
            buf.set_string(
                cx,
                cy,
                clamp_ellipsis(mode.name, content_w),
                Style::default().fg(name_color).add_modifier(Modifier::BOLD),
            );
            cy += 1;

            // Sub description (dim).
            buf.set_string(
                cx,
                cy,
                clamp_ellipsis(mode.sub, content_w),
                Style::default().fg(theme::DIM),
            );
            cy += 2;

            // TIME row — label left (muted, bold), value right (mode color).
            set_row(buf, cx, inner_right, cy, "TIME", mode.time, mode.color);
            cy += 1;
            // STEPS row — label left (muted, bold), value right (dim).
            set_row(buf, cx, inner_right, cy, "STEPS", mode.steps, theme::DIM);
        }

        // ---- NOTE box ----------------------------------------------------
        let note_area = inner[3];
        let note_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::MUTED))
            .style(Style::default().bg(theme::BG_PANEL));
        note_block.render(note_area, buf);
        if note_area.width > 6 {
            let nx = note_area.x + 2;
            let ny = note_area.y + 1;
            buf.set_string(
                nx,
                ny,
                "NOTE",
                Style::default()
                    .fg(theme::ACCENT_DIM)
                    .add_modifier(Modifier::BOLD),
            );
            buf.set_string(
                nx + 6,
                ny,
                "Skipped sections can be configured later via zeus onboard --resume or by editing ~/.zeus/config.toml.",
                Style::default().fg(theme::DIM),
            );
        }
    }
}

/// Draw a TIME/STEPS row: bold-muted label at `cx`, value right-aligned to
/// `right_edge`. Value color is caller-supplied. Char-safe width math.
fn set_row(
    buf: &mut ratatui::buffer::Buffer,
    cx: u16,
    right_edge: u16,
    cy: u16,
    label: &str,
    value: &str,
    value_color: ratatui::style::Color,
) {
    buf.set_string(
        cx,
        cy,
        label,
        Style::default()
            .fg(theme::MUTED)
            .add_modifier(Modifier::BOLD),
    );
    let vw = value.chars().count() as u16;
    if right_edge > cx + vw {
        buf.set_string(
            right_edge.saturating_sub(vw),
            cy,
            value,
            Style::default().fg(value_color),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    /// Render the screen at an explicit width (height fixed tall enough for
    /// the card grid), returning the per-row buffer for column-precise
    /// overflow assertions. Mirrors the orchestration.rs #271 harness.
    fn render_at_width(selected: usize, w: u16) -> Buffer {
        let area = Rect::new(0, 0, w, 24);
        let mut buf = Buffer::empty(area);
        ModeScreen { selected }.render(area, &mut buf);
        buf
    }

    fn buf_to_string(buf: &Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// Assert no card-interior text bleeds past a card's right border: scan
    /// every row, and for each card right-border glyph the cell immediately
    /// after must be a gap, the next card's left border, or empty — never text.
    fn assert_no_bleed_past_border(buf: &Buffer) -> usize {
        let mut checked = 0;
        for y in 0..buf.area.height {
            for x in 0..buf.area.width.saturating_sub(1) {
                let s = buf[(x, y)].symbol();
                if s == "│" || s == "┐" || s == "┘" {
                    let next = buf[(x + 1, y)].symbol();
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
        // At width 56 the 3-up grid squeezes each card to ~17 cols. Long subs
        // ("1 LLM, 1 channel, sane defaults" / "Walk every section in detail")
        // must truncate with `…` INSIDE the card border — not bleed past it.
        let buf = render_at_width(0, 56);
        let dump = buf_to_string(&buf);
        // The ellipsis must appear (something clipped at this width).
        assert!(
            dump.contains('…'),
            "expected ellipsis truncation at width 56:\n{dump}"
        );
        // And nothing may bleed past a card border.
        let checked = assert_no_bleed_past_border(&buf);
        assert!(checked > 0, "expected to scan at least one card border");
        // The full un-clipped sub strings must NOT appear verbatim at narrow.
        assert!(
            !dump.contains("1 LLM, 1 channel, sane defaults"),
            "full sub should be clipped at width 56:\n{dump}"
        );
    }

    #[test]
    fn normal_width_renders_name_and_sub_in_full() {
        // At width 120 the cards are wide enough that the full name + sub
        // render without any premature clamp.
        let buf = render_at_width(0, 120);
        let dump = buf_to_string(&buf);
        assert!(
            dump.contains("QuickStart"),
            "full name must render at width 120:\n{dump}"
        );
        assert!(
            dump.contains("1 LLM, 1 channel, sane defaults"),
            "full sub must render at width 120:\n{dump}"
        );
        assert!(
            dump.contains("Walk every section in detail"),
            "full second-card sub must render at width 120:\n{dump}"
        );
        // No bleed even at wide.
        assert_no_bleed_past_border(&buf);
    }
}
