use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme;

/// StatusBar widget — global onboarding footer (renders on every step).
/// Bottom bar with key hints: `● onboard │ ↑↓ Navigate │ ↵ Select │ Tab Field │ Esc Back │ ^N Continue`
/// All advertised keys are verified against the real dispatch in
/// `app.rs::handle_key`/`handle_key_mods` — no phantom hints (the old
/// `? Help` / `← Back` had no key arms). Width-safe: hints drop lowest-first
/// when the row is too narrow, so the line can never clip mid-word.
/// Which footer button is Tab-focused (highlighted). Mirrors `app::FooterFocus`
/// but kept as a plain triple-state here to avoid a widget→app dep cycle.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FooterHighlight {
    None,
    Back,
    Next,
}

pub struct StatusBar {
    pub can_back: bool,
    pub can_continue: bool,
    /// Which footer button (if any) is Tab-focused → rendered highlighted.
    pub footer_highlight: FooterHighlight,
}

impl Widget for StatusBar {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.height == 0 {
            return;
        }

        // Fill background
        for x in area.left()..area.right() {
            buf[(x, area.y)]
                .set_symbol(" ")
                .set_style(Style::default().bg(theme::BG_PANEL));
        }

        let mut spans = Vec::new();

        // Corner BACK/NEXT buttons are drawn LAST, overwriting the left/right
        // edges of this hint line. At narrow widths they can clip the prefix
        // (e.g. "onboard" → "rd" surviving between the buttons — the reported
        // garble). So the whole center hint run is gated on there being enough
        // room for the prefix to survive the corner overwrite. Below that, only
        // the corner buttons render — the only actionable controls anyway.
        // BACK is drawn at the LEFT edge and overwrites whatever the hint line
        // put under it — so the prefix only survives if the row is wide enough
        // that the prefix sits entirely to the RIGHT of the BACK button. We
        // therefore reserve the BACK width as dead space the center must clear,
        // not merely subtract it from the far end.
        let back_w: u16 = if self.can_back { 8 } else { 0 }; // " ← BACK "
        let next_w: u16 = if self.can_continue { 8 } else { 0 }; // " NEXT → "
        let prefix_w: u16 = 13; // " ● onboard │ "
        let continue_w: u16 = 12; // "│ ^N Continue"
        // Center renders only if the prefix clears BACK on the left AND the
        // whole center (prefix + minimal ^N) fits before NEXT on the right.
        let room_for_center = area.width >= back_w + prefix_w + continue_w + next_w;

        if room_for_center {
            // Pad the line start past the BACK button so the prefix is never
            // overwritten by it (the "rd" garble = "onboard" clipped by BACK).
            if back_w > 0 {
                spans.push(Span::raw(" ".repeat(back_w as usize)));
            }
            // Status indicator
            spans.push(Span::styled(" ● ", Style::default().fg(theme::GREEN)));
            spans.push(Span::styled("onboard ", Style::default().fg(theme::DIM)));
            spans.push(Span::styled("│ ", Style::default().fg(theme::MUTED)));
        }

        // Navigation hints — HONEST bindings only, verified against the real
        // onboarding key dispatch in `app.rs::handle_key` / `handle_key_mods`:
        //   ↑↓  → Up/Down arms (Navigate)            ✅ real
        //   ↵   → Enter arm (Select/advance/toggle)  ✅ real
        //   Tab → tab_advance: cycles screen fields  ✅ real
        //   Esc → current_step -= 1 (Back one step)  ✅ real
        //   ^N  → handle_key_mods Ctrl+N (Continue)  ✅ real (universal advance)
        // DROPPED: `? Help` — NO `Char('?')` arm exists anywhere in the
        // onboarding dispatch; it was a phantom hint (the "100% incorrect" key).
        // Priority-ordered: lowest-priority hints are dropped first when the
        // row is too narrow, so the line can NEVER clip mid-word (the "rd"
        // truncation garble). `set_line` would otherwise sever a multibyte
        // glyph at `area.width`.
        let hints: &[(&str, &str)] = &[
            ("↑↓", "Navigate"),
            ("↵", "Select"),
            ("Tab", "Field"),
            ("Esc", "Back"),
        ];

        if room_for_center {
            // Reserve space for the trailing `│ ^N Continue` segment and the
            // corner BACK/NEXT buttons, so the variable-length hint run only
            // ever consumes the space that's actually free. We grow the hint
            // run hint-by-hint and stop before it would overflow — no partial
            // hint, no mid-word clip.
            let budget = area
                .width
                .saturating_sub(back_w + prefix_w + continue_w + next_w);

            let mut used: u16 = 0;
            for (key, label) in hints {
                // width of "<key> <label> " (chars, not bytes — multibyte-safe)
                let seg_w = (key.chars().count() + 1 + label.chars().count() + 1) as u16;
                if used + seg_w > budget {
                    break; // dropping this (+ lower-priority) hints keeps us width-safe
                }
                used += seg_w;
                spans.push(Span::styled(
                    format!("{} ", key),
                    Style::default()
                        .fg(theme::ACCENT_DIM)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    format!("{} ", label),
                    Style::default().fg(theme::DIM),
                ));
            }

            // Center keeps the Ctrl+N hint (keyboard shortcut, always
            // discoverable). Only shown when there's room left after the hints
            // for the full segment — never clipped mid-word.
            if budget.saturating_sub(used) >= continue_w {
                spans.push(Span::styled("│ ", Style::default().fg(theme::MUTED)));
                spans.push(Span::styled(
                    "^N ",
                    Style::default()
                        .fg(theme::ACCENT_DIM)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled("Continue", Style::default().fg(theme::DIM)));
            }
        }

        // Draw the hint line first (fills the row), then overwrite the corners
        // with the positioned, focusable BACK/NEXT buttons.
        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);

        // Focusable footer buttons (merakizzz spec): BACK bottom-left,
        // NEXT bottom-right. Focus-highlight mirrors complete.rs focused
        // button (fg=BG on bg=ACCENT); unfocused uses ACCENT_DIM on the panel.
        let btn_style = |focused: bool| {
            if focused {
                Style::default()
                    .fg(theme::BG)
                    .bg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(theme::ACCENT_DIM)
                    .bg(theme::BG_PANEL)
                    .add_modifier(Modifier::BOLD)
            }
        };

        // BACK — bottom-left. Hidden on the first step (nothing to go back to).
        if self.can_back {
            let back = Line::from(Span::styled(
                " ← BACK ",
                btn_style(self.footer_highlight == FooterHighlight::Back),
            ));
            buf.set_line(area.x, area.y, &back, area.width);
        }

        // NEXT — bottom-right. Hidden past the final step.
        if self.can_continue {
            let label = " NEXT → ";
            let w = label.chars().count() as u16;
            let x = area.right().saturating_sub(w);
            let next = Line::from(Span::styled(
                label,
                btn_style(self.footer_highlight == FooterHighlight::Next),
            ));
            buf.set_line(x, area.y, &next, w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    /// Render the StatusBar at a given width and flatten the row to a String.
    fn render_row(width: u16, can_back: bool, can_continue: bool) -> String {
        let area = Rect::new(0, 0, width, 1);
        let mut buf = Buffer::empty(area);
        StatusBar {
            can_back,
            can_continue,
            footer_highlight: FooterHighlight::None,
        }
        .render(area, &mut buf);
        (0..width).map(|x| buf[(x, 0)].symbol()).collect()
    }

    /// The footer must advertise only keys that have a REAL arm in
    /// `app.rs::handle_key`/`handle_key_mods`. The phantom `? Help` (no
    /// `Char('?')` arm anywhere) must NOT appear — it was the "100% incorrect"
    /// key merakizzz flagged.
    #[test]
    fn footer_shows_real_keys_and_drops_phantom_help() {
        let row = render_row(120, false, false);
        // Real, verified bindings:
        assert!(row.contains("Navigate"), "missing ↑↓ Navigate: {row:?}");
        assert!(row.contains("Select"), "missing ↵ Select: {row:?}");
        assert!(row.contains("Field"), "missing Tab Field: {row:?}");
        assert!(row.contains("Back"), "missing Esc Back: {row:?}");
        assert!(row.contains("Continue"), "missing ^N Continue: {row:?}");
        assert!(row.contains("Esc"), "missing Esc key glyph: {row:?}");
        // Phantom hint must be gone:
        assert!(!row.contains("Help"), "phantom '? Help' still present: {row:?}");
        assert!(!row.contains('?'), "phantom '?' key still present: {row:?}");
    }

    /// Width-safety: at progressively narrower widths the row must NEVER clip a
    /// hint mid-word (the "rd" garble). We assert that every word fragment
    /// rendered is a COMPLETE advertised token — no dangling tail like "rd"
    /// from a severed "Navigate"/"Continue"/"forward". Lower-priority hints are
    /// dropped wholesale instead of truncated.
    #[test]
    fn footer_never_clips_mid_word_at_narrow_width() {
        // Every COMPLETE alphabetic token the footer can render: hint labels,
        // hint key-glyphs, and the corner button labels. A mid-word clip would
        // leave a tail matching none of these.
        let whole_words = [
            "onboard", "Navigate", "Select", "Field", "Back", "Continue",
            "Tab", "Esc", "N", "BACK", "NEXT",
        ];
        for width in (20u16..=120).step_by(1) {
            let row = render_row(width, true, true);
            // Collect alphabetic runs (candidate words) and verify each is a
            // prefix-free COMPLETE advertised token — i.e. it equals one of the
            // whole words. A mid-word clip would leave a tail ("igate", "rd",
            // "tinue") that matches none of them.
            for frag in row.split(|c: char| !c.is_alphabetic()) {
                if frag.is_empty() {
                    continue;
                }
                // Every alphabetic fragment must be one of the full tokens.
                // (BACK/NEXT corner buttons render uppercase — allow those too.)
                let known = whole_words.contains(&frag)
                    || frag == "BACK"
                    || frag == "NEXT";
                assert!(
                    known,
                    "width {width}: clipped/garbled fragment {frag:?} in row {row:?}",
                );
            }
        }
    }

    /// At ultra-narrow widths the whole hint run drops out cleanly (no panic,
    /// no partial glyph) — only the prefix and/or corner buttons may remain.
    #[test]
    fn footer_degrades_cleanly_when_too_narrow() {
        for width in 1u16..=12 {
            let row = render_row(width, true, true);
            // Must not contain a severed multi-char hint tail.
            assert!(!row.contains("igate"), "garbled at width {width}: {row:?}");
            assert!(!row.contains("tinue"), "garbled at width {width}: {row:?}");
        }
    }
}
