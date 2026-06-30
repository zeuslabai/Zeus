//! Skills — Installed skills + marketplace browser.
//!
//! Advanced subview (id: `skills`). Matches JSX `AdvancedSubview` `skills`
//! branch (docs/zeus-tui-production.jsx ~line 1458): summary line
//! (`<N> installed · <M> in marketplace` + BROWSE MARKETPLACE button) then one
//! row per skill — enabled glyph · name · CATEGORY · tool count · VIEW +
//! ENABLE/DISABLE buttons. Theme tokens, geometric glyphs, no emoji. No live
//! data → honest "awaiting" line; marketplace count has no endpoint → dash
//! (#284 de-mock).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::api::SkillResponse;
use crate::prod::draw::cell_mut_clamped;
use crate::theme;

/// One render row — built from live `/v1/skills` data.
struct Row<'a> {
    name: &'a str,
    category: &'a str,
    enabled: bool,
    tools: u8,
}

/// Render the `skills` subview body into `area`.
///
/// When `live` is `Some`, rows are built from the gateway's `/v1/skills`
/// response (name/enabled real; category/tools not in the API shape yet → "—"
/// / 0). When `None` (pre-fetch), shows an honest "awaiting" line — no
/// fabricated skills or counts (#284 de-mock).
pub fn render(area: Rect, buf: &mut Buffer, live: Option<&[SkillResponse]>) {
    Clear.render(area, buf);
    if area.width < 8 || area.height < 1 {
        return;
    }
    let right = area.right().min(buf.area.right());

    // Build the row list: live overlay when present, else empty.
    let rows: Vec<Row> = match live {
        Some(skills) => skills
            .iter()
            .map(|s| Row {
                name: &s.name,
                category: "—",
                enabled: s.enabled,
                tools: 0,
            })
            .collect(),
        // Pre-fetch → no fabricated roster (#284 de-mock).
        None => Vec::new(),
    };
    // Installed count: live length when fetched, else 0 (honest).
    let installed = live.map(|s| s.len() as u16).unwrap_or(0);

    // Summary line: "<N> installed · <M> in marketplace". Marketplace count
    // has no backing endpoint → honest dash, not a fabricated number.
    let mut x = area.x + 2;
    let summary = format!("{}", installed);
    x = set_str(x, area.y + 1, &summary, Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD), right, buf);
    x = set_str(x, area.y + 1, " installed   ", Style::default().fg(theme::TEXT), right, buf);
    x = set_str(x, area.y + 1, "—", Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD), right, buf);
    let _ = set_str(x, area.y + 1, " in marketplace", Style::default().fg(theme::TEXT), right, buf);
    // BROWSE MARKETPLACE button (right-aligned, accent).
    let btn = " BROWSE MARKETPLACE ";
    let bx = right.saturating_sub(btn.len() as u16 + 1);
    paint_button(bx, area.y + 1, btn, theme::ACCENT, theme::BG, buf);

    // Skill rows (3px in JSX between → 1 blank line here), 1 line each.
    let mut y = area.y + 3;
    if rows.is_empty() {
        // Honest empty state — no fabricated skills.
        set_str(area.x + 2, y, "No skills — fetching from /v1/skills…", Style::default().fg(theme::DIM), right, buf);
        return;
    }
    for s in rows.iter() {
        if y >= area.bottom() {
            break;
        }
        // Row bg panel.
        for cx in (area.x + 1)..right {
            if let Some(c) = cell_mut_clamped(buf, cx, y) { c.set_bg(theme::BG_PANEL); }
        }
        let mut x = area.x + 2;
        // Enabled glyph: ✓ green / ○ dim.
        let (glyph, gcol) = if s.enabled { ("\u{2713}", theme::GREEN) } else { ("\u{25cb}", theme::DIM) };
        x = set_str(x, y, glyph, Style::default().fg(gcol), right, buf) + 1;
        // Name (white bold, ~22 wide).
        let _ = set_str(x, y, s.name, Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD), right, buf);
        x = (area.x + 2 + 24).min(right);
        // Category, uppercased, dim.
        let _ = set_str(x, y, &s.category.to_uppercase(), Style::default().fg(theme::DIM), right, buf);
        // Tools + buttons, right-aligned.
        let toggle = if s.enabled { "DISABLE" } else { "ENABLE" };
        let tail = format!("{} tools  [VIEW]  [{}]", s.tools, toggle);
        let tx = right.saturating_sub(tail.len() as u16 + 1);
        let tcol = if s.enabled { theme::AMBER } else { theme::GREEN };
        // Tools count dim, then toggle colored.
        let split = format!("{} tools  [VIEW]  ", s.tools);
        let after = set_str(tx, y, &split, Style::default().fg(theme::DIM), right, buf);
        let _ = set_str(after, y, &format!("[{}]", toggle), Style::default().fg(tcol).add_modifier(Modifier::BOLD), right, buf);

        y += 2;
    }
}

/// Paint a solid filled button (bg=fill, fg=text).
fn paint_button(x: u16, y: u16, label: &str, fill: ratatui::style::Color, text: ratatui::style::Color, buf: &mut Buffer) {
    for (i, ch) in label.chars().enumerate() {
        let cx = x + i as u16;
        if cx >= buf.area.right() {
            break;
        }
        if let Some(c) = cell_mut_clamped(buf, cx, y) { c.set_char(ch).set_fg(text).set_bg(fill).set_style(Style::default().add_modifier(Modifier::BOLD).bg(fill).fg(text)); }
    }
}

/// Write `s` at (x,y), clipped to `max_x`. Returns x after the last cell.
fn set_str(x: u16, y: u16, s: &str, style: Style, max_x: u16, buf: &mut Buffer) -> u16 {
    let mut cx = x;
    for ch in s.chars() {
        if cx >= max_x || cx >= buf.area.right() {
            break;
        }
        if let Some(c) = cell_mut_clamped(buf, cx, y) { c.set_char(ch).set_style(style); }
        cx += 1;
    }
    cx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_yields_no_fabricated_rows() {
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        // No live data → honest empty state, no fabricated skills/counts.
        render(area, &mut buf, None);
        let dump: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("fetching from /v1/skills"));
        assert!(!dump.contains("git-flow"));
        assert!(!dump.contains("147"), "fabricated marketplace count must not render");
        render(Rect::new(0, 0, 3, 1), &mut buf, None);
    }
}
